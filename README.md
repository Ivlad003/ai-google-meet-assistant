# Jarvis — AI Google Meet Assistant

A desktop AI assistant that joins Google Meet, listens to the conversation, and responds to voice commands via GPT-5.4. Records audio, transcribes with speaker names, and saves session files. Runs natively on macOS/Linux.

---

# UA: Jarvis — AI Асистент для Google Meet

Десктопний AI асистент, який приєднується до Google Meet, слухає розмову та відповідає на голосові команди через GPT-5.4. Записує аудіо, транскрибує з іменами спікерів та зберігає файли сесій. Працює нативно на macOS/Linux.

---

## Architecture / Архітектура

```
┌─────────────────────────────────────────────────────┐
│  Jarvis (Rust)                                      │
│  ┌──────────┐  ┌─────────┐  ┌───────┐  ┌────────┐  │
│  │ Web UI   │  │ Whisper  │  │  LLM  │  │  TTS   │  │
│  │ :8080    │  │ (cloud/  │  │GPT5.4 │  │ OpenAI │  │
│  │          │  │  local)  │  │       │  │        │  │
│  └────┬─────┘  └────┬────┘  └───┬───┘  └───┬────┘  │
│       └─────────────┴─────┬─────┴───────────┘       │
│                     WebSocket Bridge (:9090)         │
│              ┌──────────────────────────┐            │
│              │ Session Recording        │            │
│              │ WAV audio + transcript   │            │
│              │ + daily log rotation     │            │
│              └──────────────────────────┘            │
└─────────────────────────┬───────────────────────────┘
                          │
┌─────────────────────────┴───────────────────────────┐
│  vexa-bot (TypeScript/Playwright)                    │
│  ┌────────────┐  ┌──────────────┐  ┌──────────────┐ │
│  │ Chromium   │  │ WebRTC Audio │  │ Speaker      │ │
│  │ Browser    │──│ Capture      │──│ Detection    │ │
│  └────────────┘  └──────────────┘  └──────────────┘ │
└─────────────────────────┬───────────────────────────┘
                          │
                   ┌──────┴──────┐
                   │ Google Meet  │
                   └─────────────┘
```

### How it works / Як це працює

**EN:**
1. Jarvis launches a Chromium browser via vexa-bot (Playwright)
2. The bot joins your Google Meet meeting as a participant
3. Audio is captured directly from WebRTC streams (no microphone needed)
4. All audio is recorded to a WAV file for the session
5. Speech is transcribed via OpenAI Whisper API or local whisper-rs
6. Speaker names are detected from meeting participant tiles
7. Smart intent detection (GPT-5) recognizes when someone addresses the bot
8. GPT-5.4 generates a response, OpenAI TTS synthesizes speech
9. The response is spoken back in the meeting via WebRTC
10. Live transcript and controls available in the Web UI at http://localhost:8080
11. On shutdown, session transcript and audio file paths are printed to terminal

**UA:**
1. Jarvis запускає Chromium браузер через vexa-bot (Playwright)
2. Бот приєднується до вашого Google Meet як учасник
3. Аудіо захоплюється напряму з WebRTC потоків (мікрофон не потрібен)
4. Все аудіо записується у WAV файл сесії
5. Мовлення транскрибується через OpenAI Whisper API або локальний whisper-rs
6. Імена спікерів визначаються з тайлів учасників мітингу
7. Розумне визначення наміру (GPT-5) розпізнає, коли хтось звертається до бота
8. GPT-5.4 генерує відповідь, OpenAI TTS синтезує мовлення
9. Відповідь озвучується в мітингу через WebRTC
10. Транскрипт та керування доступні у Web UI на http://localhost:8080
11. При завершенні шляхи до файлів сесії виводяться в термінал

## Session Files / Файли сесій

After each meeting, Jarvis saves:

```
~/Library/Application Support/jarvis/
  sessions/
    2026-03-08_111030.txt    # Transcript with speaker names
    2026-03-08_111030.wav    # Full audio recording
  logs/
    jarvis.log.2026-03-08    # Application logs
```

Transcript format:
```
[14:05:12] [Alice] Let's discuss the Q3 roadmap priorities.
[14:05:18] [Bob] I think we should focus on the API redesign first.
[14:05:24] [Alice] Jarvis, summarize the main points so far.
[14:05:30] [Jarvis] So far you've discussed Q3 priorities, with a focus on API redesign.
```

