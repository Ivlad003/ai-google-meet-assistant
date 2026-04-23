use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    body::Body,
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::server::AppState;

/// Max bytes to allocate for a single Range request (8MB)
const MAX_RANGE_BYTES: u64 = 8 * 1024 * 1024;

/// Max chat message length (characters)
const MAX_CHAT_MESSAGE_LEN: usize = 10_000;

/// Max chat history entries
const MAX_CHAT_HISTORY: usize = 50;

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
    pub live: bool,
}

#[derive(Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionInfo>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Deserialize)]
pub struct ListParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Validate that a session ID matches the expected `YYYY-MM-DD_HHMMSS` format (17 chars).
/// This is important for path traversal prevention.
pub fn is_valid_session_id(id: &str) -> bool {
    if id.len() != 17 {
        return false;
    }
    let bytes = id.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'_' {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        if i == 4 || i == 7 || i == 10 {
            continue;
        }
        if !b.is_ascii_digit() {
            return false;
        }
    }
    true
}

/// Get the sessions directory from AppState.
pub fn sessions_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("sessions")
}

/// Format session ID to readable date: YYYY-MM-DD_HHMMSS -> YYYY-MM-DD HH:MM:SS
fn format_session_date(id: &str) -> String {
    format!(
        "{} {}:{}:{}",
        &id[..10],
        &id[11..13],
        &id[13..15],
        &id[15..17]
    )
}

/// Truncate text to approximately max_bytes, breaking at the last newline
/// before the limit. Respects UTF-8 char boundaries.
fn truncate_text(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    match text[..end].rfind('\n') {
        Some(pos) => &text[..pos],
        None => &text[..end],
    }
}

/// Scan the sessions directory and return a list of SessionInfo, newest first.
/// Uses spawn_blocking to avoid blocking the async runtime.
fn scan_sessions_sync(sessions_dir: &StdPath) -> Vec<SessionInfo> {
    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let now = std::time::SystemTime::now();
    // Track which sessions have recently modified files (likely still being written to)
    let mut groups: HashMap<String, Vec<(String, u64)>> = HashMap::new();
    let mut recently_modified: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if file_name.len() < 18 || file_name.as_bytes()[17] != b'.' {
            continue;
        }
        let prefix = &file_name[..17];
        if !is_valid_session_id(prefix) {
            continue;
        }

        // Track if any file in this session was modified recently (live session)
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            if let Ok(elapsed) = now.duration_since(modified) {
                if elapsed.as_secs() < 60 {
                    recently_modified.insert(prefix.to_string());
                }
            }
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_string(),
            None => continue,
        };

        groups
            .entry(prefix.to_string())
            .or_default()
            .push((ext, size));
    }

    let mut sessions: Vec<SessionInfo> = groups
        .into_iter()
        .map(|(id, files)| {
            let mut has_audio = false;
            let mut has_video = false;
            let mut has_transcript = false;
            let mut audio_size: Option<u64> = None;
            let mut video_size: Option<u64> = None;
            let mut transcript_size: Option<u64> = None;
            let mut video_format: Option<String> = None;

            for (ext, size) in &files {
                match ext.as_str() {
                    "wav" => { has_audio = true; audio_size = Some(*size); }
                    "txt" => { has_transcript = true; transcript_size = Some(*size); }
                    "webm" | "mkv" => { has_video = true; video_size = Some(*size); video_format = Some(ext.clone()); }
                    _ => {}
                }
            }

            let preview = if has_transcript {
                read_preview(sessions_dir, &id)
            } else {
                String::new()
            };

            let date = format_session_date(&id);

            let live = recently_modified.contains(&id);
            SessionInfo {
                id, date, preview, has_audio, has_video, has_transcript,
                audio_size, video_size, transcript_size, video_format, live,
            }
        })
        .collect();

    sessions.sort_by(|a, b| b.id.cmp(&a.id));
    sessions
}

/// Read the first non-empty line from a transcript file, truncated to 120 chars.
fn read_preview(sessions_dir: &StdPath, id: &str) -> String {
    use std::io::BufRead;

    let path = sessions_dir.join(format!("{}.txt", id));
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };

    let reader = std::io::BufReader::new(file);
    for line in reader.lines().flatten() {
        let trimmed = line.trim().to_string();
        if !trimmed.is_empty() {
            if trimmed.len() > 120 {
                return format!("{}...", &trimmed[..120]);
            }
            return trimmed;
        }
    }

    String::new()
}

