# AI Google Meet Assistant

An all-in-one AI meeting assistant that joins Google Meet, listens to the conversation, and responds to voice commands via GPT-4o. Everything runs with a single `docker compose up` -- Vexa transcription platform is bundled and auto-configured.

---

# UA: AI Асистент для Google Meet

Комплексний AI асистент для мітингів, який приєднується до Google Meet, слухає розмову та відповідає на голосові команди через GPT-4o. Все запускається однією командою `docker compose up` -- платформа транскрипції Vexa вбудована та налаштовується автоматично.

---

## Architecture / Архітектура

```
                        docker compose up
┌────────────────────────────────────────────────────────────────┐
│                                                                │
│  ┌──────────┐   ┌────────────┐   ┌──────────────────────────┐ │
│  │ postgres  │   │   redis    │   │         minio            │ │
│  └─────┬────┘   └─────┬──────┘   └──────────┬───────────────┘ │
│        │              │                      │                 │
│  ┌─────┴──────────────┴──────────────────────┴───────────────┐ │
│  │  Vexa Platform (vendored)                                 │ │
│  │  admin-api, api-gateway, bot-manager, tts-service,        │ │
│  │  transcription-service, whisperlive, transcription-collector│
│  └──────────────────────┬────────────────────────────────────┘ │
│                         │  WebSocket + REST                    │
│  ┌──────────────────────┴────────────────────────────────────┐ │
│  │  meet-bot (Go)                                            │ │
│  │  - Connects to Vexa WebSocket for live transcript         │ │
│  │  - Smart intent detection (GPT-4o-mini) → GPT-4o → TTS    │ │
│  │  - Web UI dashboard on :8080                              │ │
│  └───────────────────────────────────────────────────────────┘ │
│                                                                │
│  ┌──────────┐    ┌──────────────┐                              │
│  │init-setup│    │  vexa-bot    │ (launched dynamically        │
│  │(one-time)│    │  container   │  by bot-manager into Meet)   │
│  └──────────┘    └──────────────┘                              │
│                                                                │
└────────────────────────────────────────────────────────────────┘
         │                                    │
         ▼                                    ▼
   ┌──────────┐                        ┌─────────────┐
   │ OpenAI   │                        │ Google Meet  │
   │ GPT-4o   │                        │             │
   └──────────┘                        └─────────────┘
```

### How it works / Як це працює

**EN:**
1. `docker compose up` starts the entire stack: Vexa platform + meet-bot + infrastructure
2. **init-setup** (runs once) auto-creates a Vexa user and API key -- no manual steps needed
3. **vexa-bot** joins Google Meet, captures audio, transcribes speech in real-time via WhisperLive
4. **meet-bot** connects to Vexa WebSocket and receives live transcript events
5. **Smart intent detection** (GPT-4o-mini) recognizes when someone is talking to the bot -- no exact phrase needed
6. The response is spoken back in the meeting via **OpenAI TTS**
7. Configure everything through the **Web UI** at http://localhost:8080

**UA:**
1. `docker compose up` запускає весь стек: платформа Vexa + meet-bot + інфраструктура
2. **init-setup** (одноразово) автоматично створює користувача та API ключ Vexa -- жодних ручних кроків
3. **vexa-bot** приєднується до Google Meet, захоплює аудіо, транскрибує мовлення в реальному часі через WhisperLive
4. **meet-bot** підключається до Vexa WebSocket і отримує події транскрипції в реальному часі
5. **Розумне визначення наміру** (GPT-4o-mini) розпізнає, коли хтось звертається до бота -- точна фраза не потрібна
6. Відповідь озвучується в мітингу через **OpenAI TTS**
7. Налаштовуйте все через **Web UI** на http://localhost:8080

## Quick Start / Швидкий старт

### 1. Clone and configure / Клонувати та налаштувати

```bash
git clone https://github.com/your-user/ai_google_meet_asistant.git
cd ai_google_meet_asistant
cp .env.example .env
```

