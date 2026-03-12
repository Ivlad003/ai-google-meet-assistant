# Agent Mode, Summary UI, and Audio Fix — Design

Date: 2026-03-12

## 1. Agent Mode (Name-Only Response Mode)

### Config

New field `response_mode` in `jarvis.config.json`:
- `"smart"` (default) — current LLM-based intent detection with follow-ups, phonetic matching
- `"name_only"` — keyword match on bot name only, no LLM intent call

Add to `ConfigFile` (optional string) and `Config` (enum `ResponseMode { Smart, NameOnly }`).
Invalid values in the config file fall back to `Smart` (same pattern as `transcription_mode`).

### Runtime Propagation

The audio processing task cannot access `AppState.config` RwLock (it's spawned with pre-cloned values).
Use a `tokio::sync::watch` channel to propagate `response_mode` changes to the audio loop:

1. Create `watch::channel(ResponseMode::Smart)` in main, seeded from initial config
2. Pass `watch::Receiver` into the audio processing task
3. In `update_config` handler, send new value through `watch::Sender` (stored in `AppState`)
4. Audio loop calls `rx.borrow()` each iteration to read current mode — zero-cost when unchanged

### Name Matching Logic

New method `fn name_mentioned(&self, text: &str) -> bool` on `LlmAgent`:
- Lowercases text, checks against: exact bot name, "jarvis", Ukrainian variants ("джарвіс", "джарвис", "джарвіз"), common misheard variants ("jarves", "preview jones")
- Uses **word-boundary matching**, not substring — variant must be preceded by start-of-string or non-alphanumeric char, and followed by end-of-string or non-alphanumeric char. This prevents false positives like "Jarvis had a good idea" where the name appears in a non-addressing context. Implementation: `regex::Regex` with pattern `\b{variant}\b` for ASCII variants; for Cyrillic variants, check that surrounding characters are whitespace/punctuation (since `\b` doesn't work reliably on Unicode word boundaries).

New method `fn strip_bot_name(&self, text: &str) -> String` on `LlmAgent`:
- Removes the matched bot name variant from the text and trims, so the LLM gets a clean question
- Example: "Джарвіс, підсумуй зустріч" → "підсумуй зустріч"

### Audio Loop Change (main.rs)

Read `response_mode` from `watch::Receiver` each iteration (not from RwLock).
- `NameOnly` → call `name_mentioned()`, strip bot name, use cleaned text as question
- `Smart` → existing `should_respond()` flow

### Web UI

Toggle switch in Settings panel: "Response Mode" with Smart / Name Only options.
Calls `POST /api/config` with new `response_mode` field.

### API Changes

- Add `response_mode: String` to `ConfigResponse` — so the UI can display current mode on page load
- Add `response_mode: Option<String>` to `ConfigUpdate` — so the UI can change it
- `update_config` handler: update `RwLock<Config>` AND send through `watch::Sender`

## 2. Conversation Summary in Web UI

### Backend

New endpoint `GET /api/summary` in `server.rs`:
- **Guard:** Check transcript length first (via new `agent.transcript_len()` method). If empty, return `{ "ok": false, "message": "No transcript available yet" }` without calling the LLM.
- Otherwise calls existing `agent.summary()` method
- Returns `{ "ok": true, "summary": "..." }` or `{ "ok": false, "message": "..." }`

### Reasoning Model Compatibility

`agent.summary()` calls `chat_once()` which passes `temperature` directly. If `self.model` is a reasoning model (gpt-5*, o1*, o3*, o4*), the API rejects `temperature`.
Fix: add reasoning model detection to `chat_once()` — if model matches a reasoning prefix, set `temperature: None` and add `reasoning_effort`. Same logic already in `respond()`, extract to a helper `fn is_reasoning_model(model: &str) -> bool`.

### Transcript Length Limitation

`LlmAgent.transcript` is capped at 50 entries. For long meetings, the summary only covers recent conversation. This is acceptable for v1 — the summary is "recent meeting summary" not "full meeting summary". Document this in the UI empty state text: "Click Summarize to generate a summary of recent conversation".

### Frontend

New full-width panel below transcript: "Meeting Summary"
- Read-only div for summary text
- "Summarize" button — calls endpoint, shows loading spinner
- "Copy" button — `navigator.clipboard.writeText()` with fallback (`document.execCommand('copy')` via hidden textarea) for non-secure contexts (LAN IP access)
- Empty state: "Click Summarize to generate a summary of recent conversation"

No auto-generation, no periodic calls.

## 3. WAV Audio Playback Fix

### Root Cause

`Arc::try_unwrap(wav_writer)` in main.rs fails silently because the audio processing task still holds `wav_writer_clone`. Without `finalize()`, WAV header has incorrect data length — players reject the file.

### Fix

Use **cooperative cancellation** instead of `abort()` to avoid dropping resources mid-operation:

1. Create a `tokio_util::sync::CancellationToken` (or `tokio::sync::watch<bool>`)
2. Pass it into the audio processing task
3. Audio loop uses `tokio::select!` between `audio_rx.recv()` and `cancel_token.cancelled()` — on cancellation, breaks out of the loop cleanly
4. After `ctrl_c().await`, trigger cancellation and `await` the JoinHandle (task exits its loop, drops `wav_writer_clone` normally)
5. `Arc::try_unwrap` then succeeds, `finalize()` writes correct WAV header
6. Add `warn!` log if `try_unwrap` still fails (instead of silently ignoring)

This avoids aborting while a `MutexGuard` on `wav_writer_clone` is held, which could cause issues with drop ordering.

## Files to Modify

- `jarvis/src/config.rs` — add `response_mode` field + `ResponseMode` enum + `is_reasoning_model()` helper
- `jarvis/src/llm.rs` — add `name_mentioned()`, `strip_bot_name()`, `transcript_len()` methods; fix `chat_once()` reasoning model detection
- `jarvis/src/main.rs` — `watch` channel for response mode, `CancellationToken` for graceful shutdown, audio loop changes
- `jarvis/src/server.rs` — add `/api/summary` endpoint (with empty transcript guard), add `response_mode` to `ConfigResponse`/`ConfigUpdate`, store `watch::Sender` in `AppState`
- `jarvis/src/assets/index.html` — response mode toggle (reads initial state from GET), summary panel with copy button (with clipboard fallback)
- `jarvis.config.example.json` — document `response_mode` field
