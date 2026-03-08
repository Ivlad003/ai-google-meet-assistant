# CLAUDE.md

This file provides guidance to Claude Code when working with code in this repository.

## What This Is

A desktop AI meeting assistant. **Jarvis** (Rust) is the core app — it runs a local web UI, manages a **vexa-bot** (TypeScript/Playwright) child process that joins Google Meet as a virtual participant, captures audio via WebRTC, transcribes speech (OpenAI Whisper API or local whisper-rs), detects intent via LLM, and speaks back through OpenAI TTS. Records full audio and saves timestamped transcripts with speaker names.

## Project Structure

```
jarvis/                    Rust desktop app
  src/
    main.rs                Entry point — CLI args, wiring, audio processing loop, WAV recording
    config.rs              Config from JSON file / env / CLI args
    server.rs              Axum web server — Web UI, REST API, WebSocket
    bot_bridge.rs          WebSocket bridge to vexa-bot (audio + commands + speaker tracking)
    process.rs             Manages vexa-bot child process (start/stop/find)
    transcription/
      mod.rs               Trait + silence detection + hallucination filter
      cloud.rs             OpenAI Whisper API transcriber
      local.rs             whisper-rs (local) transcriber
    llm.rs                 OpenAI chat — intent detection + response generation
    tts.rs                 OpenAI TTS synthesis
    db.rs                  SQLite for settings/history
    assets/index.html      Embedded Web UI

services/vexa-bot/         TypeScript bot framework
  core/src/
    index.ts               Main orchestrator — browser launch, recording, bridge
    constans.ts            Browser launch args (bridge mode flags)
    platforms/googlemeet/
      join.ts              RTCPeerConnection hook + meeting join flow
      recording.ts         Audio capture (direct WebRTC in bridge mode) + speaker detection
      selectors.ts         Google Meet DOM selectors
    services/
      bridge-client.ts     WebSocket client to Jarvis bridge

jarvis.config.example.json Example JSON config file
scripts/package-jarvis.sh  Packaging script
docs/plans/                Architecture designs and implementation plans
```

## Build & Run

```bash
# Build Jarvis
cd jarvis && cargo build

# Build vexa-bot TypeScript (MUST use npm run build, not just tsc)
cd services/vexa-bot && npm run build

# Run (from project root)
./jarvis/target/debug/jarvis

# With JSON config file
cp jarvis.config.example.json jarvis.config.json  # edit with your API key
./jarvis/target/debug/jarvis --config jarvis.config.json

# With env vars
MEET_URL=https://meet.google.com/abc-defg-hij LANGUAGE=uk ./jarvis/target/debug/jarvis
```

Requires: Node.js (v18+), Rust toolchain, OpenAI API key.

## Architecture — Audio Pipeline

```
Google Meet -> Playwright Chromium -> RTCPeerConnection hook
  -> WebRTC MediaStreams -> AudioContext + ScriptProcessorNode
  -> 48kHz to 16kHz resampling -> WebSocket bridge -> Jarvis
  -> WAV file recording (16kHz mono 16-bit PCM)
  -> Silence detection (RMS) -> Whisper (cloud or local) -> Transcript
  -> Speaker name from DOM observer -> Session transcript file
  -> Intent detection (GPT-5, reasoning_effort=minimal) -> Response (GPT-5.4)
  -> OpenAI TTS -> WAV base64 -> WebSocket -> vexa-bot
  -> WebRTC replaceTrack -> Meeting audio output
```

### Bridge Mode Chrome Flags

- `--mute-audio` — prevents Chrome audio output (no echo)
- `--use-fake-device-for-media-stream` — prevents real mic capture (no feedback loop)
- `--use-file-for-fake-audio-capture=/dev/null` — silence as mic input

### Key Technical Details

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

### Session Output Files

All saved to `~/Library/Application Support/jarvis/` (macOS):

- `sessions/YYYY-MM-DD_HHMMSS.txt` — transcript with timestamps and speaker names
- `sessions/YYYY-MM-DD_HHMMSS.wav` — full audio recording
- `logs/jarvis.log.YYYY-MM-DD` — daily rotated log files

On shutdown, paths are printed to terminal.

## Configuration

Three sources, priority: **CLI args/env vars > JSON config file > defaults**.

```bash
# Via JSON config (recommended)
./jarvis/target/debug/jarvis --config jarvis.config.json

# Via env vars
OPENAI_API_KEY=sk-... MEET_URL=... ./jarvis/target/debug/jarvis
```

See `jarvis.config.example.json` for all options.

### Environment Variables / CLI Args

| Variable | Default | Description |
|---|---|---|
| `OPENAI_API_KEY` | required | OpenAI API key |
| `MEET_URL` | none | Meeting URL (can set via Web UI) |
| `BOT_DISPLAY_NAME` | `Jarvis` | Bot name in meeting |
| `LANGUAGE` | `auto` | Transcription language (en, uk, auto) |
| `TRANSCRIPTION_MODE` | `cloud` | `cloud` (OpenAI API) or `local` (whisper-rs) |
| `WHISPER_MODEL` | `small` | Local whisper model name |
| `OPENAI_MODEL` | `gpt-5.4` | LLM model for responses |
| `TTS_VOICE` | `nova` | OpenAI TTS voice |
| `WEB_UI_PORT` | `8080` | Web UI port |
| `JARVIS_CONFIG` | none | Path to JSON config file |

### JSON Config Only Fields

These are only configurable via `jarvis.config.json`:

| Field | Default | Description |
|---|---|---|
| `intent_model` | `gpt-5` | Model for intent detection (uses reasoning_effort=minimal) |
| `system_prompt` | built-in | Custom system prompt for the bot |
| `intent_prompt` | built-in | Custom intent detection prompt (placeholders: `{bot_name}`, `{context}`, `{speaker}`, `{text}`) |
| `max_response_tokens` | `150` | Max tokens in LLM response |
| `temperature` | `0.7` | LLM temperature |
| `bridge_port` | `9090` | Bridge WebSocket port |

## Key Constraints

- **Google Meet requires headed browser** — headless doesn't support WebRTC properly
- **Google Meet DOM selectors change frequently** — selectors.ts uses fallback strategies with English + Ukrainian support
- **Avoid `jsname` selectors** — unstable. Use `aria-label`, `data-*`, text-based matching
- **whisper-rs not thread-safe** — single thread processes audio sequentially (local mode)
- **Bridge mode audio** — captured via WebRTC streams, NOT BrowserAudioService
- **vexa-bot build** — MUST use `npm run build` (not just `tsc`) to generate `browser-utils.global.js`

## Debugging

```bash
# Verbose logs
RUST_LOG=jarvis=debug ./jarvis/target/debug/jarvis

# Web UI
open http://localhost:8080

# Check vexa-bot build
cd services/vexa-bot/core && npx tsc --noEmit

# View session files
ls ~/Library/Application\ Support/jarvis/sessions/
```
