# Video Recording Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Record the meeting page at 720p using Playwright's built-in `recordVideo`, remux to MKV via ffmpeg (or keep WebM as fallback), controlled by `record_video` config flag.

**Architecture:** Playwright captures the browser page as WebM (VP8) during the meeting. On graceful shutdown, Jarvis sends a `shutdown` command via bridge WebSocket; vexa-bot closes page/context (finalizing video), runs ffmpeg remux to MKV, sends `video_ready` event, then exits. Jarvis logs the final video path.

**Tech Stack:** Playwright `recordVideo`, ffmpeg (optional, host-installed), Rust (config/process/bridge changes), TypeScript (browser context + shutdown handler)

**Design doc:** `docs/plans/2026-03-12-video-recording-design.md`

---

### Task 1: Add `record_video` to Rust config

**Files:**
- Modify: `jarvis/src/config.rs`

**Step 1: Add field to ConfigFile**

In `ConfigFile` struct, add after `response_mode`:
```rust
pub record_video: Option<bool>,
```

**Step 2: Add field to Config struct**

In `Config` struct, add after `tools`:
```rust
pub record_video: bool,
```

**Step 3: Wire it in `Config::from_file()`**

In the `Self { ... }` block at the end, add:
```rust
record_video: cf.record_video.unwrap_or(false),
```

**Step 4: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | head -20`
Expected: builds successfully (unused field warnings are ok for now)

**Step 5: Commit**

```bash
git add jarvis/src/config.rs
git commit -m "feat: add record_video config field (default false)"
```

---

### Task 2: Add `record_video` to config API

**Files:**
- Modify: `jarvis/src/server.rs`

**Step 1: Add to ConfigResponse**

In `ConfigResponse` struct (line ~59), add:
```rust
record_video: bool,
```

In `get_config()`, add to the `ConfigResponse { ... }` block:
```rust
record_video: cfg.record_video,
```

**Step 2: Add to ConfigUpdate**

In `ConfigUpdate` struct (line ~81), add:
```rust
record_video: Option<bool>,
```

In `update_config()`, add before the final `Json(ConfigUpdateResponse { ... })`:
```rust
if let Some(record_video) = update.record_video {
    cfg.record_video = record_video;
}
```

**Step 3: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | head -20`
Expected: builds successfully

**Step 4: Commit**

```bash
git add jarvis/src/server.rs
git commit -m "feat: expose record_video in config API"
```

---

### Task 3: Add config example entry

**Files:**
- Modify: `jarvis.config.example.json`

**Step 1: Add record_video field**

Add `"record_video": false` after `"response_mode": "smart"` line.

**Step 2: Commit**

```bash
git add jarvis.config.example.json
git commit -m "docs: add record_video to config example"
```

---

### Task 4: Add `Shutdown` command and `video_ready` event to bridge protocol

**Files:**
- Modify: `jarvis/src/bot_bridge.rs`

**Step 1: Add Shutdown variant to CoreMessage**

In the `CoreMessage` enum, add:
```rust
#[serde(rename = "shutdown")]
Shutdown,
```

**Step 2: Add video_ready_path to BridgeState**

Add a new field to `BridgeState`:
```rust
/// Path to finalized video file, set by vexa-bot on shutdown
pub video_ready_path: Mutex<Option<String>>,
```

Initialize in `BridgeState::new()`:
```rust
video_ready_path: Mutex::new(None),
```

**Step 3: Handle video_ready event in handle_socket**

In the `BotMessage::Event` match arm (after the speaker_activity block, before `let _ = state.event_tx.send(...)`), add:
```rust
if event == "video_ready" {
    if let Some(path) = data.get("path").and_then(|v| v.as_str()) {
        *state.video_ready_path.lock().await = Some(path.to_string());
        tracing::info!("[bridge] video ready: {}", path);
    }
}
```