/// Collect valid session .txt file stems from directory (sync, for use in spawn_blocking).
fn collect_txt_stems_sync(dir: &StdPath) -> Vec<String> {
    let mut entries = Vec::new();
    let read_dir = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return entries,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if is_valid_session_id(stem) {
                entries.push(stem.to_string());
            }
        }
    }
    entries.sort_by(|a, b| b.cmp(a));
    entries
}

// ─── Shared chat helper ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ChatHistoryMessage {
    role: String,
    content: String,
}

/// Shared chat logic: builds messages, calls LLM, handles tool execution.
/// All chat endpoints delegate to this after constructing the system prompt.
async fn do_chat(
    state: &AppState,
    system: &str,
    message: String,
    history: &[ChatHistoryMessage],
) -> Json<serde_json::Value> {
    // Validate input sizes
    if message.len() > MAX_CHAT_MESSAGE_LEN {
        return Json(serde_json::json!({ "error": "Message too long (max 10000 chars)" }));
    }

    let model = state.config.read().await.openai_model.clone();

    let mut messages: Vec<(String, String)> = history
        .iter()
        .take(MAX_CHAT_HISTORY)
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    messages.push(("user".to_string(), message));

    match crate::llm::chat_with_context(
        &state.openai_key,
        &state.http_client,
        &model,
        system,
        messages,
        0.7,
        1000,
    )
    .await
    {
        Ok(reply) => {
            let final_reply = maybe_execute_tool(&reply, state).await;
            Json(serde_json::json!({ "reply": final_reply }))
        }
        Err(e) => {
            tracing::error!("Chat LLM error: {}", e);
            Json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

/// Build tools section for chat system prompts.
async fn tools_prompt_for_chat(state: &AppState) -> String {
    let cfg = state.config.read().await;
    crate::tools::tools_prompt(&cfg.tools)
}

/// Check if an LLM reply contains a tool call, execute it, and return combined reply.
async fn maybe_execute_tool(reply: &str, state: &AppState) -> String {
    let (tool_name, params) = match crate::tools::parse_tool_call(reply) {
        Some(t) => t,
        None => return reply.to_string(),
    };

    let tools = state.config.read().await.tools.clone();
    let tool = match tools.iter().find(|t| t.name == tool_name) {
        Some(t) => t.clone(),
        None => return format!("{}\n\n(Tool '{}' not found)", reply, tool_name),
    };

    tracing::info!("[chat-tools] executing: {} with {:?}", tool_name, params);
    let result = crate::tools::execute_tool(&tool, &params, &state.http_client).await;

    if result.success {
        format!("Tool `{}` executed successfully:\n{}", tool_name, result.output)
    } else {
        format!("Tool `{}` failed:\n{}", tool_name, result.output)
    }
}

// ─── List sessions ───────────────────────────────────────────────────────────

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListParams>,
) -> Json<SessionListResponse> {
    let dir = sessions_dir(&state);
    let all = tokio::task::spawn_blocking(move || scan_sessions_sync(&dir))
        .await
        .unwrap_or_default();
    let total = all.len();

    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);
    let sessions: Vec<SessionInfo> = all.into_iter().skip(offset).take(limit).collect();

    Json(SessionListResponse { sessions, total, limit, offset })
}

// ─── Transcript endpoints ────────────────────────────────────────────────────

async fn get_transcript(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_valid_session_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = sessions_dir(&state).join(format!("{}.txt", id));
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "text": text })))
}

async fn download_transcript(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_valid_session_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = sessions_dir(&state).join(format!("{}.txt", id));
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    if let Ok(val) = HeaderValue::from_str(&format!("attachment; filename=\"{}.txt\"", id)) {
        headers.insert(header::CONTENT_DISPOSITION, val);
    }
    Ok((headers, text))
}

// ─── Audio/video serving with Range support ──────────────────────────────────

