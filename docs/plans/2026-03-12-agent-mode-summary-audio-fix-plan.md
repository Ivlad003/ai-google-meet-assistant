# Agent Mode, Summary UI, and Audio Fix — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add name-only response mode toggle, conversation summary with copy-to-clipboard in web UI, and fix WAV files not playing after session.

**Architecture:** Three independent changes: (1) `response_mode` config + `watch` channel for runtime propagation + word-boundary name matching, (2) `/api/summary` endpoint with empty-transcript guard + reasoning model fix in `chat_once()` + UI panel, (3) cooperative cancellation of audio task via `watch<bool>` for clean WAV finalization.

**Tech Stack:** Rust (axum, tokio, hound, serde), HTML/CSS/JS (embedded in `src/assets/index.html`)

---

### Task 1: Add `ResponseMode` enum and config fields

**Files:**
- Modify: `jarvis/src/config.rs`
- Modify: `jarvis.config.example.json`

**Step 1: Add ResponseMode enum to config.rs**

Add after the `TranscriptionMode` enum (after line 66):

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseMode {
    Smart,
    NameOnly,
}
```

**Step 2: Add response_mode to ConfigFile struct**

Add after line 21 (`pub temperature: Option<f32>,`):

```rust
pub response_mode: Option<String>,
```

**Step 3: Add response_mode to Config struct**

Add after line 58 (`pub temperature: f32,`):

```rust
pub response_mode: ResponseMode,
```

**Step 4: Add response_mode mapping in `Config::from_file`**

Add after `temperature: cf.temperature.unwrap_or(0.7),` in the `Self { ... }` block:

```rust
response_mode: match cf.response_mode.as_deref() {
    Some("name_only") => ResponseMode::NameOnly,
    _ => ResponseMode::Smart,
},
```

**Step 5: Add `is_reasoning_model` helper function**

Add after the `Config` impl block (after `dirs_or_default` function, end of file):

```rust
/// Check if a model name is a reasoning model (no temperature, uses reasoning_effort).
pub fn is_reasoning_model(model: &str) -> bool {
    model.starts_with("gpt-5")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
}
```

**Step 6: Update example config**

In `jarvis.config.example.json`, add after `"temperature": 0.7,`:

```json
"response_mode": "smart",
```

**Step 7: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors (unused warnings OK at this stage)

**Step 8: Commit**

```bash
git add jarvis/src/config.rs jarvis.config.example.json
git commit -m "feat: add ResponseMode enum and response_mode config field"
```

---

### Task 2: Add `name_mentioned()`, `strip_bot_name()`, and `transcript_len()` to LlmAgent

**Files:**
- Modify: `jarvis/src/llm.rs`

**Step 1: Add word-boundary helper function**

Add before `impl LlmAgent` (before line 66):

```rust
/// Check if `needle` appears at a word boundary in `haystack`.
/// For ASCII: checks that chars before/after needle are non-alphanumeric.
/// For Cyrillic: checks that chars before/after are whitespace or punctuation.
fn contains_at_word_boundary(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    let mut start = 0;
    while let Some(pos) = h[start..].find(&n) {
        let abs_pos = start + pos;
        let end_pos = abs_pos + n.len();

        let before_ok = if abs_pos == 0 {
            true
        } else {
            h[..abs_pos]
                .chars()
                .last()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        };

        let after_ok = if end_pos >= h.len() {
            true
        } else {
            h[end_pos..]
                .chars()
                .next()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        };

        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + n.len().max(1);
    }
    false
}
```

**Step 2: Add `name_mentioned()` method**

Add to `impl LlmAgent` block, after `add_bot_response_to_transcript` method (after line ~125):

```rust
/// Check if the bot name is mentioned in the text using word-boundary matching.
/// Used in "name_only" response mode to skip LLM intent detection.
pub fn name_mentioned(&self, text: &str) -> bool {
    let variants = [
        self.bot_name.as_str(),
        "jarvis",
        "джарвіс",
        "джарвис",
        "джарвіз",
        "jarves",
        "ві джарвіс",
        "ай джарвіс",
        "preview jones",
    ];

    variants.iter().any(|v| contains_at_word_boundary(text, v))
}
```

**Step 3: Add `strip_bot_name()` method**

Add immediately after `name_mentioned`:

```rust
/// Remove the bot name variant from text and trim, giving the LLM a clean question.
/// Example: "Джарвіс, підсумуй зустріч" → "підсумуй зустріч"
pub fn strip_bot_name(&self, text: &str) -> String {
    let lower = text.to_lowercase();
    let variants = [
        self.bot_name.to_lowercase(),
        "jarvis".to_string(),
        "джарвіс".to_string(),
        "джарвис".to_string(),
        "джарвіз".to_string(),
        "jarves".to_string(),
        "ві джарвіс".to_string(),
        "ай джарвіс".to_string(),
        "preview jones".to_string(),
    ];

    let mut result = text.to_string();
    for variant in &variants {
        if let Some(pos) = lower.find(variant.as_str()) {
            let byte_end = pos + variant.len();
            // Remove the variant and clean up surrounding punctuation/whitespace
            result = format!("{}{}", &text[..pos], &text[byte_end..]);
            result = result.trim_matches(|c: char| c.is_whitespace() || c == ',').trim().to_string();
            break;
        }
    }
    if result.is_empty() {
        text.trim().to_string()
    } else {
        result
    }
}
```

**Step 4: Add `transcript_len()` method**

Add immediately after `strip_bot_name`:

```rust
/// Return the number of transcript entries (for empty-transcript guards).
pub fn transcript_len(&self) -> usize {
    self.transcript.lock().unwrap().len()
}
```

**Step 5: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors

**Step 6: Commit**

```bash
git add jarvis/src/llm.rs
git commit -m "feat: add name_mentioned(), strip_bot_name(), transcript_len() to LlmAgent"
```

---

### Task 3: Fix reasoning model detection in `chat_once()`

**Files:**
- Modify: `jarvis/src/llm.rs`

**Step 1: Update `respond()` to use the shared helper**

In `llm.rs`, replace lines 256-257:

```rust
// Reasoning models (gpt-5, o3, etc.) don't support temperature
let is_reasoning = self.model.starts_with("gpt-5") || self.model.starts_with("o1") || self.model.starts_with("o3") || self.model.starts_with("o4");
```

with:

```rust
let is_reasoning = crate::config::is_reasoning_model(&self.model);
```

**Step 2: Update `chat_once()` to handle reasoning models**

In `chat_once()` method (around line 354), replace the `ChatRequest` construction:

```rust
let req = ChatRequest {
    model: model.to_string(),
    messages: vec![ChatMessage {
        role: "user".to_string(),
        content: prompt.to_string(),
    }],
    temperature: temp,
    max_completion_tokens: max_tokens,
    reasoning_effort: reasoning_effort.map(|s| s.to_string()),
};
```

with:

```rust
let is_reasoning = crate::config::is_reasoning_model(model);
let req = ChatRequest {
    model: model.to_string(),
    messages: vec![ChatMessage {
        role: "user".to_string(),
        content: prompt.to_string(),
    }],
    temperature: if is_reasoning { None } else { temp },
    max_completion_tokens: if is_reasoning { max_tokens.max(1000) } else { max_tokens },
    reasoning_effort: if is_reasoning {
        Some(reasoning_effort.unwrap_or("low").to_string())
    } else {
        reasoning_effort.map(|s| s.to_string())
    },
};
```

**Step 3: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors

**Step 4: Commit**

```bash
git add jarvis/src/llm.rs
git commit -m "fix: use shared is_reasoning_model() in chat_once() to prevent API errors"
```

---

### Task 4: Add `watch` channel for response_mode + cooperative shutdown signal

**Files:**
- Modify: `jarvis/src/main.rs`
- Modify: `jarvis/src/server.rs`

**Step 1: Add `response_mode_tx` and `shutdown_tx` to AppState**

In `server.rs`, add to `AppState` struct (after `bot_process` field):

```rust
pub response_mode_tx: tokio::sync::watch::Sender<crate::config::ResponseMode>,
pub shutdown_tx: tokio::sync::watch::Sender<bool>,
```

**Step 2: Create watch channels in main.rs**

In `main.rs`, add after the `bot_process` creation (line ~134) and before `AppState` construction:

```rust
// Watch channels for runtime config propagation and cooperative shutdown
let (response_mode_tx, response_mode_rx) = tokio::sync::watch::channel(cfg.response_mode.clone());
let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
```

**Step 3: Add watch senders to AppState construction**

In `main.rs`, add to the `AppState { ... }` block (after `bot_process`):

```rust
response_mode_tx,
shutdown_tx: shutdown_tx.clone(),
```

**Step 4: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors (unused variable warnings OK)

**Step 5: Commit**

```bash
git add jarvis/src/main.rs jarvis/src/server.rs
git commit -m "feat: add watch channels for response_mode propagation and cooperative shutdown"
```

---

### Task 5: Wire response_mode + cooperative shutdown into audio loop

**Files:**
- Modify: `jarvis/src/main.rs`

**Step 1: Pass watch receivers into audio task**

In `main.rs`, add to the pre-spawn clones (around line ~164, near `let tools_list = ...`):

```rust
let mut response_mode_rx = response_mode_rx;
let mut shutdown_rx = shutdown_rx;
```

**Step 2: Capture JoinHandle from audio task spawn**

Change line ~166 from:

```rust
tokio::spawn(async move {
```

to:

```rust
let audio_task = tokio::spawn(async move {
```

**Step 3: Replace the `while let Some(samples) = audio_rx.recv().await` loop**

Replace the audio loop opening (line ~171):

```rust
while let Some(samples) = audio_rx.recv().await {
```

with:

```rust
loop {
    let samples = tokio::select! {
        s = audio_rx.recv() => match s {
            Some(samples) => samples,
            None => break,
        },
        _ = shutdown_rx.changed() => break,
    };
```

Everything inside the loop body stays the same.

**Step 4: Replace the intent detection block**

Replace the current intent detection (line ~221):

```rust
if let Some(question) = agent_clone.should_respond(&speaker_label, &seg.text).await {
```

with:

```rust
// Determine if bot should respond based on response_mode
let question = {
    let mode = response_mode_rx.borrow().clone();
    match mode {
        crate::config::ResponseMode::NameOnly => {
            if agent_clone.name_mentioned(&seg.text) {
                let cleaned = agent_clone.strip_bot_name(&seg.text);
                Some(if cleaned.is_empty() { seg.text.clone() } else { cleaned })
            } else {
                None
            }
        }
        crate::config::ResponseMode::Smart => {
            agent_clone.should_respond(&speaker_label, &seg.text).await
        }
    }
};

if let Some(question) = question {
```

Everything after `if let Some(question) = question {` stays unchanged.

**Step 5: Replace shutdown and WAV finalization**

Replace the shutdown block (from `ctrl_c().await` to end of main, lines ~362-385):

```rust
tracing::info!("Jarvis running. Press Ctrl+C to stop.");
tokio::signal::ctrl_c().await?;
tracing::info!("Shutting down...");

// Stop vexa-bot on shutdown
if let Ok(mut proc) = bot_process.lock() {
    let _ = proc.stop();
}

// Signal audio task to shut down cooperatively
let _ = shutdown_tx.send(true);
// Wait for audio task to exit (drops its Arc<wav_writer> clone)
let _ = audio_task.await;

// Finalize WAV file (write header with correct data length)
match Arc::try_unwrap(wav_writer) {
    Ok(writer) => {
        let writer = writer.into_inner();
        if let Err(e) = writer.finalize() {
            tracing::warn!("Failed to finalize WAV file: {}", e);
        } else {
            tracing::info!("Audio file finalized: {}", session_audio_path.display());
        }
    }
    Err(_) => {
        tracing::warn!("Could not finalize WAV file — other references still held");
    }
}

// Print session file paths to terminal
println!();
println!("=== Session Complete ===");
println!("Transcript: {}", session_transcript_path.display());
println!("Audio:      {}", session_audio_path.display());
println!("Logs:       {}", logs_dir.display());
println!("========================");

Ok(())
```

**Step 6: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors

**Step 7: Commit**

```bash
git add jarvis/src/main.rs
git commit -m "feat: wire response_mode watch + cooperative shutdown into audio loop

Fixes WAV playback by ensuring audio task drops wav_writer Arc
before finalization. Uses tokio::select! with shutdown signal
instead of abort() to avoid dropping resources mid-operation."
```

---

### Task 6: Add response_mode to web API config endpoints

**Files:**
- Modify: `jarvis/src/server.rs`

**Step 1: Add response_mode to ConfigResponse**

Add to `ConfigResponse` struct (after `openai_model: String,`):

```rust
response_mode: String,
```

**Step 2: Add response_mode to get_config handler**

In `get_config`, add to the `Json(ConfigResponse { ... })` block:

```rust
response_mode: match cfg.response_mode {
    crate::config::ResponseMode::NameOnly => "name_only".to_string(),
    crate::config::ResponseMode::Smart => "smart".to_string(),
},
```

**Step 3: Add response_mode to ConfigUpdate**

Add to `ConfigUpdate` struct (after `openai_model: Option<String>,`):

```rust
response_mode: Option<String>,
```

**Step 4: Handle response_mode in update_config**

In `update_config`, add after the openai_model update block:

```rust
if let Some(mode) = update.response_mode {
    let new_mode = match mode.as_str() {
        "name_only" => crate::config::ResponseMode::NameOnly,
        _ => crate::config::ResponseMode::Smart,
    };
    cfg.response_mode = new_mode.clone();
    // Propagate to audio task via watch channel
    let _ = state.response_mode_tx.send(new_mode);
}
```

**Step 5: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors

**Step 6: Commit**

```bash
git add jarvis/src/server.rs
git commit -m "feat: add response_mode to config API endpoints with watch propagation"
```

---

### Task 7: Add `/api/summary` endpoint

**Files:**
- Modify: `jarvis/src/server.rs`

**Step 1: Add summary route to router**

In the `router` function, add before `.with_state(state)`:

```rust
.route("/api/summary", get(get_summary))
```

**Step 2: Add handler function**

Add after the `leave_meeting` function:

```rust
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
```

**Step 3: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors

**Step 4: Commit**

```bash
git add jarvis/src/server.rs
git commit -m "feat: add /api/summary endpoint with empty-transcript guard"
```

---

### Task 8: Update Web UI — response mode toggle + summary panel

**Files:**
- Modify: `jarvis/src/assets/index.html`

**Step 1: Add response mode select to Settings panel**

In `index.html`, add after the OpenAI Model `<div class="field">` block (after the `</select></div>` for model, before `<div class="controls">`):

```html
<div class="field">
  <label>Response Mode</label>
  <select id="response-mode">
    <option value="smart">Smart (LLM intent detection)</option>
    <option value="name_only">Name Only (respond when name said)</option>
  </select>
</div>
```

**Step 2: Add summary panel after transcript panel**

After the transcript panel closing `</div>` (the one that closes `<div class="panel transcript-panel">`), add:

```html
<div class="panel transcript-panel">
  <h2>Meeting Summary</h2>
  <div class="controls" style="margin-bottom: 12px; margin-top: 0;">
    <button class="btn btn-primary" id="summarize-btn" onclick="getSummary()">Summarize</button>
    <button class="btn btn-success" id="copy-btn" onclick="copySummary()" style="display:none;">Copy to Clipboard</button>
  </div>
  <div id="summary-content" style="background: #0d1117; border-radius: 4px; padding: 12px; min-height: 60px; font-size: 14px; line-height: 1.6; white-space: pre-wrap; color: #aaa;">
    Click Summarize to generate a summary of recent conversation.
  </div>
</div>
```

**Step 3: Update `loadConfig` to load response_mode**

In the `loadConfig` function, add after `$('model').value = c.openai_model || 'gpt-4o';`:

```javascript
$('response-mode').value = c.response_mode || 'smart';
```

**Step 4: Update `saveConfig` to send response_mode**

In the `saveConfig` function, add to the `body` object (after `openai_model`):

```javascript
response_mode: $('response-mode').value,
```

**Step 5: Add `getSummary` and `copySummary` functions**

Add before the closing `</script>` tag:

```javascript
async function getSummary() {
  const btn = $('summarize-btn');
  btn.textContent = 'Generating...';
  btn.disabled = true;
  try {
    const r = await fetch('/api/summary');
    const res = await r.json();
    const el = $('summary-content');
    if (res.ok) {
      el.style.color = '#e0e0e0';
      el.textContent = res.summary;
      $('copy-btn').style.display = '';
    } else {
      el.style.color = '#e94560';
      el.textContent = res.message;
    }
  } catch(e) {
    $('summary-content').style.color = '#e94560';
    $('summary-content').textContent = 'Failed: ' + e.message;
  } finally {
    btn.textContent = 'Summarize';
    btn.disabled = false;
  }
}

function copySummary() {
  const text = $('summary-content').textContent;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(() => toast('Copied to clipboard!')).catch(() => fallbackCopy(text));
  } else {
    fallbackCopy(text);
  }
}

function fallbackCopy(text) {
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.style.position = 'fixed';
  ta.style.left = '-9999px';
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand('copy');
    toast('Copied to clipboard!');
  } catch(e) {
    toast('Copy failed');
  }
  document.body.removeChild(ta);
}
```

**Step 6: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors

**Step 7: Commit**

```bash
git add jarvis/src/assets/index.html
git commit -m "feat: add response mode toggle and summary panel with clipboard to web UI"
```

---

### Task 9: Update documentation

**Files:**
- Modify: `docs/technical-details.md`

**Step 1: Add response_mode to configuration table**

In `docs/technical-details.md`, add to the configuration reference table after the `temperature` row:

```markdown
| `response_mode` | No | `smart` | `smart` (LLM intent detection) or `name_only` (keyword match on bot name) |
```

**Step 2: Commit**

```bash
git add docs/technical-details.md
git commit -m "docs: add response_mode to configuration reference"
```

---

### Task 10: Final build and smoke test

**Step 1: Clean build**

Run: `cd jarvis && cargo build 2>&1 | tail -10`
Expected: `Finished` with no errors and no warnings related to our changes

**Step 2: Quick startup check**

Run: `cd jarvis && timeout 3 ./target/debug/jarvis --config ../jarvis.config.example.json 2>&1 || true`
Expected: Output includes "Jarvis v0.1.0 starting..." (will exit on fake API key, but confirms startup path works)

**Step 3: Final commit if any fixups needed**