Edit `.env` -- only two values required:

```bash
OPENAI_API_KEY=sk-...
MEET_URL=https://meet.google.com/abc-defg-hij
```

### 2. Build the vexa-bot image / Зібрати образ vexa-bot

The vexa-bot image must be pre-built because bot-manager launches it dynamically:

```bash
docker compose build vexa-bot
```

### 3. Start everything / Запустити все

```bash
docker compose up -d
```

This starts ~12 containers: PostgreSQL, Redis, MinIO, Vexa services, init-setup, and meet-bot.

On first boot, init-setup automatically creates a Vexa user and API key. If `MEET_URL` is set, it also launches a bot into the meeting.

### 4. Open Web UI / Відкрити Web UI

Open http://localhost:8080 in your browser.

### 5. Admit the bot / Впустити бота

When the bot requests to join your Google Meet, click "Admit" in the meeting lobby.

### Useful commands / Корисні команди

```bash
# View logs
docker compose logs -f meet-bot

# Stop everything
docker compose down

# Stop and remove all data (forces re-setup on next start)
docker compose down -v

# Rebuild after code changes
docker compose up --build -d
```

## Web UI / Веб-інтерфейс

The dashboard at **http://localhost:8080** provides:

**EN:**
- **Settings panel**: meeting URL, trigger phrase, bot name, TTS voice, OpenAI model
- **System prompt editor**: customize the bot personality and behavior
- **Bot controls**: Launch Bot / Stop Bot buttons
- **Status indicators**: Vexa health, bot connection status
- **Live transcript**: real-time WebSocket feed of meeting speech

**UA:**
- **Панель налаштувань**: URL мітингу, тригерна фраза, ім'я бота, голос TTS, модель OpenAI
- **Редактор системного промпту**: налаштування особистості та поведінки бота
- **Керування ботом**: кнопки "Запустити бота" / "Зупинити бота"
- **Індикатори статусу**: стан Vexa, з'єднання бота
- **Транскрипт в реальному часі**: WebSocket стрім мовлення з мітингу

Settings like trigger phrase, bot name, TTS voice, and system prompt can be changed on the fly without restarting. Changes to meeting URL or API key require a container restart.

## Usage / Використання

### Basic: Q&A in meetings / Базове: Питання та відповіді

Just talk to the bot naturally -- no exact trigger phrase needed. The bot uses smart intent detection (GPT-4o-mini) to recognize when someone is addressing it.

Speak in the meeting:
> "Jarvis, what are the main risks of this approach?"

The bot will answer via voice in the meeting. It responds in English or Ukrainian.

**UA:** Просто звертайтеся до бота природно -- точна тригерна фраза не потрібна. Бот використовує розумне визначення наміру (GPT-4o-mini), щоб розпізнати, коли хтось звертається до нього.

Скажіть у мітингу:
> "Джарвіс, які основні ризики цього підходу?"

Бот відповість голосом у мітингу. Відповідає англійською або українською.

### Use Cases / Варіанти використання

#### Meeting Assistant / Асистент мітингу
```
"Jarvis, summarize what we discussed so far"
"Jarvis, what action items were mentioned?"
"Jarvis, what did John say about the deadline?"
```

#### Code Review Helper / Помічник код-рев'ю
```
"Jarvis, explain the difference between mutex and channel in Go"
"Jarvis, what's the best way to handle errors in async Python?"
"Jarvis, write a regex to validate email addresses"
```

#### Architecture Discussions / Обговорення архітектури
```
"Jarvis, what are pros and cons of microservices vs monolith?"
"Jarvis, should we use PostgreSQL or MongoDB for this use case?"
"Jarvis, explain the CAP theorem in simple terms"
```

#### Brainstorming / Мозковий штурм
```
"Jarvis, suggest 5 names for our new product"
"Jarvis, what are alternatives to Redis for pub/sub?"
"Jarvis, how would you improve our onboarding flow?"
```