async fn serve_file(
    state: &AppState,
    id: &str,
    extensions: &[&str],
    content_types: &[&str],
    headers: &HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    if !is_valid_session_id(id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let dir = sessions_dir(state);
    let mut file_path = None;
    let mut content_type = "";
    for (i, ext) in extensions.iter().enumerate() {
        let candidate = dir.join(format!("{}.{}", id, ext));
        if candidate.exists() {
            file_path = Some(candidate);
            content_type = content_types[i];
            break;
        }
    }

    let file_path = file_path.ok_or(StatusCode::NOT_FOUND)?;
    let metadata = tokio::fs::metadata(&file_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let file_size = metadata.len();

    let ct_val = HeaderValue::from_str(content_type).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(range_header) = headers.get(header::RANGE) {
        let range_str = range_header.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;

        let range_str = range_str
            .strip_prefix("bytes=")
            .ok_or(StatusCode::RANGE_NOT_SATISFIABLE)?;

        let parts: Vec<&str> = range_str.splitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(StatusCode::RANGE_NOT_SATISFIABLE);
        }

        let start: u64 = if parts[0].is_empty() {
            let suffix: u64 = parts[1].parse().map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;
            file_size.saturating_sub(suffix)
        } else {
            parts[0].parse().map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?
        };

        let end: u64 = if parts[1].is_empty() || parts[0].is_empty() {
            file_size - 1
        } else {
            parts[1].parse().map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?
        };

        if start > end || start >= file_size {
            return Err(StatusCode::RANGE_NOT_SATISFIABLE);
        }
        let end = end.min(file_size - 1);
        let length = end - start + 1;

        // Cap allocation to prevent OOM from malicious Range headers
        if length > MAX_RANGE_BYTES {
            return Err(StatusCode::RANGE_NOT_SATISFIABLE);
        }

        let mut file = tokio::fs::File::open(&file_path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut buf = vec![0u8; length as usize];
        file.read_exact(&mut buf)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let cr_val = HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, file_size))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let cl_val = HeaderValue::from_str(&length.to_string())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok((
            StatusCode::PARTIAL_CONTENT,
            [
                (header::CONTENT_TYPE, ct_val),
                (header::CONTENT_LENGTH, cl_val),
                (header::CONTENT_RANGE, cr_val),
                (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            ],
            Body::from(buf),
        )
            .into_response())
    } else {
        // Full file — read into memory (fine for typical session files <200MB)
        let data = tokio::fs::read(&file_path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let cl_val = HeaderValue::from_str(&file_size.to_string())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, ct_val),
                (header::CONTENT_LENGTH, cl_val),
                (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            ],
            Body::from(data),
        )
            .into_response())
    }
}

async fn get_audio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    serve_file(&state, &id, &["wav"], &["audio/wav"], &headers).await
}

async fn get_video(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    serve_file(&state, &id, &["webm", "mkv"], &["video/webm", "video/x-matroska"], &headers).await
}

// ─── Search endpoint ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    max_results: Option<usize>,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchSessionResult>,
}

#[derive(Serialize)]
struct SearchSessionResult {
    session_id: String,
    session_date: String,
    matches: Vec<SearchMatch>,
}

#[derive(Serialize)]
struct SearchMatch {
    line: usize,
    text: String,
    context_before: Option<String>,
    context_after: Option<String>,
}

async fn search_sessions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let max_results = req.max_results.unwrap_or(100).min(500);
    let query = req.query.to_lowercase();

    if query.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let dir = sessions_dir(&state);
    let dir_clone = dir.clone();
    let entries = tokio::task::spawn_blocking(move || collect_txt_stems_sync(&dir_clone))
        .await
        .unwrap_or_default();

    let mut results: Vec<SearchSessionResult> = Vec::new();
    let mut total_matches = 0;

    'outer: for session_id in &entries {
        let path = dir.join(format!("{}.txt", session_id));
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut session_matches: Vec<SearchMatch> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            if line.to_lowercase().contains(&query) {
                session_matches.push(SearchMatch {
                    line: i + 1,
                    text: line.to_string(),
                    context_before: if i > 0 { Some(lines[i - 1].to_string()) } else { None },
                    context_after: if i + 1 < lines.len() { Some(lines[i + 1].to_string()) } else { None },
                });

                total_matches += 1;
                if total_matches >= max_results {
                    break;
                }
            }
        }

        if !session_matches.is_empty() {
            results.push(SearchSessionResult {
                session_id: session_id.clone(),
                session_date: format_session_date(session_id),
                matches: session_matches,
            });
        }

        if total_matches >= max_results {
            break 'outer;
        }
    }

    Ok(Json(SearchResponse { results }))
}

