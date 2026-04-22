# Technical Details

## Bridge Mode Chrome Flags

- `--mute-audio` — prevents Chrome audio output (no echo)
- `--use-fake-device-for-media-stream` — prevents real mic capture (no feedback loop)
- `--use-file-for-fake-audio-capture=/dev/null` — silence as mic input

## Key Technical Details

- **RTCPeerConnection hook** (join.ts) — patches `RTCPeerConnection` before page load to capture remote audio tracks into `__vexaCapturedRemoteAudioStreams`
- **Silence detection** (transcription/mod.rs) — RMS threshold (0.005) filters silence before Whisper, preventing hallucinations
- **Hallucination filter** (transcription/mod.rs) — catches repeated words, YouTube/podcast outros (EN/UK/RU), music markers, short alphanumeric fragments
- **HTTP error handling** — cloud.rs and local.rs validate HTTP status codes before processing responses
- **Bridge mode detection** — `process.env.BRIDGE_URL` via `isBridgeMode()`, not `bridgeClient` (set later)
- **Meeting monitoring** — bridge mode keeps recording promise pending via participant counting loop
- **Speaker detection** — bridge mode polls participant tiles for speaking indicators via `__vexaBridgeSpeakerEvent`, tracks last speaker in `BridgeState.current_speaker`
- **Audio recording** — all incoming audio saved to WAV file via `hound` crate (16kHz mono 16-bit PCM)
- **Session transcripts** — each session writes `[HH:MM:SS] [Speaker] text` to a `.txt` file
- **File logging** — `tracing-appender` writes daily rotated logs alongside console output

## Docker Environment

### How Headless WebRTC Works

Google Meet requires a headed browser for WebRTC. In Docker, we use:
- **Xvfb** (`:99`) — virtual X11 framebuffer so Chrome thinks it has a display
- **PulseAudio** — virtual audio devices for TTS injection into meetings

The entrypoint (`docker/entrypoint.sh`) starts Xvfb and PulseAudio as root, then drops to `pwuser` via `gosu` before running Jarvis.

### Virtual Audio Devices

```
meet_sink     — null sink for general audio capture
tts_sink      — null sink for TTS audio injection
virtual_mic   — remap source from tts_sink.monitor (Chrome sees as real mic)
```

Audio capture in bridge mode bypasses PulseAudio — it hooks `RTCPeerConnection` directly via `AudioContext` + `ScriptProcessorNode`.

### Docker-Specific Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DOCKER_MODE` | `1` | Set automatically. Skips browser auto-open |
| `VEXA_BOT_DIR` | `/app/vexa-bot` | Path to vexa-bot inside container |
| `JARVIS_DATA_DIR` | `/data/jarvis` | Data directory (sessions, logs, db) |
| `JARVIS_CONFIG` | `/etc/jarvis/config.json` | Mounted config file path |
| `DISPLAY` | `:99` | Xvfb virtual display |
| `PULSE_SERVER` | `unix:/tmp/pulse-server` | PulseAudio socket for cross-user access |

### Docker Data Paths

```
/data/jarvis/              (mounted volume: jarvis-data)
  sessions/
    YYYY-MM-DD_HHMMSS.txt  — transcript
    YYYY-MM-DD_HHMMSS.wav  — audio recording
  logs/
    jarvis.log.YYYY-MM-DD  — daily rotated logs
  jarvis.db                — SQLite database
```

### Multi-Stage Build

1. **rust-builder** (Ubuntu 22.04) — compiles Jarvis binary. Must match runtime glibc (Jammy = 2.35)
2. **ts-builder** (node:20-bookworm) — compiles vexa-bot TypeScript + browser-utils bundle
3. **runtime** (playwright:v1.56.0-jammy) — Xvfb, PulseAudio, ffmpeg, gosu + both artifacts

### Security

- Jarvis runs as `pwuser` (UID 1001), not root
- Xvfb/PulseAudio start as root in entrypoint, then `gosu pwuser` drops privileges
- Caddy is the sole entry point — Jarvis port is not exposed externally (`expose` not `ports`)
- Caddy enforces HTTP `basic_auth` on all requests (user/hash from env vars)
- API key is never exposed via `/api/config` GET endpoint
- For VPS: replace `:8080` with domain in Caddyfile for auto HTTPS via Let's Encrypt

### Caddy Auth Setup

```bash
# Generate bcrypt hash
docker run --rm caddy:2-alpine caddy hash-password --plaintext 'your-password'

# In .env — escape $ as $$ for docker-compose
CADDY_AUTH_USER=admin
CADDY_AUTH_HASH=$$2a$$14$$your-hash-here
```

Note: `basicauth` is deprecated in Caddy 2.10+ — use `basic_auth` (with underscore).

## Session Output Files

**Native** — saved to `~/Library/Application Support/jarvis/` (macOS):
**Docker** — saved to `/data/jarvis/` (mounted volume):

- `sessions/YYYY-MM-DD_HHMMSS.txt` — transcript with timestamps and speaker names
- `sessions/YYYY-MM-DD_HHMMSS.wav` — full audio recording
- `logs/jarvis.log.YYYY-MM-DD` — daily rotated log files

On shutdown, paths are printed to terminal.

## Configuration Reference

All settings in `jarvis.config.json`. Loads from current directory by default.

```bash
# Default (loads ./jarvis.config.json)
./jarvis/target/debug/jarvis

# Custom path
./jarvis/target/debug/jarvis --config /path/to/config.json

# Or via env var
JARVIS_CONFIG=/path/to/config.json ./jarvis/target/debug/jarvis
```

See `jarvis.config.example.json` for all options.

| Field | Required | Default | Description |
|---|---|---|---|
| `openai_key` | **Yes** | — | OpenAI API key |
| `meet_url` | No | — | Meeting URL (can set via Web UI) |
| `bot_name` | No | `Jarvis` | Bot display name |
| `language` | No | `auto` | Transcription language (en, uk, auto) |
| `openai_model` | No | `gpt-5.4` | LLM model for responses |
| `intent_model` | No | `gpt-5` | Intent detection model (uses reasoning_effort=minimal) |
| `tts_voice` | No | `nova` | OpenAI TTS voice |
| `transcription_mode` | No | `cloud` | `cloud` (OpenAI API) or `local` (whisper-rs) |
| `whisper_model` | No | `small` | Local whisper model name |
| `port` | No | `8080` | Web UI port |
| `bridge_port` | No | `9090` | Bridge WebSocket port |
| `max_response_tokens` | No | `150` | Max tokens in LLM response |
| `temperature` | No | `0.7` | LLM temperature |
| `response_mode` | No | `smart` | `smart` (LLM intent detection) or `name_only` (keyword match on bot name) |
| `system_prompt` | No | built-in | Custom system prompt |
| `intent_prompt` | No | built-in | Custom intent prompt (`{bot_name}`, `{context}`, `{speaker}`, `{text}`) |
| `tools` | No | `[]` | Custom tool integrations |