#### Translation / Переклад
```
"Jarvis, translate 'deployment pipeline' to Ukrainian"
"Jarvis, how do you say 'code review' in Japanese?"
```

### Customizing behavior / Налаштування поведінки

Use the Web UI system prompt editor, or set environment variables:

| Variable | Default | Description EN | Опис UA |
|---|---|---|---|
| `TRIGGER_PHRASE` | `hey bot` | Hint phrase for intent detection | Фраза-підказка для визначення наміру |
| `BOT_DISPLAY_NAME` | `Jarvis` | Bot name shown in Meet | Ім'я бота в Meet |
| `OPENAI_MODEL` | `gpt-4o` | OpenAI model for responses | Модель OpenAI для відповідей |
| `TTS_VOICE` | `nova` | OpenAI TTS voice (alloy, echo, fable, onyx, nova, shimmer) | Голос OpenAI TTS |

### Extending: Custom triggers / Розширення: Власні тригери

The bot's behavior is controlled by the system prompt. Edit it via the Web UI at http://localhost:8080 or modify the default in `internal/llm/openai.go`.

**EN:**
- Respond in a specific persona (e.g., "You are a senior Go developer")
- Only answer questions about a specific topic
- Generate code snippets when asked
- Act as a translator between meeting participants

**UA:**
- Відповідати в конкретній ролі (напр. "Ти - senior Go розробник")
- Відповідати лише на питання з конкретної теми
- Генерувати фрагменти коду на запит
- Виступати перекладачем між учасниками мітингу

## Environment Variables / Змінні середовища

| Variable | Required | Default | Description |
|---|---|---|---|
| `OPENAI_API_KEY` | Yes | -- | OpenAI API key |
| `MEET_URL` | No* | -- | Meeting URL (e.g., `https://meet.google.com/abc-defg-hij`). Can be set later via Web UI |
| `TRIGGER_PHRASE` | No | `hey bot` | Activation phrase |
| `BOT_DISPLAY_NAME` | No | `Jarvis` | Bot name in meeting |
| `OPENAI_MODEL` | No | `gpt-4o` | OpenAI model |
| `TTS_VOICE` | No | `nova` | TTS voice |
| `TTS_PROVIDER` | No | `openai` | TTS provider |
| `WEB_UI_PORT` | No | `8080` | Web UI port |
| `SUMMARY_INTERVAL` | No | `10m` | Meeting summary interval |

*`MEET_URL` is not required at startup -- the bot starts in "web UI only" mode and you can set the meeting URL through the dashboard. Supports Google Meet, Microsoft Teams, and Zoom URLs.

## Debugging / Дебаг

```bash
# Meet-bot logs (Go service + web UI)
docker compose logs -f meet-bot

# Init-setup output (first boot user/key creation)
docker compose logs init-setup

# Vexa bot container logs (the one that joins Meet)
docker logs $(docker ps --filter "name=vexa-bot" -q --latest)

# Check WhisperLive transcription
docker compose logs -f whisperlive-remote

# Check transcription collector
docker compose logs -f transcription-collector

# All services status
docker compose ps

# Check Vexa API health
curl http://localhost:8056/health

# Check Redis pub/sub channels
docker compose exec redis redis-cli PUBSUB CHANNELS '*'
```

### Common issues / Типові проблеми

| Problem | Solution EN | Рішення UA |
|---|---|---|
| `vexa-bot` image not found | Run `docker compose build vexa-bot` first | Спочатку виконайте `docker compose build vexa-bot` |
| init-setup fails | Check `docker compose logs init-setup`. Usually means admin-api not ready yet -- retry with `docker compose up -d` | Перевірте `docker compose logs init-setup`. Зазвичай admin-api ще не готовий -- спробуйте `docker compose up -d` |
| Bot not joining Meet | Check that MEET_URL is correct. Verify with `docker compose logs -f meet-bot` | Перевірте правильність MEET_URL. Дивіться `docker compose logs -f meet-bot` |
| VAD silence detected | Mic is muted or audio too quiet. Unmute mic in Google Meet | Мікрофон вимкнено або аудіо занадто тихе. Увімкніть мік в Google Meet |
| Bot timeout joining | Nobody admitted the bot. Click "Admit" in Google Meet lobby | Ніхто не впустив бота. Натисніть "Admit" у лобі Google Meet |
| Garbled transcription | Wrong language detected. Try speaking more clearly at start | Неправильна мова. Спробуйте говорити чіткіше на початку |
| Web UI not loading | Check meet-bot is running: `docker compose ps meet-bot` | Перевірте чи meet-bot працює: `docker compose ps meet-bot` |

