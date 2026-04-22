# Sessions UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Sessions tab to the Jarvis Web UI with session history browsing, audio/video playback, file downloads, and AI chat with transcriptions.

**Architecture:** New `sessions.rs` module handles all session-related API endpoints. Session data is file-based (scan `sessions/` directory). AI chat uses standalone OpenAI calls (not the live `LlmAgent`) via a new public `chat_once` helper in `llm.rs`. Frontend is a tabbed SPA in the single embedded `index.html`.

**Tech Stack:** Rust/Axum (backend), vanilla JS/HTML/CSS (frontend), OpenAI API (chat)

**Design doc:** `docs/plans/2026-04-22-sessions-ui-design.md`

---

### Task 1: Expose `data_dir` in AppState and make `chat_once` reusable

**Files:**
- Modify: `jarvis/src/server.rs` — add `data_dir` and `openai_key` to `AppState`
- Modify: `jarvis/src/main.rs` — pass `data_dir` and `openai_key` when constructing `AppState`
- Modify: `jarvis/src/llm.rs` — extract a public standalone `chat_with_context()` function

**Step 1: Add fields to AppState**

In `jarvis/src/server.rs`, add to the `AppState` struct:

```rust
pub struct AppState {
    pub config: RwLock<Config>,
    pub transcript_tx: broadcast::Sender<String>,
    pub bridge_state: Arc<BridgeState>,
    pub agent: Arc<LlmAgent>,
    pub bot_process: Arc<std::sync::Mutex<process::VexaBotProcess>>,
    pub response_mode_tx: tokio::sync::watch::Sender<crate::config::ResponseMode>,
    pub data_dir: std::path::PathBuf,   // NEW
    pub openai_key: String,             // NEW
}
```

**Step 2: Pass new fields in main.rs**

In `jarvis/src/main.rs`, update AppState construction (~line 241):

```rust
let app_state = Arc::new(server::AppState {
    config: tokio::sync::RwLock::new(cfg.clone()),
    transcript_tx: transcript_tx.clone(),
    bridge_state: bridge_state.clone(),
    agent: agent.clone(),
    bot_process: bot_process.clone(),
    response_mode_tx,
    data_dir: cfg.data_dir.clone(),       // NEW
    openai_key: cfg.openai_key.clone(),   // NEW
});
```

**Step 3: Add standalone chat function to llm.rs**

Add this public function at the module level in `jarvis/src/llm.rs` (outside the `impl LlmAgent` block):

```rust
/// Standalone LLM chat for session review — does NOT use live agent history.
/// Used by session chat endpoints to avoid polluting live meeting context.
pub async fn chat_with_context(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    messages: Vec<(String, String)>,  // (role, content) pairs
    temperature: f32,
    max_tokens: u32,
) -> anyhow::Result<String> {
    let client = Client::new();
    let is_reasoning = crate::config::is_reasoning_model(model);

    let mut chat_messages = vec![ChatMessage {
        role: "system".to_string(),
        content: system_prompt.to_string(),
    }];
    for (role, content) in messages {
        chat_messages.push(ChatMessage { role, content });
    }

    let req = ChatRequest {
        model: model.to_string(),
        messages: chat_messages,
        temperature: if is_reasoning { None } else { Some(temperature) },
        max_completion_tokens: if is_reasoning { max_tokens.max(1000) } else { max_tokens },
        reasoning_effort: if is_reasoning { Some("low".to_string()) } else { None },
    };

    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&req)
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        let api_msg = serde_json::from_str::<ApiErrorResponse>(&body)
            .ok()
            .and_then(|r| r.error)
            .map(|e| e.message)
            .unwrap_or_else(|| body.clone());
        anyhow::bail!("OpenAI API error (HTTP {}): {}", status, api_msg);
    }

    let chat_resp: ChatResponse = serde_json::from_str(&body)?;
    Ok(chat_resp
        .choices
        .first()
        .map(|c| c.message.content.as_deref().unwrap_or("").trim().to_string())
        .unwrap_or_default())
}
```

**Step 4: Build and verify**

Run: `cd jarvis && cargo build 2>&1`
Expected: Compiles with no errors (new fields initialized, new function unused but compiles).

**Step 5: Commit**

```
feat: expose data_dir in AppState and add standalone chat_with_context
```

---

### Task 2: Create `sessions.rs` — session scanner and list endpoint

**Files:**
- Create: `jarvis/src/sessions.rs`
- Modify: `jarvis/src/main.rs` — add `mod sessions;`
- Modify: `jarvis/src/server.rs` — mount session routes

**Step 1: Create `jarvis/src/sessions.rs` with scanner + list endpoint**

