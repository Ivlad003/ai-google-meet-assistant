# Technical Details

## Bridge Mode Chrome Flags

- `--mute-audio` — prevents Chrome audio output (no echo)
- `--use-fake-device-for-media-stream` — prevents real mic capture (no feedback loop)
- `--use-file-for-fake-audio-capture=/dev/null` — silence as mic input

## Key Technical Details

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

## Session Output Files

All saved to `~/Library/Application Support/jarvis/` (macOS):

- `sessions/YYYY-MM-DD_HHMMSS.txt` — transcript with timestamps and speaker names
- `sessions/YYYY-MM-DD_HHMMSS.wav` — full audio recording
- `logs/jarvis.log.YYYY-MM-DD` — daily rotated log files

On shutdown, paths are printed to terminal.

## Configuration Reference

All settings in `jarvis.config.json`. Loads from current directory by default.

```bash
# Default (loads ./jarvis.config.json)
./jarvis/target/debug/jarvis

# Custom path
./jarvis/target/debug/jarvis --config /path/to/config.json

# Or via env var
JARVIS_CONFIG=/path/to/config.json ./jarvis/target/debug/jarvis
```

See `jarvis.config.example.json` for all options.

| Field | Required | Default | Description |
|---|---|---|---|
| `openai_key` | **Yes** | — | OpenAI API key |
| `meet_url` | No | — | Meeting URL (can set via Web UI) |
| `bot_name` | No | `Jarvis` | Bot display name |
| `language` | No | `auto` | Transcription language (en, uk, auto) |
| `openai_model` | No | `gpt-5.4` | LLM model for responses |
| `intent_model` | No | `gpt-5` | Intent detection model (uses reasoning_effort=minimal) |
| `tts_voice` | No | `nova` | OpenAI TTS voice |
| `transcription_mode` | No | `cloud` | `cloud` (OpenAI API) or `local` (whisper-rs) |
| `whisper_model` | No | `small` | Local whisper model name |
| `port` | No | `8080` | Web UI port |
| `bridge_port` | No | `9090` | Bridge WebSocket port |
| `max_response_tokens` | No | `150` | Max tokens in LLM response |
| `temperature` | No | `0.7` | LLM temperature |
| `response_mode` | No | `smart` | `smart` (LLM intent detection) or `name_only` (keyword match on bot name) |
| `system_prompt` | No | built-in | Custom system prompt |
| `intent_prompt` | No | built-in | Custom intent prompt (`{bot_name}`, `{context}`, `{speaker}`, `{text}`) |
| `tools` | No | `[]` | Custom tool integrations |
