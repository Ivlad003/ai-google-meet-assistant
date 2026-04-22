# Docker VPS Deployment Design

**Date:** 2026-04-22
**Status:** Approved

## Goal

Package Jarvis + vexa-bot into a single Docker image for headless VPS deployment with HTTPS via Caddy reverse proxy.

## Architecture

Single Docker container running:
- **Xvfb** (:99) — virtual framebuffer so Chrome runs in "headed" mode with full WebRTC support
- **PulseAudio** — virtual audio devices (tts_sink, virtual_mic) for TTS injection into meetings
- **Jarvis** (Rust binary) — HTTP API on :8080, WebSocket bridge on :9090
- **vexa-bot** (Node.js) — spawned as child process by Jarvis, launches Chromium via Playwright

Caddy sidecar container handles HTTPS with automatic Let's Encrypt certificates.

## Design Decisions

| Decision | Choice | Reason |
|----------|--------|--------|
| Container topology | Single container | Jarvis spawns vexa-bot as child process; splitting requires major refactor |
| Config management | Env vars override mounted config file | Secrets via env vars, bulk config via file; zero Rust code changes using jq in entrypoint |
| HTTPS | Caddy sidecar | Auto-cert, zero config, 2-line Caddyfile |
| Base image | mcr.microsoft.com/playwright:v1.56.0-jammy | Includes all browser deps, fonts, codecs |
| Browser mode | Headed via Xvfb (NOT headless) | Chrome headless doesn't support WebRTC audio properly |

## Multi-stage Dockerfile

1. **rust-builder** (rust:1.82-bookworm) — compiles Jarvis binary
2. **ts-builder** (node:20-bookworm) — compiles vexa-bot TypeScript + browser-utils bundle
3. **runtime** (playwright:v1.56.0-jammy) — installs Xvfb/PulseAudio/ffmpeg, copies artifacts, runs entrypoint

## Entrypoint Flow

1. Start Xvfb on :99
2. Start PulseAudio daemon
3. Create virtual audio sinks (zoom_sink, tts_sink, virtual_mic)
4. Configure ALSA to route through PulseAudio
5. Apply env var overrides to config JSON via jq
6. Exec Jarvis binary

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| OPENAI_API_KEY | Yes | Overrides openai_key in config |
| BOT_NAME | No | Overrides bot_name |
| MEET_URL | No | Default meeting URL |
| LANGUAGE | No | Transcription language |
| TTS_VOICE | No | OpenAI TTS voice |
| RESPONSE_MODE | No | smart or name_only |

## docker-compose Services

- **jarvis** — main app container, shm_size: 2g, restart: unless-stopped
- **caddy** — reverse proxy, ports 80/443, auto-HTTPS

## Volumes

- `jarvis-data:/data/jarvis` — sessions, logs, database
- `./jarvis.config.json:/etc/jarvis/config.json:ro` — config file
- `caddy-data:/data` — Caddy certificates

## Path Changes

- Jarvis data dir: `/data/jarvis/` (set via JARVIS_DATA_DIR env var or defaults to ~/.local/share/jarvis/)
- vexa-bot location: `/app/vexa-bot/` (added to find_vexa_bot_dir() search paths)
- Skip browser open in Docker (check DOCKER_MODE env var)

## Deployment

```bash
git clone <repo> && cd <repo>
cp .env.example .env          # edit with API key
cp jarvis.config.example.json jarvis.config.json  # customize
docker compose up -d
```
