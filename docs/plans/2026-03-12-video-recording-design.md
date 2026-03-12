# Video Recording Design

## Goal

Record full meeting video (composite page view) at 720p as MKV (or MP4 with re-encode), saved alongside existing WAV + transcript files.

## Approach

Use Playwright's built-in `recordVideo` option when creating the browser context via `browser.newContext()` (NOT `launchPersistentContext`, which doesn't support `recordVideo`). Playwright captures the browser page at 1280x720 and writes a WebM (VP8) file automatically. On meeting end, vexa-bot gracefully closes the page, then runs one of:

- **ffmpeg present:** `ffmpeg -i temp.webm -c:v copy -c:a copy output.mkv` (zero-cost container remux — MKV supports VP8 natively, unlike MP4)
- **ffmpeg missing:** `fs.rename(temp.webm, output.webm)` (move, not copy — avoids doubling disk usage)

## Config

New field in `jarvis.config.json`:

```json
{
  "record_video": false
}
```

Defaults to `false`. Exposed via config API endpoint for future UI toggle (no Web UI changes in this iteration — API-only).

## Output

```
~/Library/Application Support/jarvis/sessions/YYYY-MM-DD_HHMMSS.mkv   (or .webm)
~/Library/Application Support/jarvis/sessions/YYYY-MM-DD_HHMMSS.wav
~/Library/Application Support/jarvis/sessions/YYYY-MM-DD_HHMMSS.txt
```

## Data Flow

```
jarvis.config.json → Config.record_video
  → main.rs checks ffmpeg on PATH, computes video output path using session timestamp
  → process.rs::start() receives new parameters: record_video, video_output_path, ffmpeg_available
  → process.rs sets env vars: RECORD_VIDEO=true, VIDEO_OUTPUT_PATH=..., FFMPEG_AVAILABLE=true/false
  → vexa-bot reads env vars at startup
  → vexa-bot uses browser.newContext() (not launchPersistentContext) with:
      recordVideo: { dir: '/tmp/vexa-video-<session_id>/', size: {width: 1280, height: 720} }
  → Viewport set to 1280x720 to match recording resolution
  → Meeting runs, Playwright records page automatically
  → Graceful shutdown: Jarvis sends "shutdown" command via bridge WebSocket
  → vexa-bot receives shutdown → calls page.close() → context.close()
  → Playwright finalizes video → temp .webm file ready at page.video().path()
  → ffmpeg available: ffmpeg -i temp.webm -c:v copy -c:a copy output.mkv
  → ffmpeg missing: fs.rename(temp.webm, output.webm)
  → vexa-bot sends video_ready message with final path via bridge
  → Clean up /tmp/vexa-video-<session_id>/
  → vexa-bot exits
  → Jarvis logs final video path on shutdown (received from bridge message, or computes expected path as fallback)
  → If vexa-bot doesn't exit within 5s after shutdown command, Jarvis sends SIGKILL
```

## File Changes

| File | Change |
|---|---|
| `jarvis/src/config.rs` | Add `record_video: bool` field to `ConfigFile` (Option, default `false`) and `Config` |
| `jarvis/src/process.rs` | Add `record_video`, `video_output_path`, `ffmpeg_available` parameters to `start()`. Pass as env vars to child process. |
| `jarvis/src/server.rs` | Add `record_video` to config API request/response |
| `jarvis/src/main.rs` | Check ffmpeg on PATH (via `which`/`where`), compute video output path using session timestamp, pass new params to `process.rs::start()`, log video path on shutdown |
| `jarvis/src/bot_bridge.rs` | Add `Shutdown` variant to `CoreMessage` (Jarvis→bot). Add `VideoReady { path: String }` variant to inbound messages (bot→Jarvis). |
| `services/vexa-bot/core/src/index.ts` | Read env vars, switch from `launchPersistentContext` to `browser.launch()` + `browser.newContext()` with `recordVideo`, handle shutdown message gracefully (page.close → ffmpeg/rename → send video_ready → exit) |
| `services/vexa-bot/core/src/services/bridge-client.ts` | Handle incoming `shutdown` command, emit event for graceful teardown |
| `jarvis.config.example.json` | Add `"record_video": false` |

## Files NOT Changed

- `recording.ts` — audio capture untouched
- `join.ts` — RTCPeerConnection hook unchanged
- Web UI (`index.html`) — no UI changes in this iteration

## Browser Context Change: launchPersistentContext → newContext

The current codebase uses `chromium.launchPersistentContext()`. Playwright's `recordVideo` option is only supported on `browser.newContext()`, not on persistent contexts.

**Required change in `index.ts`:**
```
// Before:
const context = await chromium.launchPersistentContext(userDataDir, { ... });
const page = context.pages()[0];

// After:
const browser = await chromium.launch({ args: [...] });
const context = await browser.newContext({
  recordVideo: process.env.RECORD_VIDEO === 'true'
    ? { dir: tempVideoDir, size: { width: 1280, height: 720 } }
    : undefined,
  viewport: { width: 1280, height: 720 },
  userAgent: '...',
});
const page = await context.newPage();
```

