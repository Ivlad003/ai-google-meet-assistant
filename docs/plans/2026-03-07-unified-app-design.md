# Unified App Design: Vexa + AI Meet Assistant

**Date:** 2026-03-07
**Goal:** Clone repo → set OPENAI_API_KEY + MEET_URL → `docker compose up` → bot joins meeting and responds to voice commands.

## Repository Structure

```
ai_google_meet_assistant/
├── .env.example              # OPENAI_API_KEY + MEET_URL only
├── .gitignore                # Exclude model files, volumes, .env
├── docker-compose.yml        # Single compose for all services
├── scripts/
│   └── init-setup.sh         # Init container: create user+key, launch bot
├── cmd/bot/main.go           # Meet-bot entry point (updated: adds HTTP server)
├── internal/                 # Meet-bot Go code
│   ├── bot/bot.go
│   ├── config/config.go      # Updated: MEET_URL parser, file-based API key, config.json
│   ├── llm/openai.go         # Updated: hot-reloadable system prompt
│   ├── vexa/
│   └── web/
│       ├── server.go         # HTTP server + API handlers + WS transcript stream
│       └── index.html        # Single-page dashboard (embedded via go:embed)
├── Dockerfile                # Meet-bot Dockerfile
├── services/                 # Vexa services (vendored verbatim)
│   ├── admin-api/
│   ├── api-gateway/
│   ├── bot-manager/
│   ├── transcription-collector/
│   ├── transcription-service/
│   ├── tts-service/
│   ├── vexa-bot/             # With join button fix applied
│   └── WhisperLive/
├── libs/                     # Vexa shared libs (vendored)
│   └── shared-models/
├── migrations/               # Vexa DB migrations
└── README.md                 # Bilingual EN/UA docs
```

## Auto-Setup Flow (init-setup container)

```
1. Wait for postgres healthy
2. Wait for admin-api healthy (retry loop, max 60s)
3. Check marker file /shared/.setup-done
   → If exists, skip to step 7
4. POST /admin/users → create bot@local.ai user
5. POST /admin/users/{id}/tokens → get API token
6. Write token to /shared/api-key + create /shared/.setup-done
7. Parse MEET_URL → extract platform + meeting_id
8. Wait for api-gateway healthy
9. POST /bots → launch vexa-bot into meeting
10. Exit 0
```

## Docker Compose Services

```yaml
# Infrastructure
postgres          # DB with healthcheck
redis             # Pub/sub
minio + minio-init # Object storage

# Vexa Core
admin-api         # User/token management (healthcheck added)
api-gateway       # REST + WebSocket proxy
bot-manager       # Launches vexa-bot containers
transcription-collector
tts-service
whisperlive-remote

# Bootstrap
init-setup        # One-shot, depends_on admin-api+postgres healthy

# AI Assistant
meet-bot          # depends_on init-setup completed_successfully
                  # exposes port 8080 for web UI
```

## Shared Volume

`setup-data` mounted at `/shared`:
- init-setup (rw): writes `/shared/api-key`, `/shared/.setup-done`, reads config.json
- meet-bot (rw): reads `/shared/api-key`, reads/writes `/shared/config.json`

## Web UI

Single `index.html` embedded in meet-bot binary via `go:embed`, served at `http://localhost:8080`.

### Layout

```
┌─────────────────────────────────────────────────┐
│  AI Meet Assistant                    [status]   │
├──────────────────────┬──────────────────────────┤
│  Settings            │  System Prompt           │
│                      │                          │
│  Meeting URL [____]  │  [                    ]  │
│  OpenAI Key  [____]  │  [  textarea with     ]  │
│  Trigger     [____]  │  [  LLM instructions  ]  │
│  Bot Name    [____]  │  [                    ]  │
│  TTS Voice   [▼___]  │  [                    ]  │
│  Model       [▼___]  │                          │
│                      │  [Save Prompt]           │
│  [Save & Reload]     │                          │
├──────────────────────┤                          │
│  Meeting Bot:        │                          │
│  [Launch Bot] [Stop] │                          │
│                      │                          │
│  Bot: ● Connected    │                          │
│  Meeting: active     │                          │
│  Vexa: ● healthy     │  (read-only status)      │
├──────────────────────┴──────────────────────────┤
│  Live Transcript                                │
│  ┌─────────────────────────────────────────────┐│
│  │ 10:32 John: Hey bot, what is two plus two?  ││
│  │ 10:32 Bot: Two plus two equals four.        ││
│  │ 10:35 John: Thanks!                         ││
│  └─────────────────────────────────────────────┘│
└─────────────────────────────────────────────────┘
```

