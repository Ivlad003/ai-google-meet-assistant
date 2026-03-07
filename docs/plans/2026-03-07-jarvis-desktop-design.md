# Jarvis Desktop App — Design Document

## Goal

Replace 15 Docker containers with a single Rust binary + embedded Node.js for vexa-bot.
User downloads one archive, runs `./jarvis`, opens http://localhost:8080 — done.

## Architecture

```
jarvis (one process)
|
+-- Rust Core
|   +-- WebSocket server (:9090)      <-- vexa-bot connects here
|   |   +-- audio stream from vexa-bot
|   |   +-- commands to vexa-bot (speak, stop)
|   |   +-- events from vexa-bot (joined, participants)
|   |
|   +-- Transcription (user choice)
|   |   +-- local: whisper-rs (whisper.cpp)
|   |   +-- cloud: OpenAI Whisper API
|   |
|   +-- Intent Detection (GPT-4o-mini)
|   +-- LLM Responses (GPT-4o)
|   +-- TTS (OpenAI API -> audio -> vexa-bot)
|   |
|   +-- SQLite (history, settings)
|   +-- Web UI (:8080, rust-embed)
|   |   +-- GET /                  -- dashboard
|   |   +-- GET/POST /api/config   -- settings
|   |   +-- GET /api/status        -- status
|   |   +-- POST /api/join         -- join meeting
|   |   +-- POST /api/leave        -- leave meeting
|   |   +-- WS /api/transcript     -- live transcript
|   |
|   +-- Process Manager
|       +-- launches vexa-bot as child process
|
+-- vexa-bot (child process)
    +-- node (embedded runtime)
    +-- Playwright + Chromium
        +-- joins Google Meet
        +-- RTCPeerConnection hook (captures audio)
        +-- streams audio -> ws://localhost:9090/ws/audio
        +-- receives speak commands <- ws://localhost:9090/ws/commands
```

## What Gets Removed vs. What Stays

| Was (Docker)                  | Becomes (Rust)                                   |
|-------------------------------|--------------------------------------------------|
| PostgreSQL                    | SQLite (embedded)                                |
| Redis                         | Direct WebSocket (no queues)                     |
| MinIO                         | Local filesystem                                 |
| admin-api (Python)            | Not needed -- single user                        |
| api-gateway (Python)          | Not needed -- direct connection                  |
| bot-manager (Python)          | Process Manager in Rust (launches vexa-bot)      |
| transcription-service (Python)| whisper-rs or OpenAI API                         |
| WhisperLive (Python)          | WebSocket server in Rust                         |
| transcription-collector (Python)| Logic in Rust Core                             |
| tts-service (Python)          | Direct OpenAI TTS API call                       |
| init-setup                    | Not needed -- nothing to initialize              |
| meet-bot (Go)                 | Rust Core                                        |
| vexa-bot (Node.js)            | **Stays** (child process)                        |

## WebSocket Protocol (Rust <-> vexa-bot)

Single WebSocket connection on ws://localhost:9090/ws

### vexa-bot -> Rust

```json
// Audio chunk (PCM float32, 16kHz, base64)
{"type": "audio", "data": "<base64>", "sample_rate": 16000}

// Events
{"type": "event", "event": "joined", "data": {"meeting_id": "abc-defg-hij"}}
{"type": "event", "event": "speaker_start", "data": {"name": "Vlad", "id": "..."}}
{"type": "event", "event": "speaker_end", "data": {"name": "Vlad", "id": "..."}}
{"type": "event", "event": "participants", "data": {"list": ["Vlad", "Jarvis"]}}
{"type": "event", "event": "left", "data": {}}
```

### Rust -> vexa-bot

```json
// Speak TTS audio
{"type": "speak", "audio": "<base64 wav>"}

// Commands
{"type": "command", "action": "mute"}
{"type": "command", "action": "unmute"}
{"type": "command", "action": "leave"}
```

