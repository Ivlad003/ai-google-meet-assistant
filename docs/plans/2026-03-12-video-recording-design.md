# Video Recording Design

## Goal

Record full meeting video (composite page view) at 720p as MP4, saved alongside existing WAV + transcript files.

## Approach

Use Playwright's built-in `recordVideo` option when creating the browser context. Playwright captures the browser page at 1280x720 and writes a WebM (VP8) file automatically. On meeting end, run a lightweight `ffmpeg -c copy` remux to convert WebM → MP4 (no re-encoding, ~1 second). Falls back to saving as `.webm` if ffmpeg is not installed.

## Config

New field in `jarvis.config.json`:

```json
{
  "record_video": false
}
```

Defaults to `false`. Exposed via config API for future UI toggle.

## Output

```
~/Library/Application Support/jarvis/sessions/YYYY-MM-DD_HHMMSS.mp4   (or .webm)
~/Library/Application Support/jarvis/sessions/YYYY-MM-DD_HHMMSS.wav
~/Library/Application Support/jarvis/sessions/YYYY-MM-DD_HHMMSS.txt
```

## Data Flow

```
jarvis.config.json → Config.record_video
  → process.rs sets env vars: RECORD_VIDEO=true, VIDEO_OUTPUT_PATH=..., FFMPEG_AVAILABLE=true/false
  → vexa-bot reads env vars at startup
  → Playwright creates browser context with recordVideo: { dir: '/tmp/vexa-video/', size: {width: 1280, height: 720} }
  → Viewport set to 1280x720 to match recording resolution
  → Meeting runs, Playwright records page automatically
  → Page closes → temp .webm file ready at page.video().path()
  → ffmpeg available: ffmpeg -i temp.webm -c copy output.mp4
  → ffmpeg missing: cp temp.webm output.webm
  → Clean up /tmp/vexa-video/
  → Jarvis logs final video path on shutdown
```

## File Changes

| File | Change |
|---|---|
| `jarvis/src/config.rs` | Add `record_video: bool` field, default `false` |
| `jarvis/src/process.rs` | Pass `RECORD_VIDEO`, `VIDEO_OUTPUT_PATH`, `FFMPEG_AVAILABLE` env vars to child process |
| `jarvis/src/server.rs` | Add `record_video` to config API request/response |
| `jarvis/src/main.rs` | Check ffmpeg on PATH, compute video output path, log it on shutdown |
| `services/vexa-bot/core/src/index.ts` | Read env vars, add `recordVideo` to context options, post-meeting ffmpeg remux |
| `jarvis.config.example.json` | Add `"record_video": false` |

## Files NOT Changed

- `recording.ts` — audio capture untouched
- `bridge-client.ts` — no protocol changes
- `bot_bridge.rs` — no new message types
- `join.ts` — RTCPeerConnection hook unchanged
- Web UI (`index.html`) — no UI changes in this iteration

## ffmpeg Detection & Fallback

On startup when `record_video` is true:
1. Jarvis checks if `ffmpeg` is on PATH via `which ffmpeg`
2. If missing: logs warning — "record_video enabled but ffmpeg not found; video will be saved as .webm"
3. Passes `FFMPEG_AVAILABLE=true/false` env var to vexa-bot

Why not bundle ffmpeg:
- Binary is ~80-100MB, would bloat the project
- Most dev machines already have it (`brew install ffmpeg`)
- WebM fallback is usable — VLC, Chrome, Firefox all play it

## Error Handling

- If Playwright `recordVideo` fails (permissions, disk space): log warning, don't crash the meeting. Audio + transcript still work independently.
- If ffmpeg remux fails: keep the `.webm` file and log the path.
- On crash/ungraceful exit: temp files remain in `/tmp/vexa-video/` but are harmless (OS cleans `/tmp`).

## New Dependency

- `ffmpeg` (optional, host-installed) — for WebM → MP4 container remux only (`-c copy`, no re-encoding)

## Scope

~6 small, focused changes. No architectural changes to the audio pipeline or WebSocket protocol. Video recording is entirely orthogonal to existing functionality.