### API Endpoints (meet-bot HTTP server, port 8080)

| Endpoint | Method | Description |
|---|---|---|
| `/` | GET | Serve index.html |
| `/api/config` | GET | Return current config.json |
| `/api/config` | POST | Save config.json, hot-reload applicable settings |
| `/api/status` | GET | Bot state + Vexa health (read-only) |
| `/api/launch` | POST | Call Vexa POST /bots to launch into meeting |
| `/api/stop` | POST | Call Vexa DELETE /bots to leave meeting |
| `/api/transcript` | WebSocket | Stream live transcript events to browser |

### Hot-reload vs Restart

- **Immediate** (no restart): system prompt, trigger phrase, bot name, TTS voice, model
- **Restart required**: meeting URL, OpenAI API key (UI shows indicator)

### Config File (`/shared/config.json`)

```json
{
  "meet_url": "https://meet.google.com/abc-defg-hij",
  "openai_api_key": "sk-...",
  "openai_model": "gpt-4o",
  "trigger_phrase": "hey bot",
  "bot_display_name": "AI Assistant",
  "tts_voice": "nova",
  "system_prompt": "You are an AI meeting assistant..."
}
```

Written by UI via POST /api/config. Read by meet-bot on startup and on reload.
Init-setup reads meet_url from this file if it exists (fallback to MEET_URL env var).

### Vexa Status (read-only)

UI polls `/api/status` every 5 seconds. Meet-bot pings api-gateway health endpoint internally. Shows green/red dot. No start/stop control — Vexa lifecycle managed via `docker compose up/down` in terminal.

## Config Changes (config.go)

New MEET_URL parsing:
```
meet.google.com/abc-defg-hij        → google_meet / abc-defg-hij
teams.microsoft.com/l/meetup-join/… → msteams / <path>
zoom.us/j/12345678                  → zoom / 12345678
us05web.zoom.us/j/12345678          → zoom / 12345678
```

Config resolution order:
1. `/shared/config.json` (highest priority, written by UI)
2. Environment variables (MEET_URL, VEXA_API_KEY, etc.)
3. `/shared/api-key` file (auto-generated key)
4. Hardcoded defaults (api-gateway:8000, etc.)

## .env.example

```bash
OPENAI_API_KEY=sk-...
MEET_URL=https://meet.google.com/abc-defg-hij

# Optional
# TRIGGER_PHRASE=hey bot
# BOT_DISPLAY_NAME=AI Assistant
# TTS_VOICE=nova
```

## What Stays Unchanged

- All Vexa service code (verbatim copy, except vexa-bot join.ts fix)
- Meet-bot core: bot.go, ws.go, client.go (logic unchanged)
- All Vexa Dockerfiles
- Database migrations

## New/Changed Files

| File | Change | ~Lines |
|---|---|---|
| docker-compose.yml | Merged single file | ~200 |
| scripts/init-setup.sh | Auto-setup script | ~60 |
| internal/config/config.go | URL parser + file key + config.json | ~50 |
| internal/llm/openai.go | Hot-reloadable system prompt | ~15 |
| internal/web/server.go | HTTP server + API + WS handler | ~150 |
| internal/web/index.html | Single-page dashboard | ~250 |
| cmd/bot/main.go | Start HTTP server alongside bot | ~10 |
| .env.example | Simplified | ~8 |
| .gitignore | Model files, volumes | ~10 |
| README.md | Updated instructions | ~100 |

Total: ~850 lines new/changed code.