## Rust Dependencies (Cargo.toml)

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.7"
tower-http = { version = "0.5", features = ["fs", "cors"] }
tokio-tungstenite = "0.21"
reqwest = { version = "0.12", features = ["json", "multipart"] }
whisper-rs = "0.12"
rusqlite = { version = "0.31", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rust-embed = "8"
base64 = "0.22"
tracing = "0.1"
tracing-subscriber = "0.3"
clap = { version = "4", features = ["derive"] }
```

## File Structure

```
jarvis/
+-- Cargo.toml
+-- src/
|   +-- main.rs              # Entry point, CLI args, startup
|   +-- config.rs            # Config (.env, CLI args, SQLite)
|   +-- server.rs            # Axum HTTP + WS server (:8080)
|   +-- bot_bridge.rs        # WebSocket server for vexa-bot (:9090)
|   +-- transcription/
|   |   +-- mod.rs           # trait Transcriber
|   |   +-- local.rs         # whisper-rs implementation
|   |   +-- cloud.rs         # OpenAI Whisper API
|   +-- llm.rs               # OpenAI GPT-4o (intent + respond + summary)
|   +-- tts.rs               # OpenAI TTS API
|   +-- db.rs                # SQLite (history, settings)
|   +-- process.rs           # Launch vexa-bot as child process
|   +-- assets/
|       +-- index.html       # Web UI (rust-embed)
+-- vexa-bot/                # Copy from current project
|   +-- core/
+-- node/                    # Embedded Node.js runtime (added at build)
```

## Transcription Modes

### Local (whisper-rs)

- Models: base (~140MB), small (~460MB), medium (~1.5GB)
- Downloaded on first use to ~/.jarvis/models/
- CPU-only by default, GPU if available
- Latency: ~1-3s per chunk on modern CPU

### Cloud (OpenAI Whisper API)

- POST https://api.openai.com/v1/audio/transcriptions
- Cost: ~$0.006/minute
- Best accuracy
- Requires internet

User selects mode via Web UI or CLI flag: `--transcription local|cloud`

## User Experience

```bash
# First run
$ ./jarvis --openai-key sk-... --meet https://meet.google.com/abc-defg-hij

# Output:
# Jarvis v1.0 starting...
# Web UI: http://localhost:8080
# Downloading Chromium (first run)... done
# Joining meeting abc-defg-hij...
# Connected. Listening for speech.

# With .env file
$ echo "OPENAI_API_KEY=sk-..." > .env
$ ./jarvis --meet https://meet.google.com/abc-defg-hij

# Just Web UI (set meeting later through dashboard)
$ ./jarvis
```

## Configuration

| Flag / Env | Default | Description |
|---|---|---|
| `--openai-key` / `OPENAI_API_KEY` | (required) | OpenAI API key |
| `--meet` / `MEET_URL` | (optional) | Meeting URL |
| `--bot-name` / `BOT_DISPLAY_NAME` | Jarvis | Bot name in meeting |
| `--transcription` / `TRANSCRIPTION_MODE` | cloud | local or cloud |
| `--whisper-model` | small | base, small, medium (local only) |
| `--port` / `WEB_UI_PORT` | 8080 | Web UI port |
| `--tts-voice` / `TTS_VOICE` | nova | OpenAI TTS voice |
| `--model` / `OPENAI_MODEL` | gpt-4o | LLM model |

Settings persist in SQLite (~/.jarvis/jarvis.db).

## vexa-bot Modifications

Current vexa-bot connects to Redis for commands and sends audio to WhisperLive.
Need to modify it to:

1. Connect to ws://localhost:9090/ws instead of Redis
2. Stream audio chunks over WebSocket instead of WhisperLive
3. Receive speak commands (with audio data) over WebSocket instead of Redis
4. Remove all Redis, bot-manager, WhisperLive dependencies

Files to modify:
- `vexa-bot/core/src/services/audio/` -- redirect audio output to WS
- `vexa-bot/core/src/services/commands/` -- listen on WS instead of Redis
- `vexa-bot/core/src/platforms/googlemeet/join.ts` -- keep RTCPeerConnection hook as-is

## Build & Distribution

```bash
# Development
$ cargo run -- --openai-key sk-... --meet https://meet.google.com/abc

# Release build
$ cargo build --release
# Output: target/release/jarvis (~20MB)

# Package with Node.js + vexa-bot
$ ./scripts/package.sh macos-arm64
# Output: jarvis-v1.0-macos-arm64.tar.gz (~70MB)
#   jarvis/
#     jarvis        (Rust binary)
#     node          (Node.js runtime)
#     vexa-bot/     (Node.js code)
```

Target platforms:
- macOS arm64 (Apple Silicon)
- macOS x64 (Intel)
- Linux x64
- Windows x64

## Language Support

- Bot responds in English or Ukrainian only
- Intent detection prompt includes language constraint
- Transcription: Whisper auto-detects language

## Development Order (7 tasks)

1. **Scaffold** -- Cargo.toml, main.rs, CLI args, config, SQLite
2. **Bot Bridge** -- WebSocket server :9090, protocol with vexa-bot
3. **Modify vexa-bot** -- connect to local WS instead of Redis/bot-manager
4. **Transcription** -- whisper-rs (local) + OpenAI API (cloud), trait Transcriber
5. **LLM + TTS** -- intent detection, respond, summary, OpenAI TTS
6. **Web UI** -- axum server :8080, port index.html, WS transcript
7. **Process Manager** -- launch Node.js + vexa-bot, embedded runtime, packaging
