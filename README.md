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
[11:11:12] [Borys Stoilovskyi] І тому, коли я включаю LED-лєнту...
[11:11:18] [Borys Stoilovskyi] на телевізорі, а коли без світла...
[11:11:24] [Vlad Kosmach] Так, раз, два, три, чотири, п'ять.
[11:11:30] [Jarvis] Привіт! Я завжди готовий допомогти.
```

## Quick Start / Швидкий старт

### Prerequisites / Вимоги

- **Node.js** (v18+)
- **Rust** toolchain
- **OpenAI API key**

### 1. Clone and configure / Клонувати та налаштувати

```bash
git clone https://github.com/aspect-build/ai_google_meet_asistant.git
cd ai_google_meet_asistant
```

**Option A: JSON config (recommended) / JSON конфіг (рекомендовано):**

```bash
cp jarvis.config.example.json jarvis.config.json
```

Edit `jarvis.config.json` — set your `openai_key` and `meet_url`.

**Option B: Environment file / Файл оточення:**

```bash
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

# Build vexa-bot (TypeScript + browser utils bundle)
cd services/vexa-bot && npm install --ignore-scripts && npm run build && cd ../..
```

### 3. Run / Запустити

```bash
# With JSON config
./jarvis/target/debug/jarvis --config jarvis.config.json

# Or with env vars
./jarvis/target/debug/jarvis

# Or with a meeting URL directly
MEET_URL=https://meet.google.com/abc-defg-hij ./jarvis/target/debug/jarvis
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

```bash
LANGUAGE=auto ./jarvis/target/debug/jarvis   # auto-detect (default)
LANGUAGE=uk ./jarvis/target/debug/jarvis     # Ukrainian
LANGUAGE=en ./jarvis/target/debug/jarvis     # English
```

## Configuration / Конфігурація

All settings can be provided via **JSON config file**, **environment variables**, or **CLI args**.
Priority: CLI args/env > JSON config > defaults.

```bash
# Recommended: use JSON config
./jarvis/target/debug/jarvis --config jarvis.config.json
```

See `jarvis.config.example.json` for all available options.

### Environment Variables / Змінні середовища

| Variable | Required | Default | Description |
|---|---|---|---|
| `OPENAI_API_KEY` | Yes* | — | OpenAI API key (*or set in JSON config) |
| `MEET_URL` | No | — | Meeting URL (can set via Web UI) |
| `BOT_DISPLAY_NAME` | No | `Jarvis` | Bot name in meeting |
| `LANGUAGE` | No | `auto` | Transcription language: `auto`, `en`, `uk` |
| `TRANSCRIPTION_MODE` | No | `cloud` | `cloud` (OpenAI API) or `local` (whisper-rs) |
| `WHISPER_MODEL` | No | `small` | Local whisper model (small, medium, large) |
| `OPENAI_MODEL` | No | `gpt-5.4` | LLM model for responses |
| `TTS_VOICE` | No | `nova` | TTS voice (alloy, echo, fable, onyx, nova, shimmer) |
| `WEB_UI_PORT` | No | `8080` | Web UI port |
| `JARVIS_CONFIG` | No | — | Path to JSON config file |

### JSON Config Only / Тільки в JSON конфігу

| Field | Default | Description |
|---|---|---|
| `intent_model` | `gpt-5` | Model for intent detection (uses reasoning_effort=minimal) |
| `system_prompt` | built-in | Custom system prompt |
| `intent_prompt` | built-in | Custom intent prompt (`{bot_name}`, `{context}`, `{speaker}`, `{text}`) |
| `max_response_tokens` | `150` | Max tokens in response |
| `temperature` | `0.7` | LLM temperature |
| `bridge_port` | `9090` | Bridge WebSocket port |

## Debugging / Дебаг

```bash
# Verbose logs
RUST_LOG=jarvis=debug ./jarvis/target/debug/jarvis

# Check vexa-bot build
cd services/vexa-bot/core && npx tsc --noEmit

# View session files
ls ~/Library/Application\ Support/jarvis/sessions/
```

| Problem | Solution |
|---|---|
| Bot not joining | Check MEET_URL. Look at logs for `[vexa-bot]` errors |
| "you you you" in transcript | Silence filter should catch this. Ensure participants are speaking |
| Bot timeout | Click "Admit" in Google Meet lobby |
| Bad transcription | Set `LANGUAGE=en` or `LANGUAGE=uk` instead of `auto` |
| Bot can't find buttons | Google Meet UI language must match selectors. English and Ukrainian are supported |
| OpenAI API errors | Check `OPENAI_API_KEY` is valid |
| `post_join_setup_error` | Run `npm run build` (not just `tsc`) in services/vexa-bot |

## Tech Stack

- **Rust** — Jarvis desktop app (Axum, hound, tracing-appender, tokio)
- **TypeScript/Playwright** — vexa-bot browser automation with stealth plugin
- **OpenAI GPT-5.4** — LLM responses + GPT-5 for intent detection (reasoning_effort=minimal)
- **OpenAI Whisper** — speech-to-text (cloud or local via whisper-rs)
- **OpenAI TTS** — text-to-speech
- **WebRTC** — audio capture and playback

## License

MIT
