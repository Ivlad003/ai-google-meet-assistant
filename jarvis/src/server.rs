use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{Request, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::bot_bridge::BridgeState;
use crate::config::Config;
use crate::llm::LlmAgent;
use crate::process;

#[derive(Embed)]
#[folder = "src/assets/"]
struct Assets;

pub struct AppState {
    pub config: RwLock<Config>,
    pub transcript_tx: broadcast::Sender<String>,
    pub bridge_state: Arc<BridgeState>,
    pub agent: Arc<LlmAgent>,
    pub bot_process: Arc<std::sync::Mutex<process::VexaBotProcess>>,
    pub response_mode_tx: tokio::sync::watch::Sender<crate::config::ResponseMode>,
    pub data_dir: std::path::PathBuf,
    pub openai_key: String,
    pub http_client: reqwest::Client,
    pub auth_enabled: bool,
    pub auth_user: String,
    pub auth_password: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    let sessions = crate::sessions::router(state.clone());
    let app = Router::new()
        .route("/", get(index))
        .route("/api/config", get(get_config).post(update_config))
        .route("/api/status", get(get_status))
        .route("/api/join", post(join_meeting))
        .route("/api/leave", post(leave_meeting))
        .route("/api/transcript", get(transcript_ws))
        .route("/api/summary", get(get_summary))
        .route("/api/tools/generate", post(generate_tool))
        .merge(sessions);

    // Health endpoint bypasses auth (needed for Traefik/Dokploy health checks)
    let health = Router::new().route("/health", get(|| async { "ok" }));

    // Apply basic auth middleware if enabled
    if state.auth_enabled {
        app.layer(axum::middleware::from_fn_with_state(state.clone(), basic_auth_middleware))
            .with_state(state)
            .merge(health)
    } else {
        app.with_state(state).merge(health)
    }
}

async fn basic_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if let Some(auth_header) = req.headers().get("authorization").and_then(|v| v.to_str().ok()) {
        if let Some(credentials) = auth_header.strip_prefix("Basic ") {
            if let Ok(decoded) = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                credentials,
            ) {
                if let Ok(decoded_str) = String::from_utf8(decoded) {
                    if let Some((user, password)) = decoded_str.split_once(':') {
                        if user == state.auth_user
                            && password == state.auth_password
                        {
                            return next.run(req).await;
                        }
                    }
                }
            }
        }
    }

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", "Basic realm=\"Jarvis\"")
        .body(axum::body::Body::from("Unauthorized"))
        .unwrap()
}