## Tech Stack

- **Go 1.24** -- meet-bot service (pure Go, no CGO, ~10MB distroless image)
- **Vexa** -- meeting bot platform (vendored, joins Meet, captures audio, transcribes, TTS)
- **OpenAI GPT-4o** -- LLM for responses and summaries
- **OpenAI GPT-4o-mini** -- fast intent detection (is someone talking to the bot?)
- **OpenAI TTS** -- text-to-speech (via Vexa tts-service)
- **gorilla/websocket** -- WebSocket client with auto-reconnect
- **Docker Compose** -- single-command orchestration of ~12 containers
- **PostgreSQL, Redis, MinIO** -- Vexa infrastructure (auto-managed)

## Development History / Історія розробки

### Session 1: Unified App (2026-03-07)

Merged the Vexa transcription platform into a single self-contained repo. Key milestones:

1. **Vendored Vexa services** -- copied `services/`, `libs/`, `alembic.ini` from the Vexa repo
2. **Unified docker-compose.yml** -- 15 services orchestrated with health checks and dependency ordering
3. **Auto-setup script** (`scripts/init-setup.sh`) -- creates Vexa user + API key on first boot, no manual curl commands
4. **Meet-bot rewrite** -- Go service with Web UI dashboard, WebSocket transcript streaming, hot-reload config
5. **Config system** -- env vars > config.json > defaults, MEET_URL auto-parsing for Google Meet/Teams/Zoom

#### Bugs fixed during deployment:

| Bug | Root Cause | Fix |
|-----|-----------|-----|
| `audioTracks=0` in Google Meet | Google Meet doesn't create audio DOM elements like Teams | Added RTCPeerConnection hook in `join.ts` to intercept WebRTC tracks and create hidden `<audio>` elements |
| Whisper misheard trigger phrase | `base` model too small -- "hey bot" transcribed as "hey what", "hi buddy", "high board", "hey boss" | Replaced keyword matching with LLM-based intent detection (GPT-4o-mini) |
| Bot responded in Russian | Cyrillic speaker name biased GPT toward Russian | Added explicit language constraint in system prompt: English/Ukrainian only |
| transcription-service crash loop | CTranslate2 not compiled with CUDA | Set `DEVICE=cpu`, `COMPUTE_TYPE=int8`, `MODEL_SIZE=base` |
| Healthcheck 404s | `/health` endpoint doesn't exist in admin-api/api-gateway | Changed to `/docs` |
| DB tables missing | admin-api started before schema created | Added `db-init` service that runs `init_db()` first |
| WhisperLive connection refused | Wrong port in `REMOTE_TRANSCRIBER_URL` | Changed from port 80 to 8000 |
| init-setup wrong port | Used external port inside Docker network | Changed to internal port 8001 |
| User creation 422 | Invalid email TLD `.local` | Changed to `.ai` |

#### Key architectural decisions:

- **Intent detection over keyword matching** -- GPT-4o-mini classifies each transcript line (~5s, cheap) instead of brittle string matching against Whisper errors
- **RTCPeerConnection hook** -- the critical fix that enabled Google Meet audio capture by intercepting WebRTC tracks before the page loads
- **Auto-setup with marker files** -- `data/shared/.setup-done` prevents duplicate user creation on restarts
- **go:embed for Web UI** -- single binary with embedded HTML, no static file serving needed

## License

MIT
