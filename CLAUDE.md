# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A unified all-in-one AI meeting assistant. A Go service connects to the vendored Vexa platform via WebSocket, listens for live meeting transcripts, detects a trigger phrase, generates responses with OpenAI GPT-4o, and speaks back via TTS. Includes a web UI dashboard on port 8080. The Vexa transcription platform (Python services, PostgreSQL, Redis, MinIO) is vendored into `services/` and `libs/` -- everything starts with a single `docker compose up`.

## Build & Run

```bash
# First time: build the vexa-bot image (bot-manager launches it dynamically)
docker compose build vexa-bot

# Start everything (~12 containers)
docker compose up -d

# View meet-bot logs
docker compose logs -f meet-bot

# View all logs
docker compose logs -f

# Stop
docker compose down

# Stop and remove all data (forces re-setup on next start)
docker compose down -v

# Rebuild after code changes
docker compose up --build -d
```

**Init-setup** (`scripts/init-setup.sh`) runs as a one-shot container on first boot: waits for admin-api, creates a Vexa user + API key, saves the key to `data/shared/api-key`, and optionally launches a bot into the meeting if `MEET_URL` is set.

**Cannot run Vexa natively on macOS** -- requires the full Docker stack. The meet-bot binary itself is pure Go (CGO_ENABLED=0) but needs a running Vexa instance to connect to.

### Local Go checks

```bash
# All packages compile locally (no CGO deps):
go build ./...

# Tests
go test ./internal/config/... ./internal/llm/...

# Single test
go test ./internal/llm/ -run TestShouldRespond
```

## Architecture

```
cmd/bot/main.go           Entry point — loads config, starts web UI + bot loop
internal/
  config/config.go        Config loading: env vars > config.json > defaults
                            MEET_URL parsing (Google Meet, Teams, Zoom)
                            File-based Vexa API key from /shared/api-key
                            Hot-reloadable fields via ApplyHotReload()
                            Persistent config.json in /shared/ for web UI changes
  bot/bot.go              Orchestrator — connectWithRetry → event loop:
                            transcript.mutable → trigger check → LLM → /speak
                            Auto-reconnect on WS errors, backoff on missing meetings
                            Broadcasts transcripts to web UI via TranscriptBroadcaster
  vexa/client.go          REST client: GetTranscript, Speak, StopSpeaking
  vexa/ws.go              WebSocket client: subscribe, readLoop, pingLoop, reconnect
                            Events: transcript.mutable, speak.completed, meeting.status
  llm/openai.go           OpenAI GPT-4o: Respond, Summary
                            ShouldRespond uses GPT-4o-mini for smart intent detection
                            No keyword matching -- LLM classifies if speaker addresses bot
                            UpdateSettings() for hot-reload of trigger/name/prompt
  web/server.go           HTTP server on :8080 with go:embed index.html
                            Endpoints: /api/config, /api/status, /api/launch, /api/stop
                            WebSocket: /api/transcript (live transcript broadcast)
  web/index.html          Single-page dashboard (vanilla HTML/CSS/JS, dark theme)
                            Settings, system prompt editor, bot controls, live transcript

services/                 Vendored Vexa platform services:
  admin-api/                User/token management
  api-gateway/              REST + WebSocket gateway
  bot-manager/              Launches vexa-bot containers via Docker socket
  tts-service/              OpenAI TTS proxy
  transcription-service/    Whisper model serving
  transcription-collector/  Aggregates transcript segments
  WhisperLive/              Real-time audio → transcription
  vexa-bot/                 Chromium bot that joins meetings (built separately)
  mcp/                      MCP service

libs/shared-models/       Shared DB models and Alembic migrations
scripts/init-setup.sh     First-boot auto-setup (user + API key + bot launch)
data/shared/              Bind-mounted volume for init-setup ↔ meet-bot communication
docker-compose.yml        Full stack: infra + Vexa + init-setup + meet-bot
Dockerfile                Two-stage: Go builder → distroless runtime
```

### Data Flow

```
Vexa bot in Meet → captures audio → WhisperLive → transcription-collector → Redis pub/sub
    → api-gateway WebSocket → meet-bot (Go service)
    → trigger detected → OpenAI GPT-4o → response text
    → POST /speak → Vexa TTS → audio played in Meet
    → transcript broadcast → web UI WebSocket → browser dashboard
```

### Web UI (port 8080)

API endpoints:
- `GET /api/config` — current config as JSON
- `POST /api/config` — update config (hot-reloads trigger, name, voice, model, prompt)
- `GET /api/status` — Vexa health, meeting info, bot status
- `POST /api/launch` — launch a Vexa bot into a meeting
- `POST /api/stop` — stop the Vexa bot
- `WS /api/transcript` — live transcript WebSocket stream

### Key Integration Points

- **Vexa WebSocket** (`/ws`): Subscribe with `{"action":"subscribe","meetings":[{"platform":"google_meet","native_id":"..."}]}`. Auth via `X-API-Key` header.
- **Vexa REST**: `GET /transcripts/{platform}/{id}` for bootstrap, `POST /bots/{platform}/{id}/speak` for TTS
- **Vexa Admin API** (internal, port 8057): Used by init-setup.sh only. `POST /admin/users` and `POST /admin/users/{id}/tokens`.
- **vexa-bot image**: Must be pre-built with `docker compose build vexa-bot`. Bot-manager launches containers from this image dynamically via Docker socket.

## Key Constraints

- **Intent detection** uses GPT-4o-mini to classify if a transcript line is addressed to the bot — no exact trigger phrase needed, robust against Whisper transcription errors
- **Language constraint** — system prompt enforces English/Ukrainian only, never Russian
- **`speaking` uses `sync/atomic.Bool`** to prevent overlapping TTS while bot is talking
- **Agent struct uses `sync.Mutex`** for transcript/history access from multiple goroutines
- **Segment dedup** by `absolute_start_time` field — mutable segments get updated in place
- **Auto-reconnect** on WS disconnect with exponential backoff (5s-30s)
- **Config hot-reload** — trigger phrase, bot name, TTS voice, model, system prompt can change without restart via web UI
- **Config persistence** — web UI changes saved to `/shared/config.json`, survives container restarts
- **vexa-bot image** must be pre-built before `docker compose up` — bot-manager references it by name `vexa-bot:dev`

## Configuration

All config via environment variables (`.env` file) or web UI (persisted to `config.json`).

Required: `OPENAI_API_KEY`

Optional: `MEET_URL` (can be set via web UI), `TRIGGER_PHRASE` (hey bot), `BOT_DISPLAY_NAME` (Jarvis), `OPENAI_MODEL` (gpt-4o), `TTS_VOICE` (nova), `TTS_PROVIDER` (openai), `WEB_UI_PORT` (8080), `SUMMARY_INTERVAL` (10m)

Config priority: environment variable > config.json > default value.

Vexa API key is auto-generated by init-setup.sh and stored at `data/shared/api-key`. No manual Vexa configuration needed.

## Debugging

```bash
# Meet-bot logs (Go service + web UI)
docker compose logs -f meet-bot

# Init-setup (first boot only)
docker compose logs init-setup

# Vexa bot joining Meet
docker logs $(docker ps --filter "name=vexa-bot" -q --latest)

# WhisperLive transcription
docker compose logs -f whisperlive-remote

# All services
docker compose ps

# Vexa API health
curl http://localhost:8056/health

# Redis pub/sub channels (should show tc:meeting:N:mutable)
docker compose exec redis redis-cli PUBSUB CHANNELS '*'
```