async fn index() -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => {
            Html(String::from_utf8_lossy(content.data.as_ref()).to_string()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// --- Config endpoints ---

#[derive(Serialize)]
struct ConfigResponse {
    meet_url: Option<String>,
    bot_display_name: String,
    tts_voice: String,
    openai_model: String,
    response_mode: String,
    record_video: bool,
    language: String,
    tools: serde_json::Value,
}

async fn get_config(State(state): State<Arc<AppState>>) -> Json<ConfigResponse> {
    let cfg = state.config.read().await;
    Json(ConfigResponse {
        meet_url: cfg.meet_url.clone(),
        bot_display_name: cfg.bot_name.clone(),
        tts_voice: cfg.tts_voice.clone(),
        openai_model: cfg.openai_model.clone(),
        response_mode: match cfg.response_mode {
            crate::config::ResponseMode::NameOnly => "name_only".to_string(),
            crate::config::ResponseMode::Smart => "smart".to_string(),
        },
        record_video: cfg.record_video,
        language: cfg.language.clone(),
        tools: serde_json::json!(cfg.tools),
    })
}

#[derive(Deserialize)]
struct ConfigUpdate {
    meet_url: Option<String>,
    bot_display_name: Option<String>,
    tts_voice: Option<String>,
    openai_model: Option<String>,
    response_mode: Option<String>,
    record_video: Option<bool>,
    language: Option<String>,
    tools: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ConfigUpdateResponse {
    ok: bool,
    needs_restart: bool,
}

async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(update): Json<ConfigUpdate>,
) -> Json<ConfigUpdateResponse> {
    let mut cfg = state.config.write().await;

    if let Some(url) = update.meet_url {
        cfg.meet_url = if url.is_empty() { None } else { Some(url) };
    }
    if let Some(name) = update.bot_display_name {
        cfg.bot_name = name;
    }
    if let Some(voice) = update.tts_voice {
        cfg.tts_voice = voice;
    }
    if let Some(model) = update.openai_model {
        cfg.openai_model = model;
    }
    if let Some(mode) = update.response_mode {
        let new_mode = match mode.as_str() {
            "name_only" => crate::config::ResponseMode::NameOnly,
            _ => crate::config::ResponseMode::Smart,
        };
        cfg.response_mode = new_mode.clone();
        // Propagate to audio task via watch channel
        let _ = state.response_mode_tx.send(new_mode);
    }

    if let Some(record_video) = update.record_video {
        cfg.record_video = record_video;
    }

    if let Some(language) = update.language {
        cfg.language = language;
    }

    if let Some(tools) = update.tools {
        if let Ok(parsed) = serde_json::from_value::<Vec<crate::tools::ToolDef>>(tools) {
            cfg.tools = parsed;
        }
    }

    Json(ConfigUpdateResponse {
        ok: true,
        needs_restart: false,
    })
}

// --- Status endpoint ---

#[derive(Serialize)]
struct StatusResponse {
    bridge_connected: bool,
    meeting_active: bool,
    participant_count: u32,
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let connected = state.bridge_state.connection_count.load(std::sync::atomic::Ordering::Relaxed) > 0;
    let bot_running = state.bot_process.lock().map(|mut p| p.is_running()).unwrap_or(false);
    Json(StatusResponse {
        bridge_connected: connected,
        meeting_active: connected || bot_running,
        participant_count: 0,
    })
}

// --- Join / Leave placeholders ---

#[derive(Deserialize)]
struct JoinRequest {
    meet_url: Option<String>,
}

#[derive(Serialize)]
struct ActionResponse {
    ok: bool,
    message: String,
}

async fn join_meeting(
    State(state): State<Arc<AppState>>,
    Json(body): Json<JoinRequest>,
) -> Json<ActionResponse> {
    // Determine meet URL from request body or config
    let meet_url = match body.meet_url {
        Some(ref url) if !url.is_empty() => url.clone(),
        _ => {
            let cfg = state.config.read().await;
            match cfg.meet_url.clone() {
                Some(url) => url,
                None => {
                    return Json(ActionResponse {
                        ok: false,
                        message: "No meeting URL provided".to_string(),
                    });
                }
            }
        }
    };

    // Update config with the meet URL
    {
        let mut cfg = state.config.write().await;
        cfg.meet_url = Some(meet_url.clone());
    }

    // Find node and vexa-bot directory
    let node_path = match process::find_node() {
        Ok(p) => p,
        Err(e) => {
            return Json(ActionResponse {
                ok: false,
                message: format!("Node.js not found: {}", e),
            });
        }
    };

    let vexa_bot_dir = match process::find_vexa_bot_dir() {
        Ok(p) => p,
        Err(e) => {
            return Json(ActionResponse {
                ok: false,
                message: format!("vexa-bot not found: {}", e),
            });
        }
    };

    let cfg = state.config.read().await;
    let bridge_url = format!("ws://localhost:{}/ws", cfg.bridge_port);
    let bot_name = cfg.bot_name.clone();
    drop(cfg);

    // Start the process
    let mut proc = state.bot_process.lock().unwrap();
    match proc.start(&node_path, &vexa_bot_dir, &bridge_url, &meet_url, &bot_name, false, "", false) {
        Ok(()) => Json(ActionResponse {
            ok: true,
            message: format!("Joining meeting: {}", meet_url),
        }),
        Err(e) => Json(ActionResponse {
            ok: false,
            message: format!("Failed to start vexa-bot: {}", e),
        }),
    }
}

async fn leave_meeting(State(state): State<Arc<AppState>>) -> Json<ActionResponse> {
    let mut proc = state.bot_process.lock().unwrap();
    if !proc.is_running() {
        return Json(ActionResponse {
            ok: false,
            message: "Bot is not currently in a meeting".to_string(),
        });
    }

    match proc.stop() {
        Ok(()) => Json(ActionResponse {
            ok: true,
            message: "Left meeting".to_string(),
        }),
        Err(e) => Json(ActionResponse {
            ok: false,
            message: format!("Failed to stop vexa-bot: {}", e),
        }),
    }
}

// --- Tool generation endpoint ---

#[derive(Deserialize)]
struct ToolGenerateRequest {
    prompt: String,
    #[serde(default)]
    history: Vec<ToolChatMsg>,
}

#[derive(Deserialize)]
struct ToolChatMsg {
    role: String,
    content: String,
}

async fn generate_tool(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ToolGenerateRequest>,
) -> Json<serde_json::Value> {
    let model = state.config.read().await.openai_model.clone();

    let system = r#"You are a tool configuration assistant for Jarvis, an AI meeting bot.
Generate tool definitions as JSON matching this schema:
{
  "name": "tool_name",
  "type": "curl",
  "description": "What the tool does",
  "method": "GET or POST or PUT or DELETE",
  "url": "https://example.com/api",
  "headers": {"Authorization": "Bearer TOKEN"},
  "parameters": {"param_name": "description of parameter"},
  "body_template": {"key": "{{param_name}}"},
  "prompt_template": null,
  "working_directory": null
}

Rules:
- "type" is always "curl" for HTTP tools
- "parameters" describes what the tool accepts
- "body_template" uses {{param}} placeholders that get filled from parameters
- Reply with ONLY valid JSON for the tool object, no markdown, no explanation outside the JSON
- If the user asks to modify a tool, output the complete modified tool JSON"#;

    let mut messages: Vec<(String, String)> = req.history
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    messages.push(("user".to_string(), req.prompt));

    match crate::llm::chat_with_context(&state.openai_key, &state.http_client, &model, system, messages, 0.7, 1000).await {
        Ok(reply) => {
            // Try to parse as JSON tool definition
            match serde_json::from_str::<serde_json::Value>(&reply) {
                Ok(tool_json) => Json(serde_json::json!({
                    "tool": tool_json,
                    "raw": reply
                })),
                Err(_) => {
                    // LLM didn't return pure JSON, return as explanation
                    Json(serde_json::json!({
                        "tool": null,
                        "raw": reply,
                        "error": "Response was not valid JSON. You can manually edit it."
                    }))
                }
            }
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("LLM error: {}", e)
        })),
    }
}

// --- Summary endpoint ---

async fn get_summary(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    if state.agent.transcript_len() == 0 {
        return Json(serde_json::json!({
            "ok": false,
            "message": "No transcript available yet"
        }));
    }

    match state.agent.summary().await {
        Ok(summary) => Json(serde_json::json!({ "ok": true, "summary": summary })),
        Err(e) => Json(serde_json::json!({ "ok": false, "message": format!("{}", e) })),
    }
}

// --- WebSocket transcript broadcast ---

async fn transcript_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_transcript_ws(socket, state))
}

async fn handle_transcript_ws(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.transcript_tx.subscribe();

    // Forward transcript lines to the browser
    let send_task = tokio::spawn(async move {
        while let Ok(line) = rx.recv().await {
            let payload = serde_json::json!({ "text": line });
            if sender
                .send(Message::Text(payload.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Keep connection alive by draining incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }

    send_task.abort();
}