```rust
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::server::AppState;

/// Session metadata for the list endpoint
#[derive(Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub date: String,
    pub preview: String,
    pub has_audio: bool,
    pub has_video: bool,
    pub has_transcript: bool,
    pub audio_size: Option<u64>,
    pub video_size: Option<u64>,
    pub transcript_size: Option<u64>,
    pub video_format: Option<String>,
}

#[derive(Serialize)]
struct SessionListResponse {
    sessions: Vec<SessionInfo>,
    total: usize,
    limit: usize,
    offset: usize,
}

#[derive(Deserialize)]
struct ListParams {
    limit: Option<usize>,
    offset: Option<usize>,
}

/// Scan the sessions directory and return grouped session metadata.
fn scan_sessions(sessions_dir: &std::path::Path) -> Vec<SessionInfo> {
    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    // Collect all files, group by timestamp prefix
    let mut file_map: std::collections::HashMap<String, Vec<(String, u64)>> =
        std::collections::HashMap::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };

        // Parse timestamp prefix: YYYY-MM-DD_HHMMSS
        // Filename examples: 2026-04-22_143000.txt, 2026-04-22_143000.wav
        let stem = match filename.find('.') {
            Some(pos) => &filename[..pos],
            None => continue,
        };

        // Validate timestamp format (17 chars: YYYY-MM-DD_HHMMSS)
        if stem.len() != 17 || stem.chars().nth(10) != Some('_') {
            continue;
        }

        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        // Skip files modified within last 60 seconds (active session)
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                if modified.elapsed().map(|d| d.as_secs() < 60).unwrap_or(false) {
                    continue;
                }
            }
        }

        file_map
            .entry(stem.to_string())
            .or_default()
            .push((filename, size));
    }

    let mut sessions: Vec<SessionInfo> = file_map
        .into_iter()
        .map(|(id, files)| {
            let mut info = SessionInfo {
                date: format!(
                    "{} {}:{}:{}",
                    &id[..10],
                    &id[11..13],
                    &id[13..15],
                    &id[15..17]
                ),
                id,
                preview: String::new(),
                has_audio: false,
                has_video: false,
                has_transcript: false,
                audio_size: None,
                video_size: None,
                transcript_size: None,
                video_format: None,
            };

            for (filename, size) in &files {
                if filename.ends_with(".txt") {
                    info.has_transcript = true;
                    info.transcript_size = Some(*size);
                } else if filename.ends_with(".wav") {
                    info.has_audio = true;
                    info.audio_size = Some(*size);
                } else if filename.ends_with(".webm") {
                    info.has_video = true;
                    info.video_size = Some(*size);
                    info.video_format = Some("webm".to_string());
                } else if filename.ends_with(".mkv") {
                    info.has_video = true;
                    info.video_size = Some(*size);
                    info.video_format = Some("mkv".to_string());
                }
            }

            // Read first non-empty transcript line for preview
            if info.has_transcript {
                let txt_path = sessions_dir.join(format!("{}.txt", &info.id));
                if let Ok(content) = std::fs::read_to_string(&txt_path) {
                    info.preview = content
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .chars()
                        .take(120)
                        .collect();
                }
            }

            info
        })
        .collect();

    // Sort by id descending (newest first)
    sessions.sort_by(|a, b| b.id.cmp(&a.id));
    sessions
}

/// Helper: resolve session directory path
fn sessions_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("sessions")
}

/// Helper: validate session ID format to prevent path traversal
fn is_valid_session_id(id: &str) -> bool {
    id.len() == 17
        && id.chars().nth(10) == Some('_')
        && id.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '_')
}

// --- Handlers ---

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListParams>,
) -> Json<SessionListResponse> {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);

    let all = scan_sessions(&sessions_dir(&state));
    let total = all.len();
    let page: Vec<SessionInfo> = all.into_iter().skip(offset).take(limit).collect();

    Json(SessionListResponse {
        sessions: page,
        total,
        limit,
        offset,
    })
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .with_state(state)
}
```

**Step 2: Register the module in main.rs**

Add `mod sessions;` alongside existing module declarations (~line 9):

```rust
mod sessions;
```

**Step 3: Mount session routes in server.rs**

In `server.rs` `router()` function, merge the sessions router:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    let sessions = crate::sessions::router(state.clone());
    Router::new()
        .route("/", get(index))
        .route("/api/config", get(get_config).post(update_config))
        .route("/api/status", get(get_status))
        .route("/api/join", post(join_meeting))
        .route("/api/leave", post(leave_meeting))
        .route("/api/transcript", get(transcript_ws))
        .route("/api/summary", get(get_summary))
        .merge(sessions)
        .with_state(state)
}
```

**Step 4: Build and verify**

Run: `cd jarvis && cargo build 2>&1`
Expected: Compiles. Test manually: `curl http://localhost:8080/api/sessions` should return `{"sessions":[],"total":0,"limit":20,"offset":0}` (or populated if sessions exist).

**Step 5: Commit**

```
feat: add sessions scanner and GET /api/sessions endpoint
```

---

### Task 3: Transcript endpoints (JSON + raw download)

**Files:**
- Modify: `jarvis/src/sessions.rs`

**Step 1: Add transcript handlers**

Append to `sessions.rs`:

```rust
#[derive(Serialize)]
struct TranscriptResponse {
    text: String,
}

async fn get_transcript(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<TranscriptResponse>, StatusCode> {
    if !is_valid_session_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = sessions_dir(&state).join(format!("{}.txt", id));
    let text = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(TranscriptResponse { text }))
}

async fn download_transcript(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_valid_session_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = sessions_dir(&state).join(format!("{}.txt", id));
    let text = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;

    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}.txt\"", id),
            ),
        ],
        text,
    ))
}
```

**Step 2: Add routes**

Update the `router()` function in `sessions.rs`:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}/transcript", get(get_transcript))
        .route("/api/sessions/{id}/transcript/download", get(download_transcript))
        .with_state(state)
}
```

**Step 3: Build and verify**

Run: `cd jarvis && cargo build 2>&1`

**Step 4: Commit**

```
feat: add transcript JSON and download endpoints
```

---

### Task 4: Audio and video file serving with Range support

**Files:**
- Modify: `jarvis/src/sessions.rs`

**Step 1: Add file serving handler with Range support**

Add to `sessions.rs`:

```rust
use axum::http::{header, HeaderMap, HeaderValue};
use axum::body::Body;
use tokio::io::AsyncReadExt;