## Quick Start: Docker (Recommended) / Швидкий старт: Docker

The easiest way to run Jarvis — no Rust/Node.js installation needed.

### Prerequisites / Вимоги

- **Docker** + **Docker Compose**
- **OpenAI API key**

### 1. Clone and configure / Клонувати та налаштувати

```bash
git clone https://github.com/Ivlad003/ai-google-meet-assistant.git
cd ai-google-meet-assistant
cp jarvis.config.example.json jarvis.config.json
cp .env.example .env
```

Edit `.env` — set your `OPENAI_API_KEY` and optionally `MEET_URL`:

```bash
OPENAI_API_KEY=sk-proj-your-key-here
MEET_URL=https://meet.google.com/abc-defg-hij
```

### 2. Run / Запустити

```bash
docker compose up -d
```

Open http://localhost:8080 — you'll be prompted for basic auth (default: `admin` / password you set).

### 3. Admit the bot / Впустити бота

When the bot requests to join your Google Meet, click "Admit" in the meeting lobby.

### 4. Stop / Зупинити

```bash
docker compose down
```

Session files (transcripts, audio) are persisted in the `jarvis-data` Docker volume.

---

## Setting Up Authentication / Налаштування аутентифікації

All access goes through Caddy with HTTP basic auth. Generate a password hash:

```bash
docker run --rm caddy:2-alpine caddy hash-password --plaintext 'your-password'
```

Add to `.env` — **escape every `$` as `$$`** (docker-compose requirement):

```bash
CADDY_AUTH_USER=admin
# Original hash:  $2a$14$abc123...
# Escaped for .env: $$2a$$14$$abc123...
CADDY_AUTH_HASH=$$2a$$14$$your-escaped-hash-here
```

Restart: `docker compose down && docker compose up -d`

---

## VPS Deployment with HTTPS / Розгортання на VPS з HTTPS

For production deployment on a VPS with automatic HTTPS via Let's Encrypt:

### 1. Configure domain

Edit `Caddyfile` — replace `:8080` with your domain name:

```
your-domain.com {
    basic_auth {
        {$CADDY_AUTH_USER:admin} {$CADDY_AUTH_HASH}
    }
    reverse_proxy jarvis:8080
}
```

### 2. Start with Caddy

```bash
docker compose up -d
```

Caddy auto-obtains HTTPS certificates. Your bot is accessible at `https://your-domain.com` with basic auth.

### Docker Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `OPENAI_API_KEY` | **Yes** | OpenAI API key (overrides config file) |
| `MEET_URL` | No | Default meeting URL |
| `BOT_NAME` | No | Bot display name |
| `LANGUAGE` | No | Transcription language (auto/en/uk) |
| `TTS_VOICE` | No | OpenAI TTS voice |
| `RESPONSE_MODE` | No | `smart` or `name_only` |
| `RUST_LOG` | No | Log level (default: `jarvis=info`) |
| `CADDY_AUTH_USER` | No | Basic auth username (default: admin) |
| `CADDY_AUTH_HASH` | No | Basic auth password hash |

---

## Quick Start: Native Build / Швидкий старт: Нативна збірка

For development or if you prefer running without Docker.

### Prerequisites / Вимоги

- **Node.js** (v18+)
- **Rust** toolchain
- **OpenAI API key**

### 1. Clone and configure / Клонувати та налаштувати

```bash
git clone https://github.com/Ivlad003/ai-google-meet-assistant.git
cd ai-google-meet-assistant
cp jarvis.config.example.json jarvis.config.json
```

Edit `jarvis.config.json` — set your `openai_key` and optionally `meet_url`.

### 2. Build / Зібрати

```bash
# Build Jarvis (Rust)
cd jarvis && cargo build && cd ..

# Build vexa-bot (TypeScript + browser utils bundle)
cd services/vexa-bot/core && npm install && npm run build && cd ../../..
```

### 3. Run / Запустити

```bash
./jarvis/target/debug/jarvis
```

By default it loads `jarvis.config.json` from the current directory. To use a different path:

```bash
./jarvis/target/debug/jarvis --config /path/to/config.json
```

Open http://localhost:8080, enter a meeting URL, and click "Launch Bot".

### 4. Admit the bot / Впустити бота

When the bot requests to join your Google Meet, click "Admit" in the meeting lobby.