// ─── Chat endpoints ────────────────────────────────────────────────────────

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

async fn session_chat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SessionChatRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_valid_session_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let path = sessions_dir(&state).join(format!("{}.txt", id));
    let transcript = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let tools_section = tools_prompt_for_chat(&state).await;
    let system = format!(
        "You are a helpful assistant analyzing a meeting transcript. \
         Answer questions based on the following transcript:\n\n{}{}",
        truncate_text(&transcript, 12000),
        tools_section
    );

    Ok(do_chat(&state, &system, req.message, &req.history).await)
}

async fn cross_session_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CrossSessionChatRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if req.session_ids.is_empty() || req.session_ids.len() > 3 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let dir = sessions_dir(&state);
    let mut combined = String::new();

    for id in &req.session_ids {
        if !is_valid_session_id(id) {
            return Err(StatusCode::BAD_REQUEST);
        }
        let path = dir.join(format!("{}.txt", id));
        let transcript = tokio::fs::read_to_string(&path)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;

        if !combined.is_empty() { combined.push_str("\n\n"); }
        combined.push_str(&format!("=== SESSION: {} ===\n{}", format_session_date(id), truncate_text(&transcript, 4000)));
    }

    let tools_section = tools_prompt_for_chat(&state).await;
    let system = format!(
        "You are a helpful assistant analyzing multiple meeting transcripts. \
         Answer questions based on the following sessions:\n\n{}{}",
        combined, tools_section
    );

    Ok(do_chat(&state, &system, req.message, &req.history).await)
}

// ─── Global chat endpoint ───────────────────────────────────────────────────

async fn global_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SessionChatRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let dir = sessions_dir(&state);
    let dir_clone = dir.clone();
    let txt_files = tokio::task::spawn_blocking(move || collect_txt_stems_sync(&dir_clone))
        .await
        .unwrap_or_default();

    let max_per_session = 2000usize;
    let max_sessions = 10usize;
    let mut combined = String::new();

    for (i, session_id) in txt_files.iter().take(max_sessions).enumerate() {
        let path = dir.join(format!("{}.txt", session_id));
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            if i > 0 { combined.push_str("\n\n"); }
            combined.push_str(&format!("=== SESSION: {} ===\n{}", format_session_date(session_id), truncate_text(&text, max_per_session)));
        }
    }

    if combined.is_empty() {
        return Ok(Json(serde_json::json!({ "error": "No sessions available" })));
    }

    let tools_section = tools_prompt_for_chat(&state).await;
    let system = format!(
        "You are a helpful assistant with access to all meeting transcripts from this workspace.\n\
         Answer questions based on the meeting history below. Be concise and accurate.\n\
         Reference which session/date information comes from when relevant.\n\
         If the answer isn't in the transcripts, say so.\n\n\
         MEETING HISTORY ({} most recent sessions):\n{}{}",
        txt_files.len().min(max_sessions),
        combined,
        tools_section
    );

    Ok(do_chat(&state, &system, req.message, &req.history).await)
}

// ─── Delete session ─────────────────────────────────────────────────────────

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_valid_session_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let dir = sessions_dir(&state);
    let mut deleted = false;
    for ext in &["txt", "wav", "webm", "mkv"] {
        let path = dir.join(format!("{}.{}", id, ext));
        if tokio::fs::remove_file(&path).await.is_ok() {
            deleted = true;
        }
    }
    if deleted {
        Ok(Json(serde_json::json!({"ok": true})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn router(_state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/search", post(search_sessions))
        .route("/api/sessions/chat", post(cross_session_chat))
        .route("/api/sessions/chat/global", post(global_chat))
        .route("/api/sessions/:id/transcript", get(get_transcript))
        .route("/api/sessions/:id/transcript/download", get(download_transcript))
        .route("/api/sessions/:id/audio", get(get_audio))
        .route("/api/sessions/:id/video", get(get_video))
        .route("/api/sessions/:id/chat", post(session_chat))
        .route("/api/sessions/:id", delete(delete_session))
}