async fn serve_file(
    state: &AppState,
    id: &str,
    extensions: &[&str],
    content_types: &[&str],
    headers: &HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_valid_session_id(id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let dir = sessions_dir(state);
    let mut file_path = None;
    let mut content_type = "";

    for (ext, ct) in extensions.iter().zip(content_types.iter()) {
        let p = dir.join(format!("{}.{}", id, ext));
        if p.exists() {
            file_path = Some(p);
            content_type = ct;
            break;
        }
    }

    let file_path = file_path.ok_or(StatusCode::NOT_FOUND)?;
    let file_size = std::fs::metadata(&file_path)
        .map(|m| m.len())
        .map_err(|_| StatusCode::NOT_FOUND)?;

    // Parse Range header
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("bytes="))
        .and_then(|s| {
            let parts: Vec<&str> = s.splitn(2, '-').collect();
            let start: u64 = parts[0].parse().ok()?;
            let end: u64 = if parts.len() > 1 && !parts[1].is_empty() {
                parts[1].parse().ok()?
            } else {
                file_size - 1
            };
            Some((start, end))
        });

    match range {
        Some((start, end)) => {
            let end = end.min(file_size - 1);
            let len = end - start + 1;

            let mut file = tokio::fs::File::open(&file_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let mut buf = vec![0u8; len as usize];
            file.read_exact(&mut buf)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            Ok((
                StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (
                        header::CONTENT_RANGE,
                        HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, file_size))
                            .unwrap(),
                    ),
                    (
                        header::CONTENT_LENGTH,
                        HeaderValue::from_str(&len.to_string()).unwrap(),
                    ),
                    (
                        header::ACCEPT_RANGES,
                        HeaderValue::from_static("bytes"),
                    ),
                ],
                Body::from(buf),
            )
                .into_response())
        }
        None => {
            // Serve entire file
            let bytes = tokio::fs::read(&file_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            Ok((
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (
                        header::CONTENT_LENGTH,
                        HeaderValue::from_str(&file_size.to_string()).unwrap(),
                    ),
                    (
                        header::ACCEPT_RANGES,
                        HeaderValue::from_static("bytes"),
                    ),
                ],
                Body::from(bytes),
            )
                .into_response())
        }
    }
}

use tokio::io::AsyncSeekExt;

async fn get_audio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    serve_file(&state, &id, &["wav"], &["audio/wav"], &headers).await
}

async fn get_video(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    serve_file(
        &state,
        &id,
        &["webm", "mkv"],
        &["video/webm", "video/x-matroska"],
        &headers,
    )
    .await
}
```

**Step 2: Add routes**

```rust
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}/transcript", get(get_transcript))
        .route("/api/sessions/{id}/transcript/download", get(download_transcript))
        .route("/api/sessions/{id}/audio", get(get_audio))
        .route("/api/sessions/{id}/video", get(get_video))
        .with_state(state)
}
```

**Step 3: Build and verify**

Run: `cd jarvis && cargo build 2>&1`

**Step 4: Commit**

```
feat: add audio/video file serving with HTTP Range support
```

---

### Task 5: Search endpoint

**Files:**
- Modify: `jarvis/src/sessions.rs`

**Step 1: Add search handler**

```rust
#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    max_results: Option<usize>,
}

#[derive(Serialize)]
struct SearchMatch {
    line: usize,
    text: String,
    context_before: Option<String>,
    context_after: Option<String>,
}

#[derive(Serialize)]
struct SearchSessionResult {
    session_id: String,
    session_date: String,
    matches: Vec<SearchMatch>,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchSessionResult>,
}

async fn search_sessions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SearchRequest>,
) -> Json<SearchResponse> {
    let query = body.query.to_lowercase();
    let max_results = body.max_results.unwrap_or(100).min(500);
    let dir = sessions_dir(&state);

    let mut results = Vec::new();
    let mut total_matches = 0usize;

    // Get sorted list of transcript files (newest first)
    let mut txt_files: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                == Some("txt")
        })
        .collect();
    txt_files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    for entry in txt_files {
        if total_matches >= max_results {
            break;
        }

        let path = entry.path();
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if is_valid_session_id(s) => s.to_string(),
            _ => continue,
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut session_matches = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            if total_matches >= max_results {
                break;
            }
            if line.to_lowercase().contains(&query) {
                session_matches.push(SearchMatch {
                    line: i + 1,
                    text: line.to_string(),
                    context_before: if i > 0 {
                        Some(lines[i - 1].to_string())
                    } else {
                        None
                    },
                    context_after: if i + 1 < lines.len() {
                        Some(lines[i + 1].to_string())
                    } else {
                        None
                    },
                });
                total_matches += 1;
            }
        }

        if !session_matches.is_empty() {
            let date = format!(
                "{} {}:{}:{}",
                &stem[..10],
                &stem[11..13],
                &stem[13..15],
                &stem[15..17]
            );
            results.push(SearchSessionResult {
                session_id: stem,
                session_date: date,
                matches: session_matches,
            });
        }
    }

    Json(SearchResponse { results })
}
```

**Step 2: Add route**

```rust
.route("/api/sessions/search", post(search_sessions))
```

**Important:** This route MUST be registered before the `{id}` routes to avoid the path `search` being captured as an `{id}`. Update route order:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/search", post(search_sessions))
        .route("/api/sessions/{id}/transcript", get(get_transcript))
        .route("/api/sessions/{id}/transcript/download", get(download_transcript))
        .route("/api/sessions/{id}/audio", get(get_audio))
        .route("/api/sessions/{id}/video", get(get_video))
        .with_state(state)
}
```

