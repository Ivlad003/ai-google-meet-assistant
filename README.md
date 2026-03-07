# Jarvis — AI Google Meet Assistant

A desktop AI assistant that joins Google Meet, listens to the conversation, and responds to voice commands via GPT-4o. Runs natively on macOS/Linux.

---

# UA: Jarvis — AI Асистент для Google Meet

Десктопний AI асистент, який приєднується до Google Meet, слухає розмову та відповідає на голосові команди через GPT-4o. Працює нативно на macOS/Linux.

---

## Architecture / Архітектура

```
┌─────────────────────────────────────────────────────┐
│  Jarvis (Rust)                                      │
│  ┌──────────┐  ┌─────────┐  ┌───────┐  ┌────────┐  │
│  │ Web UI   │  │ Whisper  │  │  LLM  │  │  TTS   │  │
│  │ :8080    │  │ (cloud/  │  │GPT-4o │  │ OpenAI │  │
│  │          │  │  local)  │  │       │  │        │  │
│  └────┬─────┘  └────┬────┘  └───┬───┘  └───┬────┘  │
│       └─────────────┴─────┬─────┴───────────┘       │
│                     WebSocket Bridge (:9090)         │
└─────────────────────────┬───────────────────────────┘
                          │
┌─────────────────────────┴───────────────────────────┐
│  vexa-bot (TypeScript/Playwright)                    │
│  ┌────────────┐  ┌──────────────┐  ┌──────────────┐ │
│  │ Chromium   │  │ WebRTC Audio │  │ Bridge       │ │
│  │ Browser    │──│ Capture      │──│ Client       │ │
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
4. Speech is transcribed via OpenAI Whisper API or local whisper-rs
5. Smart intent detection (GPT-4o-mini) recognizes when someone addresses the bot
6. GPT-4o generates a response, OpenAI TTS synthesizes speech
7. The response is spoken back in the meeting via WebRTC
8. Live transcript and controls available in the Web UI at http://localhost:8080

**UA:**
1. Jarvis запускає Chromium браузер через vexa-bot (Playwright)
2. Бот приєднується до вашого Google Meet як учасник
3. Аудіо захоплюється напряму з WebRTC потоків (мікрофон не потрібен)
4. Мовлення транскрибується через OpenAI Whisper API або локальний whisper-rs
5. Розумне визначення наміру (GPT-4o-mini) розпізнає, коли хтось звертається до бота
6. GPT-4o генерує відповідь, OpenAI TTS синтезує мовлення
7. Відповідь озвучується в мітингу через WebRTC
8. Транскрипт та керування доступні у Web UI на http://localhost:8080

## Quick Start / Швидкий старт

### Prerequisites / Вимоги

- **Node.js** (v18+)
- **Rust** toolchain
- **OpenAI API key**

### 1. Clone and configure / Клонувати та налаштувати

```bash
git clone https://github.com/aspect-build/ai_google_meet_asistant.git
cd ai_google_meet_asistant
cp .env.example .env
```

Edit `.env`:

```bash
OPENAI_API_KEY=sk-...
```

### 2. Build / Зібрати

```bash
# Build Jarvis (Rust)
cd jarvis && cargo build && cd ..

# Build vexa-bot (TypeScript)
cd services/vexa-bot/core && npx tsc && cd ../../..
```

### 3. Run / Запустити

```bash
./jarvis/target/debug/jarvis
```

Open http://localhost:8080, enter a meeting URL, and click "Launch Bot".

Or start with a meeting URL directly:

```bash
MEET_URL=https://meet.google.com/abc-defg-hij ./jarvis/target/debug/jarvis
```

### 4. Admit the bot / Впустити бота

When the bot requests to join your Google Meet, click "Admit" in the meeting lobby.

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

```bash
LANGUAGE=auto ./jarvis/target/debug/jarvis   # auto-detect (default)
LANGUAGE=uk ./jarvis/target/debug/jarvis     # Ukrainian
LANGUAGE=en ./jarvis/target/debug/jarvis     # English
```

## Environment Variables / Змінні середовища

| Variable | Required | Default | Description |
|---|---|---|---|
| `OPENAI_API_KEY` | Yes | — | OpenAI API key |
| `MEET_URL` | No | — | Meeting URL (can set via Web UI) |
| `BOT_DISPLAY_NAME` | No | `Jarvis` | Bot name in meeting |
| `TRIGGER_PHRASE` | No | `hey bot` | Hint for intent detection |
| `LANGUAGE` | No | `auto` | Transcription language: `auto`, `en`, `uk` |
| `TRANSCRIPTION_MODE` | No | `cloud` | `cloud` (OpenAI API) or `local` (whisper-rs) |
| `WHISPER_MODEL` | No | `small` | Local whisper model (small, medium, large) |
| `OPENAI_MODEL` | No | `gpt-4o` | LLM model for responses |
| `TTS_VOICE` | No | `nova` | TTS voice (alloy, echo, fable, onyx, nova, shimmer) |
| `WEB_UI_PORT` | No | `8080` | Web UI port |

## Debugging / Дебаг

```bash
# Verbose logs
RUST_LOG=jarvis=debug ./jarvis/target/debug/jarvis

# Check vexa-bot build
cd services/vexa-bot/core && npx tsc --noEmit
```

| Problem | Solution |
|---|---|
| Bot not joining | Check MEET_URL. Look at logs for `[vexa-bot]` errors |
| "you you you" in transcript | Silence filter should catch this. Ensure participants are speaking |
| Bot timeout | Click "Admit" in Google Meet lobby |
| Bad transcription | Set `LANGUAGE=en` or `LANGUAGE=uk` instead of `auto` |
| Bot can't find buttons | Google Meet UI language must match selectors. English and Ukrainian are supported |
| OpenAI API errors | Check `OPENAI_API_KEY` is valid. Errors include HTTP status and response body |

## Tech Stack

- **Rust** — Jarvis desktop app (Axum, whisper-rs, tokio)
- **TypeScript/Playwright** — vexa-bot browser automation
- **OpenAI GPT-4o** — LLM responses + GPT-4o-mini for intent detection
- **OpenAI Whisper** — speech-to-text (cloud or local via whisper-rs)
- **OpenAI TTS** — text-to-speech
- **WebRTC** — audio capture and playback

## License

MIT