### 5. After the meeting / Після мітингу

Press Ctrl+C to stop. Jarvis will print the session file paths:

```
=== Session Complete ===
Transcript: /Users/you/Library/Application Support/jarvis/sessions/2026-03-08_111030.txt
Audio:      /Users/you/Library/Application Support/jarvis/sessions/2026-03-08_111030.wav
Logs:       /Users/you/Library/Application Support/jarvis/logs
========================
```

## Usage / Використання

Talk to the bot naturally — no exact trigger phrase needed:

```
"Jarvis, what are the main risks of this approach?"
"Jarvis, summarize what we discussed so far"
"Jarvis, translate 'deployment pipeline' to Ukrainian"
```

**UA:**
```
"Джарвіс, які основні ризики цього підходу?"
"Джарвіс, підсумуй, що ми обговорили"
```

### Language Support / Підтримка мов

Set `language` in `jarvis.config.json`:

```json
{ "language": "uk" }
```

Values: `"auto"` (default), `"en"`, `"uk"`.

## Configuration / Конфігурація

All settings are in `jarvis.config.json`. See `jarvis.config.example.json` for the full template.

| Field | Required | Default | Description |
|---|---|---|---|
| `openai_key` | **Yes** | — | OpenAI API key |
| `meet_url` | No | — | Meeting URL (can set via Web UI) |
| `bot_name` | No | `Jarvis` | Bot display name in meeting |
| `language` | No | `auto` | Transcription language: `auto`, `en`, `uk` |
| `openai_model` | No | `gpt-5.4` | LLM model for responses |
| `intent_model` | No | `gpt-5` | Model for intent detection |
| `tts_voice` | No | `nova` | TTS voice (alloy, echo, fable, onyx, nova, shimmer) |
| `transcription_mode` | No | `cloud` | `cloud` (OpenAI API) or `local` (whisper-rs) |
| `whisper_model` | No | `small` | Local whisper model (small, medium, large) |
| `port` | No | `8080` | Web UI port |
| `bridge_port` | No | `9090` | Bridge WebSocket port |
| `max_response_tokens` | No | `150` | Max tokens in response |
| `temperature` | No | `0.7` | LLM temperature |
| `system_prompt` | No | built-in | Custom system prompt |
| `intent_prompt` | No | built-in | Custom intent prompt (`{bot_name}`, `{context}`, `{speaker}`, `{text}`) |
| `tools` | No | `[]` | Custom tool integrations (curl, claude-code) |

## Debugging / Дебаг

```bash
# Native: Verbose logs
RUST_LOG=jarvis=debug ./jarvis/target/debug/jarvis

# Docker: View logs
docker logs -f jarvis

# Docker: Verbose logs
RUST_LOG=jarvis=debug docker compose up -d jarvis

# Check vexa-bot build
cd services/vexa-bot/core && npx tsc --noEmit

# View session files (native)
ls ~/Library/Application\ Support/jarvis/sessions/

# View session files (Docker)
docker exec jarvis ls /data/jarvis/sessions/
```

| Problem | Solution |
|---|---|
| Bot not joining | Check `meet_url` in config. Look at logs for `[vexa-bot]` errors |
| "you you you" in transcript | Silence filter should catch this. Ensure participants are speaking |
| Bot timeout | Click "Admit" in Google Meet lobby |
| Bad transcription | Set `"language": "en"` or `"language": "uk"` instead of `"auto"` in config |
| Bot can't find buttons | Google Meet UI language must match selectors. English and Ukrainian are supported |
| OpenAI API errors | Check `openai_key` in config is valid |
| `post_join_setup_error` | Run `npm run build` (not just `tsc`) in services/vexa-bot |

## Tech Stack

- **Rust** — Jarvis core app (Axum, hound, tracing-appender, tokio)
- **TypeScript/Playwright** — vexa-bot browser automation with stealth plugin
- **OpenAI GPT-5.4** — LLM responses + GPT-5 for intent detection (reasoning_effort=minimal)
- **OpenAI Whisper** — speech-to-text (cloud or local via whisper-rs)
- **OpenAI TTS** — text-to-speech
- **WebRTC** — audio capture and playback
- **Docker** — containerized deployment with Xvfb + PulseAudio for headless WebRTC
- **Caddy** — reverse proxy with automatic HTTPS

## License

MIT