**Step 3: Build and verify**

Run: `cd jarvis && cargo build 2>&1`

**Step 4: Commit**

```
feat: add cross-session text search endpoint
```

---

### Task 6: Session chat endpoints (single + cross-session)

**Files:**
- Modify: `jarvis/src/sessions.rs`

**Step 1: Add chat request/response types and handlers**

```rust
#[derive(Deserialize)]
struct ChatHistoryMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct SessionChatRequest {
    message: String,
    #[serde(default)]
    history: Vec<ChatHistoryMessage>,
}

#[derive(Deserialize)]
struct CrossSessionChatRequest {
    message: String,
    session_ids: Vec<String>,
    #[serde(default)]
    history: Vec<ChatHistoryMessage>,
}

#[derive(Serialize)]
struct ChatResponse {
    reply: String,
}

#[derive(Serialize)]
struct ChatErrorResponse {
    error: String,
}

async fn session_chat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SessionChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, Json<ChatErrorResponse>)> {
    if !is_valid_session_id(&id) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ChatErrorResponse {
                error: "Invalid session ID".to_string(),
            }),
        ));
    }

    let path = sessions_dir(&state).join(format!("{}.txt", id));
    let transcript = std::fs::read_to_string(&path).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ChatErrorResponse {
                error: "Session transcript not found".to_string(),
            }),
        )
    })?;

    let model = {
        let cfg = state.config.read().await;
        cfg.openai_model.clone()
    };

    let system = format!(
        "You are a helpful assistant analyzing a meeting transcript.\n\
         Answer questions based on the transcript below. Be concise and accurate.\n\
         If the answer isn't in the transcript, say so.\n\n\
         TRANSCRIPT:\n{}",
        truncate_text(&transcript, 12000)
    );

    let mut messages: Vec<(String, String)> = body
        .history
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    messages.push(("user".to_string(), body.message));

    match crate::llm::chat_with_context(&state.openai_key, &model, &system, messages, 0.7, 1000)
        .await
    {
        Ok(reply) => Ok(Json(ChatResponse { reply })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ChatErrorResponse {
                error: format!("LLM error: {}", e),
            }),
        )),
    }
}

async fn cross_session_chat(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CrossSessionChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, Json<ChatErrorResponse>)> {
    if body.session_ids.is_empty() || body.session_ids.len() > 3 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ChatErrorResponse {
                error: "Select 1-3 sessions".to_string(),
            }),
        ));
    }

    let dir = sessions_dir(&state);
    let mut combined = String::new();

    for sid in &body.session_ids {
        if !is_valid_session_id(sid) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ChatErrorResponse {
                    error: format!("Invalid session ID: {}", sid),
                }),
            ));
        }
        let path = dir.join(format!("{}.txt", sid));
        let text = std::fs::read_to_string(&path).map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ChatErrorResponse {
                    error: format!("Session not found: {}", sid),
                }),
            )
        })?;
        let date = format!(
            "{} {}:{}:{}",
            &sid[..10],
            &sid[11..13],
            &sid[13..15],
            &sid[15..17]
        );
        combined.push_str(&format!(
            "\n=== SESSION: {} ===\n{}\n",
            date,
            truncate_text(&text, 4000)
        ));
    }

    let model = {
        let cfg = state.config.read().await;
        cfg.openai_model.clone()
    };

    let system = format!(
        "You are a helpful assistant analyzing meeting transcripts from multiple sessions.\n\
         Answer questions based on the transcripts below. Be concise and accurate.\n\
         Reference which session/date information comes from when relevant.\n\
         If the answer isn't in the transcripts, say so.\n\n\
         TRANSCRIPTS:\n{}",
        combined
    );

    let mut messages: Vec<(String, String)> = body
        .history
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    messages.push(("user".to_string(), body.message));

    match crate::llm::chat_with_context(&state.openai_key, &model, &system, messages, 0.7, 1000)
        .await
    {
        Ok(reply) => Ok(Json(ChatResponse { reply })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ChatErrorResponse {
                error: format!("LLM error: {}", e),
            }),
        )),
    }
}

/// Truncate text to approximately `max_chars` characters, breaking at line boundary.
fn truncate_text(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        return text;
    }
    // Find the last newline before max_chars
    match text[..max_chars].rfind('\n') {
        Some(pos) => &text[..pos],
        None => &text[..max_chars],
    }
}
```

**Step 2: Add routes**