This is a slightly larger change than originally scoped but is required for `recordVideo` to work. The persistent context was used for cookie persistence — if that's needed, manage cookies explicitly via `context.addCookies()` / `context.storageState()`.

## Graceful Shutdown Protocol

Current problem: `process.rs::stop()` calls `child.start_kill()` (SIGKILL). Playwright only finalizes video files on graceful `page.close()` / `context.close()`. SIGKILL = truncated/missing video.

**New shutdown sequence:**

1. Jarvis sends `{ "type": "shutdown" }` via bridge WebSocket
2. vexa-bot receives shutdown command:
   - Calls `page.close()` — Playwright finalizes video file
   - Calls `context.close()` — ensures all resources released
   - Runs ffmpeg remux (or rename fallback)
   - Sends `{ "type": "video_ready", "path": "/path/to/final.mkv" }` back via bridge
   - Calls `process.exit(0)`
3. Jarvis waits up to 5 seconds for vexa-bot to exit
4. If still running after 5s → SIGKILL as fallback

**Changes to `bot_bridge.rs`:**
- Add `Shutdown` to outbound `CoreMessage` enum
- Add handler for inbound `video_ready` message, store path in `BridgeState`

**Changes to `process.rs`:**
- New `graceful_stop()` method: send shutdown via bridge, wait with timeout, then kill
- Keep existing `stop()` as force-kill fallback

## ffmpeg Remux: VP8 Codec Compatibility

**Critical fix:** The original plan used `ffmpeg -c copy output.mp4`. This is broken — MP4 container does not support VP8 codec. ffmpeg would either error or produce an unplayable file.

**Options considered:**

| Option | Command | Speed | Compatibility |
|---|---|---|---|
| MKV remux (chosen) | `ffmpeg -i in.webm -c:v copy -c:a copy out.mkv` | ~1 second (no re-encode) | VLC, mpv, most players |
| MP4 re-encode | `ffmpeg -i in.webm -c:v libx264 -preset ultrafast out.mp4` | Minutes (CPU-heavy) | Universal |
| Keep WebM | just rename | Instant | VLC, browsers, not QuickTime |

**Decision: MKV remux.** Zero re-encoding cost, near-universal playback support (VLC, mpv, Windows Media Player with codecs). QuickTime on macOS won't play it natively, but VLC is the standard for devs. If users need MP4, they can convert manually.

## ffmpeg Detection & Fallback

On startup when `record_video` is true:
1. Jarvis checks if `ffmpeg` is on PATH via `which ffmpeg` (Unix) / `where ffmpeg` (Windows)
2. If missing: logs warning — "record_video enabled but ffmpeg not found; video will be saved as .webm"
3. Passes `FFMPEG_AVAILABLE=true/false` env var to vexa-bot

Why not bundle ffmpeg:
- Binary is ~80-100MB, would bloat the project
- Most dev machines already have it (`brew install ffmpeg`)
- WebM fallback is usable — VLC, Chrome, Firefox all play it

## Temp Directory Isolation

Each session uses a unique temp directory to prevent collisions when multiple Jarvis instances run:

```
/tmp/vexa-video-{session_id}/
```

Where `session_id` is the same `YYYY-MM-DD_HHMMSS` timestamp used for session files. Passed via `VIDEO_TEMP_DIR` env var.

Cleanup: vexa-bot removes the temp directory after remux/rename completes. On crash, temp files remain but are harmless (OS cleans `/tmp` on reboot).

## Error Handling

- If Playwright `recordVideo` fails at context creation: catch error, log warning, re-create context without `recordVideo`. Audio + transcript unaffected since they use WebRTC streams captured independently.
- If disk fills during recording: Playwright may crash the browser context. This would kill audio capture too (since it runs in the same browser). **Mitigation:** monitor temp video file size periodically; if approaching a threshold, log a warning. Acceptable risk — disk-full is an edge case that affects the entire system, not just video.
- If ffmpeg remux fails: keep the `.webm` file, log the path. User can convert manually.
- If graceful shutdown times out (vexa-bot doesn't exit in 5s): SIGKILL. Video file may be truncated — log a warning with temp file location so user can attempt manual recovery.
- On crash/ungraceful exit: temp files remain in `/tmp/vexa-video-{session_id}/` but are harmless.

## Runtime Config Toggle

`record_video` can be changed via the config API at any time, but **only takes effect on the next meeting start** (since the env vars are passed at process spawn time). Toggling mid-meeting has no effect on the current recording session.

## New Dependencies

- `ffmpeg` (optional, host-installed) — for WebM → MKV container remux only (`-c copy`, no re-encoding)

## Scope

~8 focused changes across Rust and TypeScript. Key architectural changes:
- Browser context creation method (`launchPersistentContext` → `launch` + `newContext`)
- New shutdown handshake via bridge WebSocket (1 new outbound + 1 new inbound message type)
- Video recording is otherwise orthogonal to existing audio/transcript functionality