**Step 4: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | head -20`
Expected: builds successfully

**Step 5: Commit**

```bash
git add jarvis/src/bot_bridge.rs
git commit -m "feat: add Shutdown command and video_ready event to bridge protocol"
```

---

### Task 5: Add video env vars to process.rs

**Files:**
- Modify: `jarvis/src/process.rs`

**Step 1: Extend start() signature**

Add three new parameters to `VexaBotProcess::start()`:
```rust
pub fn start(
    &mut self,
    node_path: &PathBuf,
    vexa_bot_dir: &PathBuf,
    bridge_url: &str,
    meet_url: &str,
    bot_name: &str,
    record_video: bool,
    video_output_path: &str,
    ffmpeg_available: bool,
) -> anyhow::Result<()> {
```

**Step 2: Pass env vars to child process**

After the existing `.env("HEADLESS", &headless)` line, add:
```rust
.env("RECORD_VIDEO", if record_video { "true" } else { "false" })
.env("VIDEO_OUTPUT_PATH", video_output_path)
.env("FFMPEG_AVAILABLE", if ffmpeg_available { "true" } else { "false" })
```

**Step 3: Build — expect errors**

Run: `cd jarvis && cargo build 2>&1 | head -30`
Expected: compile errors in `main.rs` and `server.rs` because callers of `start()` pass fewer args. This is expected — we'll fix callers in the next task.

**Step 4: Commit (with build errors acknowledged)**

```bash
git add jarvis/src/process.rs
git commit -m "feat: pass video recording env vars to vexa-bot process"
```

---

### Task 6: Wire video recording into main.rs

**Files:**
- Modify: `jarvis/src/main.rs`

**Step 1: Add ffmpeg detection helper**

Add this function before `main()` (after `repair_wav_files`):
```rust
/// Check if ffmpeg is available on the system PATH.
fn has_ffmpeg() -> bool {
    let cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    std::process::Command::new(cmd)
        .arg("ffmpeg")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
```

**Step 2: Compute video path and check ffmpeg after session setup**

After the line `tracing::info!("Session audio: {}", session_audio_path.display());` (around line 187), add:
```rust
// Video recording setup
let ffmpeg_available = if cfg.record_video { has_ffmpeg() } else { false };
let session_video_path = if cfg.record_video {
    let ext = if ffmpeg_available { "mkv" } else { "webm" };
    let path = sessions_dir.join(format!("{}.{}", session_ts, ext));
    tracing::info!("Session video: {}", path.display());
    if !ffmpeg_available {
        tracing::warn!("record_video enabled but ffmpeg not found; video will be saved as .webm");
    }
    Some(path)
} else {
    None
};
```

**Step 3: Update the auto-start proc.start() call**

Find the `proc.start(` call around line 445 and update to pass new args:
```rust
if let Err(e) = proc.start(
    &node_path,
    &vexa_bot_dir,
    &bridge_url,
    meet_url,
    &cfg.bot_name,
    cfg.record_video,
    session_video_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default().as_str(),
    ffmpeg_available,
) {
```

**Step 4: Update shutdown sequence — graceful stop**

Replace the current shutdown block:
```rust
// Stop vexa-bot on shutdown
if let Ok(mut proc) = bot_process.lock() {
    let _ = proc.stop();
}
```

With:
```rust
// Graceful shutdown: send shutdown command, wait for vexa-bot to exit
if bridge_state.connection_count.load(std::sync::atomic::Ordering::Relaxed) > 0 {
    tracing::info!("Sending shutdown command to vexa-bot...");
    let _ = bridge_state.command_tx.send(bot_bridge::CoreMessage::Shutdown);
    // Wait up to 10 seconds for vexa-bot to disconnect (video finalization may take a few seconds)
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(10) {
        if bridge_state.connection_count.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}
// Force kill if still running
if let Ok(mut proc) = bot_process.lock() {
    let _ = proc.stop();
}
```

**Step 5: Log video path on shutdown**

In the "Session Complete" print block at the end, add after the Audio line:
```rust
if let Some(ref video_path) = session_video_path {
    // Check if video_ready event gave us a different path
    let final_path = bridge_state.video_ready_path.lock().await.clone();
    let display_path = final_path.as_deref().unwrap_or(&video_path.to_string_lossy());
    println!("Video:      {}", display_path);
}
```

**Step 6: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | head -30`
Expected: may have errors in `server.rs` join_meeting handler — fix in next step.

**Step 7: Commit**

```bash
git add jarvis/src/main.rs
git commit -m "feat: wire video recording into main startup and shutdown"
```

---

### Task 7: Fix server.rs join_meeting caller

**Files:**
- Modify: `jarvis/src/server.rs`

**Step 1: Update join_meeting proc.start() call**

Find the `proc.start(` call in `join_meeting()` (around line 217). It needs the three new parameters. Since the web UI join doesn't have video context, pass defaults:
```rust
match proc.start(&node_path, &vexa_bot_dir, &bridge_url, &meet_url, &bot_name, false, "", false) {
```

Note: Video recording from web UI join is a future enhancement — for now it only works when Jarvis starts with `record_video: true` in config and auto-joins.

**Step 2: Build to verify**

Run: `cd jarvis && cargo build 2>&1 | head -20`
Expected: builds successfully with no errors

**Step 3: Commit**

```bash
git add jarvis/src/server.rs
git commit -m "fix: update join_meeting to match new start() signature"
```

---

### Task 8: Handle shutdown command in vexa-bot bridge-client

**Files:**
- Modify: `services/vexa-bot/core/src/services/bridge-client.ts`

**Step 1: Add onShutdown handler**

Add a new private field and public method to `BridgeClient`:

After `private onSpeakAudio` field:
```typescript
private onShutdown: (() => void) | null = null;
```

Add public method after `onSpeakReceived`:
```typescript
/**
 * Register handler for shutdown command from Rust.
 * Called when Jarvis wants vexa-bot to gracefully exit.
 */
onShutdownReceived(handler: () => void): void {
  this.onShutdown = handler;
}
```

**Step 2: Handle shutdown in message handler**

In the `on('message')` handler, add a case for shutdown after the speak/command checks:
```typescript
if (msg.type === 'shutdown' && this.onShutdown) {
  this.onShutdown();
}
```

**Step 3: Add sendVideoReady method**

Add after `sendEvent`:
```typescript
/**
 * Notify Rust that video file is ready.
 */
sendVideoReady(path: string): boolean {
  return this.sendEvent('video_ready', { path });
}
```

**Step 4: Commit**

```bash
git add services/vexa-bot/core/src/services/bridge-client.ts
git commit -m "feat: handle shutdown command and add sendVideoReady in bridge client"
```

---

### Task 9: Add recordVideo to browser context and handle video finalization

**Files:**
- Modify: `services/vexa-bot/core/src/index.ts`

**Step 1: Add fs and child_process imports**

At the top of the file, add:
```typescript
import { execSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
```

**Step 2: Create temp video directory**

In the `runBot()` function, before the browser context creation block for non-Teams platforms (around line 1221), compute the temp video dir:
```typescript
// Video recording setup
const recordVideo = process.env.RECORD_VIDEO === 'true';
const videoOutputPath = process.env.VIDEO_OUTPUT_PATH || '';
const ffmpegAvailable = process.env.FFMPEG_AVAILABLE === 'true';
let tempVideoDir: string | null = null;
if (recordVideo) {
  tempVideoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vexa-video-'));
  log(`[Video] Recording enabled, temp dir: ${tempVideoDir}`);
}
```

**Step 3: Add recordVideo to browser context options**

Find the non-Teams `browserInstance.newContext()` call (around line 1221). Add `recordVideo` to the options:
```typescript
const context = await browserInstance.newContext({
  permissions: ["camera", "microphone"],
  userAgent: userAgent,
  viewport: {
    width: 1280,
    height: 720
  },
  ...(recordVideo && tempVideoDir ? {
    recordVideo: {
      dir: tempVideoDir,
      size: { width: 1280, height: 720 }
    }
  } : {}),
});
```

**Step 4: Add video finalization function**

Before the `performGracefulLeave` function (around line 510), add:
```typescript
async function finalizeVideo(): Promise<string | null> {
  if (!page || process.env.RECORD_VIDEO !== 'true') return null;
  const videoOutputPath = process.env.VIDEO_OUTPUT_PATH || '';
  if (!videoOutputPath) return null;

  try {
    const videoPath = await page.video()?.path();
    if (!videoPath) {
      log('[Video] No video path available from Playwright');
      return null;
    }
    log(`[Video] Temp video file: ${videoPath}`);

    // Wait for the file to be fully written
    await new Promise(resolve => setTimeout(resolve, 1000));

    const ffmpegAvailable = process.env.FFMPEG_AVAILABLE === 'true';
    if (ffmpegAvailable) {
      try {
        execSync(`ffmpeg -y -i "${videoPath}" -c:v copy -c:a copy "${videoOutputPath}"`, {
          timeout: 30000,
          stdio: 'pipe',
        });
        log(`[Video] Remuxed to MKV: ${videoOutputPath}`);
        // Clean up temp file
        try { fs.unlinkSync(videoPath); } catch {}
        return videoOutputPath;
      } catch (ffmpegErr: any) {
        log(`[Video] ffmpeg remux failed: ${ffmpegErr.message}`);
        // Fallback: rename temp webm to output path with .webm extension
        const webmPath = videoOutputPath.replace(/\.[^.]+$/, '.webm');
        try {
          fs.renameSync(videoPath, webmPath);
          log(`[Video] Saved as WebM fallback: ${webmPath}`);
          return webmPath;
        } catch (renameErr: any) {
          log(`[Video] Rename failed: ${renameErr.message}`);
          return videoPath; // Return temp path as last resort
        }
      }
    } else {
      // No ffmpeg — just rename the webm
      const webmPath = videoOutputPath.replace(/\.[^.]+$/, '.webm');
      try {
        fs.renameSync(videoPath, webmPath);
        log(`[Video] Saved as WebM: ${webmPath}`);
        return webmPath;
      } catch (renameErr: any) {
        log(`[Video] Rename failed: ${renameErr.message}`);
        return videoPath;
      }
    }
  } catch (err: any) {
    log(`[Video] Finalization error: ${err.message}`);
    return null;
  }
}
```

**Step 5: Integrate video finalization into performGracefulLeave**

In `performGracefulLeave()`, add video finalization after the page close block (after line ~648 "Page closed.") but before browser close:
```typescript
// Finalize video recording if enabled
let finalVideoPath: string | null = null;
try {
  finalVideoPath = await finalizeVideo();
  if (finalVideoPath && bridgeClient?.isConnected()) {
    bridgeClient.sendVideoReady(finalVideoPath);
    log(`[Video] Sent video_ready to Jarvis: ${finalVideoPath}`);
  }
} catch (videoErr: any) {
  log(`[Video] Error during finalization: ${videoErr.message}`);
}
```

Wait — `finalizeVideo` calls `page.video()?.path()` but page is already closed at this point. We need to capture the video path BEFORE closing the page. Let me adjust:

**Step 5 (revised): Capture video path before page close, finalize after**

In `performGracefulLeave()`, BEFORE the page close block (before "Ensuring page is closed"), add:
```typescript
// Capture video path before closing (Playwright needs open page for video().path())
let tempVideoPath: string | null = null;
if (process.env.RECORD_VIDEO === 'true' && page && !page.isClosed()) {
  try {
    tempVideoPath = await page.video()?.path() || null;
    if (tempVideoPath) log(`[Video] Captured temp video path: ${tempVideoPath}`);
  } catch (e: any) {
    log(`[Video] Could not get video path: ${e.message}`);
  }
}
```

Then AFTER the page close + context close (after browser close), add the finalization using the captured path. Actually, we should finalize AFTER `page.close()` and `context.close()` (since Playwright finalizes the video on context close), but BEFORE `browserInstance.close()`. Revise the `finalizeVideo` function to accept the temp path as argument instead of reading from page:

**Revised finalizeVideo:**
```typescript
async function finalizeVideo(tempVideoPath: string): Promise<string | null> {
  const videoOutputPath = process.env.VIDEO_OUTPUT_PATH || '';
  if (!videoOutputPath || !tempVideoPath) return null;

  try {
    // Wait for Playwright to finish writing
    await new Promise(resolve => setTimeout(resolve, 1000));

    const ffmpegAvailable = process.env.FFMPEG_AVAILABLE === 'true';
    if (ffmpegAvailable) {
      try {
        execSync(`ffmpeg -y -i "${tempVideoPath}" -c:v copy -c:a copy "${videoOutputPath}"`, {
          timeout: 30000,
          stdio: 'pipe',
        });
        log(`[Video] Remuxed to MKV: ${videoOutputPath}`);
        try { fs.unlinkSync(tempVideoPath); } catch {}
        return videoOutputPath;
      } catch (ffmpegErr: any) {
        log(`[Video] ffmpeg remux failed: ${ffmpegErr.message}`);
      }
    }
    // Fallback: rename to webm
    const webmPath = videoOutputPath.replace(/\.[^.]+$/, '.webm');
    try {
      fs.renameSync(tempVideoPath, webmPath);
      log(`[Video] Saved as WebM: ${webmPath}`);
      return webmPath;
    } catch (renameErr: any) {
      log(`[Video] Rename failed, temp file at: ${tempVideoPath}`);
      return tempVideoPath;
    }
  } catch (err: any) {
    log(`[Video] Finalization error: ${err.message}`);
    return null;
  }
}
```

**Step 6: Wire it all in performGracefulLeave — the correct order:**

1. Get `tempVideoPath` from `page.video()?.path()` (before page close)
2. Close page (Playwright starts finalizing video)
3. Close context (Playwright finishes video write)
4. If `tempVideoPath`: call `finalizeVideo(tempVideoPath)` → remux/rename
5. Send `video_ready` event via bridge
6. Close browser
7. Exit

**Step 7: Register shutdown handler for bridge mode**

In `runBot()`, after the bridge client setup (find the section where `bridgeClient.onSpeakReceived` is registered), add:
```typescript
bridgeClient.onShutdownReceived(() => {
  log('[Bridge] Received shutdown command from Jarvis');
  if (!isShuttingDown) {
    performGracefulLeave(page, 0, 'bridge_shutdown');
  }
});
```

**Step 8: Build vexa-bot**

Run: `cd services/vexa-bot && npm run build 2>&1 | tail -5`
Expected: builds successfully

**Step 9: Commit**

```bash
git add services/vexa-bot/core/src/index.ts
git commit -m "feat: add video recording via Playwright recordVideo with ffmpeg remux"
```

---

### Task 10: Build, test, and verify end-to-end

**Step 1: Build Rust**

Run: `cd jarvis && cargo build 2>&1 | tail -10`
Expected: success

**Step 2: Build TypeScript**

Run: `cd services/vexa-bot && npm run build 2>&1 | tail -5`
Expected: success

**Step 3: Add record_video to actual config**

Edit `jarvis.config.json`: add `"record_video": true`

**Step 4: Run Jarvis and verify video recording starts**

Run: `RUST_LOG=jarvis=debug ./jarvis/target/debug/jarvis`

Check logs for:
- `Session video: .../sessions/YYYY-MM-DD_HHMMSS.mkv` (or .webm)
- `[Video] Recording enabled, temp dir: /tmp/vexa-video-XXXXX`

**Step 5: Join a meeting and let it run briefly**

Let the bot join, wait ~30 seconds for audio capture to confirm working.

**Step 6: Stop with Ctrl+C and verify video**

Check logs for:
- `Sending shutdown command to vexa-bot...`
- `[Video] Remuxed to MKV: ...` (or `Saved as WebM`)
- `[bridge] video ready: ...`
- Video path printed in Session Complete block

Check the video file exists and plays:
```bash
ls -la ~/Library/Application\ Support/jarvis/sessions/*.mkv
# or *.webm
```

**Step 7: Commit all remaining changes**

```bash
git add -A
git commit -m "feat: complete video recording feature with config, bridge protocol, and ffmpeg remux"
```