Update router to include chat routes. Note: `/api/sessions/chat` (cross-session) must come before `{id}` routes:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/search", post(search_sessions))
        .route("/api/sessions/chat", post(cross_session_chat))
        .route("/api/sessions/{id}/transcript", get(get_transcript))
        .route("/api/sessions/{id}/transcript/download", get(download_transcript))
        .route("/api/sessions/{id}/audio", get(get_audio))
        .route("/api/sessions/{id}/video", get(get_video))
        .route("/api/sessions/{id}/chat", post(session_chat))
        .with_state(state)
}
```

**Step 3: Build and verify**

Run: `cd jarvis && cargo build 2>&1`

**Step 4: Commit**

```
feat: add session chat and cross-session chat endpoints
```

---

### Task 7: Frontend — tab system and session list

**Files:**
- Modify: `jarvis/src/assets/index.html`

**Step 1: Add tab CSS and restructure HTML**

Replace the entire `index.html` content. This is a large change — the full file replaces the existing one. Key structural changes:

- Wrap existing dashboard content in `<div id="tab-dashboard" class="tab-content active">`
- Add `<div id="tab-sessions" class="tab-content">` with session list + detail panels
- Add tab navigation bar below header
- Keep ALL existing JS functions intact

CSS additions (add to existing `<style>` block):

```css
/* Tab navigation */
.tab-nav { display: flex; background: #16213e; border-bottom: 1px solid #0f3460; padding: 0 24px; }
.tab-nav button { background: none; border: none; color: #888; padding: 12px 20px; cursor: pointer; font-size: 14px; font-weight: 600; border-bottom: 2px solid transparent; }
.tab-nav button.active { color: #e94560; border-bottom-color: #e94560; }
.tab-nav button:hover { color: #e0e0e0; }
.tab-content { display: none; }
.tab-content.active { display: block; }

/* Sessions tab */
.sessions-layout { display: grid; grid-template-columns: 350px 1fr; gap: 16px; padding: 16px; max-width: 1400px; margin: 0 auto; min-height: calc(100vh - 110px); }
.session-list { background: #16213e; border-radius: 8px; border: 1px solid #0f3460; overflow: hidden; display: flex; flex-direction: column; }
.session-list-header { padding: 16px; border-bottom: 1px solid #0f3460; }
.session-list-header h2 { font-size: 14px; text-transform: uppercase; color: #888; letter-spacing: 1px; margin-bottom: 12px; }
.session-search { width: 100%; padding: 8px 12px; background: #1a1a2e; border: 1px solid #0f3460; border-radius: 4px; color: #e0e0e0; font-size: 13px; }
.session-search:focus { outline: none; border-color: #e94560; }
.session-items { flex: 1; overflow-y: auto; }
.session-item { padding: 12px 16px; border-bottom: 1px solid #0f3460; cursor: pointer; }
.session-item:hover { background: #1a1a2e; }
.session-item.active { background: #0f3460; border-left: 3px solid #e94560; }
.session-item .date { font-size: 13px; color: #e94560; font-weight: 600; }
.session-item .preview { font-size: 12px; color: #888; margin-top: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.session-item .badges { margin-top: 4px; display: flex; gap: 6px; }
.session-item .badge { font-size: 10px; padding: 2px 6px; border-radius: 3px; background: #0f3460; color: #aaa; }
.session-empty { padding: 40px 20px; text-align: center; color: #666; font-size: 14px; }
.load-more-btn { width: 100%; padding: 10px; background: none; border: none; border-top: 1px solid #0f3460; color: #e94560; cursor: pointer; font-size: 13px; }
.load-more-btn:hover { background: #1a1a2e; }

/* Session detail */
.session-detail { background: #16213e; border-radius: 8px; border: 1px solid #0f3460; padding: 20px; overflow-y: auto; }
.session-detail h2 { font-size: 14px; text-transform: uppercase; color: #888; letter-spacing: 1px; margin-bottom: 16px; }
.session-detail-empty { display: flex; align-items: center; justify-content: center; color: #666; font-size: 14px; }
.detail-section { margin-bottom: 20px; }
.detail-section h3 { font-size: 13px; color: #aaa; margin-bottom: 8px; text-transform: uppercase; letter-spacing: 0.5px; }
.transcript-viewer { background: #0d1117; border-radius: 4px; padding: 12px; max-height: 300px; overflow-y: auto; font-family: monospace; font-size: 13px; line-height: 1.6; white-space: pre-wrap; }
.download-bar { display: flex; gap: 8px; flex-wrap: wrap; }
.download-bar .btn { font-size: 12px; padding: 6px 14px; }

/* Media players */
.session-detail audio { width: 100%; margin-top: 4px; }
.session-detail video { width: 100%; border-radius: 4px; margin-top: 4px; max-height: 400px; }

/* Chat */
.chat-section { margin-top: 20px; }
.chat-messages { background: #0d1117; border-radius: 4px; padding: 12px; max-height: 250px; overflow-y: auto; margin-bottom: 8px; min-height: 60px; }
.chat-msg { margin-bottom: 8px; font-size: 13px; line-height: 1.5; }
.chat-msg.user { color: #4ecca3; }
.chat-msg.assistant { color: #e0e0e0; }
.chat-msg .role { font-weight: 600; margin-right: 6px; }
.chat-input-row { display: flex; gap: 8px; }
.chat-input { flex: 1; padding: 8px 12px; background: #1a1a2e; border: 1px solid #0f3460; border-radius: 4px; color: #e0e0e0; font-size: 13px; }
.chat-input:focus { outline: none; border-color: #e94560; }

/* Search results */
.search-results { padding: 8px 16px; }
.search-result-item { padding: 8px 0; border-bottom: 1px solid #0f3460; }
.search-result-item .match-text { font-family: monospace; font-size: 12px; background: #0d1117; padding: 4px 8px; border-radius: 3px; margin-top: 4px; }
.search-result-item .match-text mark { background: #e94560; color: white; padding: 0 2px; border-radius: 2px; }
.search-result-item .context-line { color: #555; font-style: italic; }
.search-check { margin-right: 8px; }
.search-chat-bar { padding: 12px 16px; border-top: 1px solid #0f3460; }

@media (max-width: 768px) {
  .sessions-layout { grid-template-columns: 1fr; }
}
```

**Step 2: Add tab navigation HTML after header**

```html
<div class="tab-nav">
  <button class="active" onclick="switchTab('dashboard')">Dashboard</button>
  <button onclick="switchTab('sessions')">Sessions</button>
</div>
```

**Step 3: Wrap existing main content**

Wrap existing `<div class="main">...</div>` content in:
```html
<div id="tab-dashboard" class="tab-content active">
  <!-- existing .main div here -->
</div>
```

**Step 4: Add Sessions tab HTML**

```html
<div id="tab-sessions" class="tab-content">
  <div class="sessions-layout">
    <div class="session-list">
      <div class="session-list-header">
        <h2>Sessions</h2>
        <input type="text" class="session-search" id="session-search" placeholder="Search across all sessions..." onkeydown="if(event.key==='Enter')searchSessions()">
      </div>
      <div class="session-items" id="session-items"></div>
      <button class="load-more-btn" id="load-more-btn" style="display:none" onclick="loadMoreSessions()">Load more</button>
    </div>
    <div class="session-detail session-detail-empty" id="session-detail">
      Select a session to view details
    </div>
  </div>
</div>
```

**Step 5: Add Sessions tab JS**

Add the following JS functions to the `<script>` block:

```javascript
// --- Tab System ---
function switchTab(tab) {
  document.querySelectorAll('.tab-content').forEach(el => el.classList.remove('active'));
  document.querySelectorAll('.tab-nav button').forEach(el => el.classList.remove('active'));
  document.getElementById('tab-' + tab).classList.add('active');
  document.querySelector('.tab-nav button[onclick*="' + tab + '"]').classList.add('active');
  if (tab === 'sessions' && !sessionsLoaded) loadSessions();
  location.hash = tab;
}

// --- Sessions State ---
let sessionsLoaded = false;
let sessionsOffset = 0;
let sessionsTotal = 0;
let currentSessionId = null;
let chatHistory = [];
let searchMode = false;
let selectedSearchSessions = new Set();

async function loadSessions() {
  sessionsOffset = 0;
  try {
    const r = await fetch('/api/sessions?limit=20&offset=0');
    const data = await r.json();
    sessionsTotal = data.total;
    sessionsOffset = data.sessions.length;
    renderSessionList(data.sessions, false);
    sessionsLoaded = true;
    $('load-more-btn').style.display = sessionsOffset < sessionsTotal ? '' : 'none';
  } catch(e) { console.error('loadSessions', e); }
}

async function loadMoreSessions() {
  try {
    const r = await fetch('/api/sessions?limit=20&offset=' + sessionsOffset);
    const data = await r.json();
    sessionsOffset += data.sessions.length;
    renderSessionList(data.sessions, true);
    $('load-more-btn').style.display = sessionsOffset < sessionsTotal ? '' : 'none';
  } catch(e) { console.error('loadMoreSessions', e); }
}

function renderSessionList(sessions, append) {
  const container = $('session-items');
  if (!append) container.innerHTML = '';
  searchMode = false;

  if (sessions.length === 0 && !append) {
    container.innerHTML = '<div class="session-empty">No sessions yet. Join a meeting from the Dashboard to get started.</div>';
    return;
  }

  for (const s of sessions) {
    const div = document.createElement('div');
    div.className = 'session-item' + (s.id === currentSessionId ? ' active' : '');
    div.onclick = () => openSession(s.id);
    let badges = '';
    if (s.has_transcript) badges += '<span class="badge">TXT</span>';
    if (s.has_audio) badges += '<span class="badge">WAV</span>';
    if (s.has_video) badges += '<span class="badge">' + (s.video_format || 'VID').toUpperCase() + '</span>';
    div.innerHTML = '<div class="date">' + s.date + '</div>'
      + '<div class="preview">' + escHtml(s.preview || 'No transcript') + '</div>'
      + '<div class="badges">' + badges + '</div>';
    container.appendChild(div);
  }
}

function escHtml(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

// --- Session Detail ---
async function openSession(id) {
  currentSessionId = id;
  chatHistory = [];
  document.querySelectorAll('.session-item').forEach(el => el.classList.remove('active'));
  document.querySelectorAll('.session-item').forEach(el => {
    if (el.onclick && el.querySelector('.date')) el.classList.toggle('active', el.dataset.sid === id);
  });
  // Re-mark active by matching
  document.querySelectorAll('.session-item').forEach(el => {
    const dateEl = el.querySelector('.date');
    if (dateEl) el.dataset.sid = el.dataset.sid || '';
  });

  const detail = $('session-detail');
  detail.className = 'session-detail';
  detail.innerHTML = '<div style="color:#888">Loading...</div>';

  try {
    // Fetch session info for available files
    const listR = await fetch('/api/sessions?limit=1&offset=0');
    const listData = await listR.json();

    // Fetch transcript
    let transcript = '';
    try {
      const tr = await fetch('/api/sessions/' + id + '/transcript');
      const td = await tr.json();
      transcript = td.text || '';
    } catch(e) {}

    let html = '';

    // Transcript
    html += '<div class="detail-section"><h3>Transcript</h3>';
    html += '<div class="transcript-viewer">' + escHtml(transcript || 'No transcript available') + '</div></div>';

    // Audio
    html += '<div class="detail-section"><h3>Audio</h3>';
    html += '<audio controls preload="metadata" src="/api/sessions/' + id + '/audio"></audio></div>';

    // Video - check by trying to load
    html += '<div class="detail-section" id="video-section" style="display:none"><h3>Video</h3>';
    html += '<video controls preload="metadata" id="session-video" src="/api/sessions/' + id + '/video"></video></div>';

    // Downloads
    html += '<div class="detail-section"><h3>Downloads</h3><div class="download-bar">';
    html += '<a class="btn btn-primary" href="/api/sessions/' + id + '/transcript/download" download>Transcript (.txt)</a>';
    html += '<a class="btn btn-primary" href="/api/sessions/' + id + '/audio" download="' + id + '.wav">Audio (.wav)</a>';
    html += '<a class="btn btn-primary" id="video-download" href="/api/sessions/' + id + '/video" download style="display:none">Video</a>';
    html += '</div></div>';

    // Chat
    html += '<div class="detail-section chat-section"><h3>Chat with Transcript</h3>';
    html += '<div class="chat-messages" id="chat-messages"></div>';
    html += '<div class="chat-input-row">';
    html += '<input type="text" class="chat-input" id="chat-input" placeholder="Ask about this meeting..." onkeydown="if(event.key===\'Enter\')sendChat()">';
    html += '<button class="btn btn-primary" onclick="sendChat()">Send</button>';
    html += '</div></div>';

    detail.innerHTML = html;

    // Check if video exists
    const videoEl = document.getElementById('session-video');
    if (videoEl) {
      videoEl.onerror = () => {
        document.getElementById('video-section').style.display = 'none';
        document.getElementById('video-download').style.display = 'none';
      };
      videoEl.onloadedmetadata = () => {
        document.getElementById('video-section').style.display = '';
        document.getElementById('video-download').style.display = '';
      };
      // Also try HEAD request to detect mkv (won't play but can download)
      fetch('/api/sessions/' + id + '/video', {method:'HEAD'}).then(r => {
        if (r.ok) {
          document.getElementById('video-download').style.display = '';
          const ct = r.headers.get('content-type') || '';
          if (ct.includes('matroska')) {
            document.getElementById('video-section').style.display = 'none'; // mkv can't play
          }
        }
      }).catch(() => {});
    }
  } catch(e) {
    detail.innerHTML = '<div style="color:#e94560">Failed to load session: ' + e.message + '</div>';
  }
}

// --- Chat ---
async function sendChat() {
  const input = $('chat-input');
  const msg = input.value.trim();
  if (!msg || !currentSessionId) return;
  input.value = '';

  const msgs = $('chat-messages');
  msgs.innerHTML += '<div class="chat-msg user"><span class="role">You:</span>' + escHtml(msg) + '</div>';
  msgs.innerHTML += '<div class="chat-msg assistant" id="chat-pending"><span class="role">Jarvis:</span> <em>Thinking...</em></div>';
  msgs.scrollTop = msgs.scrollHeight;

  try {
    const r = await fetch('/api/sessions/' + currentSessionId + '/chat', {
      method: 'POST',
      headers: {'Content-Type':'application/json'},
      body: JSON.stringify({ message: msg, history: chatHistory })
    });
    const data = await r.json();
    const pending = document.getElementById('chat-pending');
    if (data.reply) {
      chatHistory.push({ role: 'user', content: msg });
      chatHistory.push({ role: 'assistant', content: data.reply });
      if (pending) pending.innerHTML = '<span class="role">Jarvis:</span>' + escHtml(data.reply);
    } else {
      if (pending) pending.innerHTML = '<span class="role">Jarvis:</span> <em style="color:#e94560">' + escHtml(data.error || 'Error') + '</em>';
    }
  } catch(e) {
    const pending = document.getElementById('chat-pending');
    if (pending) pending.innerHTML = '<span class="role">Jarvis:</span> <em style="color:#e94560">Request failed</em>';
  }
  msgs.scrollTop = msgs.scrollHeight;
}

// --- Search ---
async function searchSessions() {
  const query = $('session-search').value.trim();
  if (!query) { loadSessions(); return; }

  const container = $('session-items');
  container.innerHTML = '<div class="session-empty">Searching...</div>';
  selectedSearchSessions.clear();

  try {
    const r = await fetch('/api/sessions/search', {
      method: 'POST',
      headers: {'Content-Type':'application/json'},
      body: JSON.stringify({ query, max_results: 100 })
    });
    const data = await r.json();
    searchMode = true;
    container.innerHTML = '';
    $('load-more-btn').style.display = 'none';

    if (data.results.length === 0) {
      container.innerHTML = '<div class="session-empty">No matches found for \'' + escHtml(query) + '\'</div>';
      return;
    }

    for (const result of data.results) {
      const div = document.createElement('div');
      div.className = 'search-result-item';
      let matchHtml = '';
      for (const m of result.matches.slice(0, 3)) {
        const highlighted = escHtml(m.text).replace(new RegExp('(' + escRegex(query) + ')', 'gi'), '<mark>$1</mark>');
        matchHtml += '<div class="match-text">';
        if (m.context_before) matchHtml += '<div class="context-line">' + escHtml(m.context_before) + '</div>';
        matchHtml += highlighted;
        if (m.context_after) matchHtml += '<div class="context-line">' + escHtml(m.context_after) + '</div>';
        matchHtml += '</div>';
      }
      if (result.matches.length > 3) matchHtml += '<div style="color:#888;font-size:11px;margin-top:4px">+' + (result.matches.length - 3) + ' more matches</div>';

      div.innerHTML = '<div style="display:flex;align-items:center">'
        + '<input type="checkbox" class="search-check" data-sid="' + result.session_id + '" onchange="toggleSearchSession(this)">'
        + '<span class="date" style="cursor:pointer;color:#e94560;font-weight:600" onclick="openSession(\'' + result.session_id + '\')">' + result.session_date + '</span>'
        + '</div>' + matchHtml;
      container.appendChild(div);
    }

    // Add cross-session chat bar
    const chatBar = document.createElement('div');
    chatBar.className = 'search-chat-bar';
    chatBar.id = 'search-chat-bar';
    chatBar.innerHTML = '<div class="chat-input-row">'
      + '<input type="text" class="chat-input" id="cross-chat-input" placeholder="Ask across selected sessions..." onkeydown="if(event.key===\'Enter\')sendCrossChat()">'
      + '<button class="btn btn-primary" onclick="sendCrossChat()">Ask</button></div>'
      + '<div class="chat-messages" id="cross-chat-messages" style="margin-top:8px"></div>';
    container.appendChild(chatBar);

  } catch(e) {
    container.innerHTML = '<div class="session-empty" style="color:#e94560">Search failed: ' + e.message + '</div>';
  }
}

function escRegex(s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }

function toggleSearchSession(cb) {
  if (cb.checked) selectedSearchSessions.add(cb.dataset.sid);
  else selectedSearchSessions.delete(cb.dataset.sid);

  if (selectedSearchSessions.size > 3) {
    cb.checked = false;
    selectedSearchSessions.delete(cb.dataset.sid);
    toast('Maximum 3 sessions for cross-session chat');
  }
}

let crossChatHistory = [];
async function sendCrossChat() {
  const input = $('cross-chat-input');
  const msg = input.value.trim();
  if (!msg) return;
  if (selectedSearchSessions.size === 0) { toast('Select at least one session'); return; }
  input.value = '';

  const msgs = $('cross-chat-messages');
  msgs.innerHTML += '<div class="chat-msg user"><span class="role">You:</span>' + escHtml(msg) + '</div>';
  msgs.innerHTML += '<div class="chat-msg assistant" id="cross-chat-pending"><span class="role">Jarvis:</span> <em>Thinking...</em></div>';
  msgs.scrollTop = msgs.scrollHeight;

  try {
    const r = await fetch('/api/sessions/chat', {
      method: 'POST',
      headers: {'Content-Type':'application/json'},
      body: JSON.stringify({
        message: msg,
        session_ids: [...selectedSearchSessions],
        history: crossChatHistory
      })
    });
    const data = await r.json();
    const pending = document.getElementById('cross-chat-pending');
    if (data.reply) {
      crossChatHistory.push({ role: 'user', content: msg });
      crossChatHistory.push({ role: 'assistant', content: data.reply });
      if (pending) pending.innerHTML = '<span class="role">Jarvis:</span>' + escHtml(data.reply);
    } else {
      if (pending) pending.innerHTML = '<span class="role">Jarvis:</span> <em style="color:#e94560">' + escHtml(data.error || 'Error') + '</em>';
    }
  } catch(e) {
    const pending = document.getElementById('cross-chat-pending');
    if (pending) pending.innerHTML = '<span class="role">Jarvis:</span> <em style="color:#e94560">Request failed</em>';
  }
  msgs.scrollTop = msgs.scrollHeight;
}

// --- URL hash routing ---
function initHash() {
  const hash = location.hash.replace('#', '') || 'dashboard';
  if (hash.startsWith('sessions/')) {
    switchTab('sessions');
    const sid = hash.replace('sessions/', '');
    if (sid) setTimeout(() => openSession(sid), 500);
  } else if (hash === 'sessions') {
    switchTab('sessions');
  }
}
```

**Step 6: Call `initHash()` at page load**

Add at the end of the existing init block:

```javascript
loadConfig();
pollStatus();
setInterval(pollStatus, 5000);
connectTranscript();
initHash();
```

**Step 7: Build and verify**

Run: `cd jarvis && cargo build 2>&1`
Expected: Compiles. Open `http://localhost:8080` and verify:
- Two tabs visible (Dashboard / Sessions)
- Dashboard tab works exactly as before
- Sessions tab shows empty state or session list
- Clicking a session shows detail view
- Audio player works
- Download buttons work
- Chat input sends and receives responses
- Search works with highlighting
- Cross-session chat works with checkbox selection

**Step 8: Commit**

```
feat: add Sessions tab with history, playback, downloads, and AI chat
```

---

### Task 8: Polish and edge cases

**Files:**
- Modify: `jarvis/src/sessions.rs` — minor fixes if needed
- Modify: `jarvis/src/assets/index.html` — UI polish

**Step 1: Test all flows manually**

Checklist:
- [ ] Tab switching preserves state
- [ ] Empty sessions list shows friendly message
- [ ] Session list paginated correctly
- [ ] Transcript displays correctly
- [ ] Audio plays and seeks (Range headers working)
- [ ] Video plays for `.webm`, download-only for `.mkv`
- [ ] Download buttons serve correct files
- [ ] Single-session chat works with follow-ups
- [ ] Search finds matches case-insensitively
- [ ] Search shows context lines
- [ ] Cross-session chat limited to 3 sessions
- [ ] Mobile layout works (single column)
- [ ] URL hash routing works (`#sessions`, `#sessions/ID`)

**Step 2: Fix any issues found during testing**

**Step 3: Final commit**

```
fix: polish sessions UI edge cases
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | AppState + standalone LLM | `server.rs`, `main.rs`, `llm.rs` |
| 2 | Session scanner + list API | `sessions.rs` (new), `main.rs`, `server.rs` |
| 3 | Transcript endpoints | `sessions.rs` |
| 4 | Audio/video serving + Range | `sessions.rs` |
| 5 | Search endpoint | `sessions.rs` |
| 6 | Chat endpoints | `sessions.rs` |
| 7 | Full frontend | `index.html` |
| 8 | Polish + testing | all |

**Total: 8 tasks, ~8 commits**
