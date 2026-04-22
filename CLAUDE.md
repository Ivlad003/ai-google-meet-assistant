# CLAUDE.md

This file provides guidance to Claude Code when working with code in this repository.

## What This Is

A desktop AI meeting assistant. **Jarvis** (Rust) is the core app — it runs a local web UI, manages a **vexa-bot** (TypeScript/Playwright) child process that joins Google Meet as a virtual participant, captures audio via WebRTC, transcribes speech (OpenAI Whisper API or local whisper-rs), detects intent via LLM, and speaks back through OpenAI TTS. Records full audio and saves timestamped transcripts with speaker names.

## Project Structure

```
jarvis/                    Rust desktop app
  src/
    main.rs                Entry point — config loading, wiring, audio processing loop, WAV recording
    config.rs              Config from JSON file with defaults
    server.rs              Axum web server — Web UI, REST API, WebSocket
    bot_bridge.rs          WebSocket bridge to vexa-bot (audio + commands + speaker tracking)
    process.rs             Manages vexa-bot child process (start/stop/find)
    transcription/
      mod.rs               Trait + silence detection + hallucination filter
      cloud.rs             OpenAI Whisper API transcriber
      local.rs             whisper-rs (local) transcriber
    llm.rs                 OpenAI chat — intent detection + response generation
    tts.rs                 OpenAI TTS synthesis
    tools.rs               Custom tool integrations (curl, etc.)
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

Dockerfile                 Multi-stage build (Rust + TS + Playwright runtime)
docker-compose.yml         Jarvis + Caddy reverse proxy
docker/entrypoint.sh       Xvfb + PulseAudio + config overrides + gosu
Caddyfile                  HTTPS reverse proxy with basic auth
.env.example               Environment variables template
```

## Build & Run

### Docker (recommended for deployment)

```bash
cp jarvis.config.example.json jarvis.config.json  # edit with your API key
cp .env.example .env                               # set OPENAI_API_KEY
docker compose up -d jarvis                        # local testing
docker compose up -d                               # with Caddy HTTPS (VPS)
```

### Native (development)

```bash
# Build Jarvis
cd jarvis && cargo build

# Build vexa-bot TypeScript (MUST use npm run build, not just tsc)
cd services/vexa-bot/core && npm install && npm run build

# Configure (from project root)
cp jarvis.config.example.json jarvis.config.json  # edit with your API key

# Run (loads ./jarvis.config.json by default)
./jarvis/target/debug/jarvis
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

For bridge mode flags, technical implementation details, session output files, and full configuration reference, see @docs/technical-details.md

## Conventions

### Rust (Jarvis)
- Error handling: `anyhow::Result` with `.context()` for meaningful errors
- Logging: `tracing` macros (`info!`, `debug!`, `warn!`, `error!`)
- Async runtime: Tokio with `#[tokio::main]`
- HTTP client: `reqwest`
- Audio: `hound` crate for WAV I/O
- Config: `serde_json` deserialization into typed structs

### TypeScript (vexa-bot)
- Build: always `npm run build` (esbuild bundles `browser-utils.global.js`)
- DOM selectors: `aria-label`, `data-*`, text-based — never `jsname`
- Browser automation: `playwright-extra` with stealth plugin
- Languages in UI matching: English and Ukrainian only, never Russian

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
