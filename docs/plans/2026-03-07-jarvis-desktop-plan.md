# Jarvis Desktop App — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace 15 Docker containers with a single Rust binary + embedded Node.js vexa-bot child process.

**Architecture:** Rust core handles transcription (whisper-rs or OpenAI API), LLM intent detection + responses (GPT-4o-mini/4o), TTS (OpenAI API), Web UI (axum + rust-embed), and SQLite storage. vexa-bot (Node.js + Playwright) runs as a child process and connects to Rust via a single WebSocket for audio streaming and command exchange.

**Tech Stack:** Rust (tokio, axum, whisper-rs, reqwest, rusqlite, rust-embed), Node.js (Playwright), SQLite

---

### Task 1: Rust Project Scaffold

**Files:**
- Create: `jarvis/Cargo.toml`
- Create: `jarvis/src/main.rs`
- Create: `jarvis/src/config.rs`
- Create: `jarvis/src/db.rs`

**Step 1: Create project directory and Cargo.toml**

```bash
mkdir -p jarvis/src
```

```toml
# jarvis/Cargo.toml
[package]
name = "jarvis"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["ws"] }
tower-http = { version = "0.5", features = ["cors"] }
tokio-tungstenite = "0.24"
futures-util = "0.3"
reqwest = { version = "0.12", features = ["json", "multipart", "stream"] }
whisper-rs = "0.12"
rusqlite = { version = "0.31", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rust-embed = "8"
base64 = "0.22"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive", "env"] }
dotenvy = "0.15"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
```

**Step 2: Create main.rs with CLI args and startup**

```rust
// jarvis/src/main.rs
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod config;
mod db;

#[derive(Parser, Debug)]
#[command(name = "jarvis", about = "AI Meeting Assistant")]
struct Args {
    /// OpenAI API key
    #[arg(long, env = "OPENAI_API_KEY")]
    openai_key: String,

    /// Meeting URL (Google Meet, Teams, or Zoom)
    #[arg(long, env = "MEET_URL")]
    meet: Option<String>,

    /// Bot display name
    #[arg(long, env = "BOT_DISPLAY_NAME", default_value = "Jarvis")]
    bot_name: String,

    /// Transcription mode: local or cloud
    #[arg(long, env = "TRANSCRIPTION_MODE", default_value = "cloud")]
    transcription: String,

    /// Whisper model for local transcription
    #[arg(long, env = "WHISPER_MODEL", default_value = "small")]
    whisper_model: String,

    /// Web UI port
    #[arg(long, env = "WEB_UI_PORT", default_value = "8080")]
    port: u16,

    /// TTS voice
    #[arg(long, env = "TTS_VOICE", default_value = "nova")]
    tts_voice: String,

    /// OpenAI model
    #[arg(long, env = "OPENAI_MODEL", default_value = "gpt-4o")]
    model: String,

    /// Trigger phrase hint for intent detection
    #[arg(long, env = "TRIGGER_PHRASE", default_value = "hey bot")]
    trigger_phrase: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("jarvis=info".parse()?))
        .init();

    let args = Args::parse();
    let cfg = config::Config::from_args(&args);

    tracing::info!("Jarvis v{} starting...", env!("CARGO_PKG_VERSION"));
    tracing::info!("Web UI: http://localhost:{}", cfg.port);

    let db = db::Database::open(&cfg.db_path)?;
    db.migrate()?;

    // TODO: start bot_bridge (Task 2)
    // TODO: start web server (Task 6)
    // TODO: start vexa-bot process (Task 7)

    tracing::info!("Jarvis running. Press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down...");
    Ok(())
}
```

**Step 3: Create config.rs**

```rust
// jarvis/src/config.rs
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub openai_key: String,
    pub meet_url: Option<String>,
    pub bot_name: String,
    pub transcription_mode: TranscriptionMode,
    pub whisper_model: String,
    pub port: u16,
    pub bridge_port: u16,
    pub tts_voice: String,
    pub openai_model: String,
    pub trigger_phrase: String,
    pub db_path: PathBuf,
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptionMode {
    Local,
    Cloud,
}

impl Config {
    pub fn from_args(args: &super::Args) -> Self {
        let data_dir = dirs_or_default();
        std::fs::create_dir_all(&data_dir).ok();

        Self {
            openai_key: args.openai_key.clone(),
            meet_url: args.meet.clone(),
            bot_name: args.bot_name.clone(),
            transcription_mode: match args.transcription.as_str() {
                "local" => TranscriptionMode::Local,
                _ => TranscriptionMode::Cloud,
            },
            whisper_model: args.whisper_model.clone(),
            port: args.port,
            bridge_port: 9090,
            tts_voice: args.tts_voice.clone(),
            openai_model: args.model.clone(),
            trigger_phrase: args.trigger_phrase.clone(),
            db_path: data_dir.join("jarvis.db"),
            data_dir,
        }
    }
}

fn dirs_or_default() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jarvis")
}
```

Add `dirs = "5"` and `anyhow = "1"` to Cargo.toml dependencies.

**Step 4: Create db.rs**

```rust
// jarvis/src/db.rs
use rusqlite::{Connection, Result};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn })
    }

    pub fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transcript_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_url TEXT,
                speaker TEXT,
                text TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS meetings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                bot_name TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                summary TEXT
            );"
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }
}
```

**Step 5: Verify it compiles**

```bash
cd jarvis && cargo build 2>&1
```

Expected: Compiles successfully.

**Step 6: Commit**

```bash
git add jarvis/
git commit -m "feat: scaffold Jarvis Rust project with config, CLI args, SQLite"
```

---

### Task 2: Bot Bridge — WebSocket Server for vexa-bot

**Files:**
- Create: `jarvis/src/bot_bridge.rs`
- Modify: `jarvis/src/main.rs`

**Step 1: Create bot_bridge.rs**

The bridge listens on :9090 and handles the WebSocket protocol defined in the design doc.

```rust
// jarvis/src/bot_bridge.rs
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

/// Messages from vexa-bot to Rust core
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum BotMessage {
    #[serde(rename = "audio")]
    Audio {
        data: String, // base64 Float32 PCM 16kHz mono
        sample_rate: u32,
    },
    #[serde(rename = "event")]
    Event {
        event: String,
        data: serde_json::Value,
    },
}

/// Messages from Rust core to vexa-bot
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum CoreMessage {
    #[serde(rename = "speak")]
    Speak { audio: String }, // base64 WAV
    #[serde(rename = "command")]
    Command { action: String },
}

pub struct BridgeState {
    /// Sends audio chunks from vexa-bot to transcription pipeline
    pub audio_tx: mpsc::Sender<Vec<f32>>,
    /// Receives audio chunks for transcription
    pub audio_rx: Mutex<mpsc::Receiver<Vec<f32>>>,
    /// Sends events from vexa-bot to core
    pub event_tx: broadcast::Sender<(String, serde_json::Value)>,
    /// Sends commands from core to vexa-bot
    pub command_tx: broadcast::Sender<CoreMessage>,
    /// Track if vexa-bot is connected
    pub connected: Mutex<bool>,
}

impl BridgeState {
    pub fn new() -> Arc<Self> {
        let (audio_tx, audio_rx) = mpsc::channel(256);
        let (event_tx, _) = broadcast::channel(64);
        let (command_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            audio_tx,
            audio_rx: Mutex::new(audio_rx),
            event_tx,
            command_tx,
            connected: Mutex::new(false),
        })
    }
}

pub fn router(state: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<BridgeState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<BridgeState>) {
    let (mut sender, mut receiver) = socket.split();

    *state.connected.lock().await = true;
    tracing::info!("[bridge] vexa-bot connected");

    // Forward commands from core to vexa-bot
    let state_clone = state.clone();
    let mut cmd_rx = state_clone.command_tx.subscribe();
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = cmd_rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    // Receive messages from vexa-bot
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                if let Ok(bot_msg) = serde_json::from_str::<BotMessage>(&text) {
                    match bot_msg {
                        BotMessage::Audio { data, .. } => {
                            if let Ok(bytes) = base64::Engine::decode(
                                &base64::engine::general_purpose::STANDARD,
                                &data,
                            ) {
                                let samples: Vec<f32> = bytes
                                    .chunks_exact(4)
                                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                    .collect();
                                let _ = state.audio_tx.send(samples).await;
                            }
                        }
                        BotMessage::Event { event, data } => {
                            tracing::info!("[bridge] event: {} {:?}", event, data);
                            let _ = state.event_tx.send((event, data));
                        }
                    }
                }
            }
            Message::Binary(bytes) => {
                // Raw binary audio: Float32 PCM directly
                let samples: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                let _ = state.audio_tx.send(samples).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
    *state.connected.lock().await = false;
    tracing::info!("[bridge] vexa-bot disconnected");
}
```

**Step 2: Wire into main.rs**

Add `mod bot_bridge;` and start the bridge server:

```rust
// In main.rs, after db.migrate()
let bridge_state = bot_bridge::BridgeState::new();
let bridge_router = bot_bridge::router(bridge_state.clone());

let bridge_addr = format!("0.0.0.0:{}", cfg.bridge_port);
tracing::info!("Bot bridge listening on {}", bridge_addr);
tokio::spawn(async move {
    let listener = tokio::net::TcpListener::bind(&bridge_addr).await.unwrap();
    axum::serve(listener, bridge_router).await.unwrap();
});
```

**Step 3: Verify it compiles**

```bash
cd jarvis && cargo build 2>&1
```

**Step 4: Commit**

```bash
git add jarvis/src/bot_bridge.rs jarvis/src/main.rs
git commit -m "feat: add WebSocket bridge server for vexa-bot communication"
```

---

### Task 3: Modify vexa-bot for Direct WebSocket Connection

**Files:**
- Create: `jarvis/vexa-bot/bridge-client.ts`
- Modify: `services/vexa-bot/core/src/index.ts` (add bridge mode)
- Modify: `services/vexa-bot/core/src/platforms/googlemeet/recording.ts` (redirect audio)

This is the most delicate task. We add a "bridge mode" to vexa-bot that connects to ws://localhost:9090/ws instead of Redis + WhisperLive. The original mode stays intact for Docker usage.

**Step 1: Create bridge-client.ts**

```typescript
// services/vexa-bot/core/src/services/bridge-client.ts
import WebSocket from 'ws';

export class BridgeClient {
    private ws: WebSocket | null = null;
    private url: string;
    private onCommand: ((action: string, data: any) => void) | null = null;
    private onSpeakAudio: ((audioBase64: string) => void) | null = null;
    private reconnectTimer: NodeJS.Timeout | null = null;

    constructor(url: string = 'ws://localhost:9090/ws') {
        this.url = url;
    }

    connect(): Promise<void> {
        return new Promise((resolve, reject) => {
            this.ws = new WebSocket(this.url);
            this.ws.on('open', () => {
                console.log('[Bridge] Connected to Rust core');
                resolve();
            });
            this.ws.on('message', (data: Buffer) => {
                try {
                    const msg = JSON.parse(data.toString());
                    if (msg.type === 'speak' && this.onSpeakAudio) {
                        this.onSpeakAudio(msg.audio);
                    } else if (msg.type === 'command' && this.onCommand) {
                        this.onCommand(msg.action, msg);
                    }
                } catch (e) {
                    console.error('[Bridge] Parse error:', e);
                }
            });
            this.ws.on('close', () => {
                console.log('[Bridge] Disconnected, reconnecting in 3s...');
                this.reconnectTimer = setTimeout(() => this.connect(), 3000);
            });
            this.ws.on('error', (err) => {
                console.error('[Bridge] Error:', err.message);
                reject(err);
            });
        });
    }

    sendAudio(float32Data: Float32Array) {
        if (this.ws?.readyState === WebSocket.OPEN) {
            // Send as binary for efficiency (Float32 PCM)
            this.ws.send(Buffer.from(float32Data.buffer));
        }
    }

    sendEvent(event: string, data: any = {}) {
        if (this.ws?.readyState === WebSocket.OPEN) {
            this.ws.send(JSON.stringify({ type: 'event', event, data }));
        }
    }

    onCommandReceived(handler: (action: string, data: any) => void) {
        this.onCommand = handler;
    }

    onSpeakReceived(handler: (audioBase64: string) => void) {
        this.onSpeakAudio = handler;
    }

    close() {
        if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
        this.ws?.close();
    }
}
```

**Step 2: Add bridge mode to vexa-bot index.ts**

In `services/vexa-bot/core/src/index.ts`, detect `BRIDGE_URL` env var. When set, use BridgeClient instead of Redis for commands, and route audio to bridge instead of WhisperLive.

Key changes:
- If `process.env.BRIDGE_URL` is set, create `BridgeClient` instead of Redis subscriber
- Replace `handleRedisMessage` with bridge command handler
- Replace `publishVoiceEvent` with `bridge.sendEvent()`
- In recording.ts, send audio chunks to bridge instead of WhisperLive

**Step 3: Modify recording.ts audio routing**

In `startGoogleRecording()`, when bridge mode is active:
- Skip WhisperLive connection entirely
- In the `onaudioprocess` callback, call `bridgeClient.sendAudio(float32Data)` instead of `whisperLive.sendAudioData()`

**Step 4: Modify TTS playback for bridge mode**

When bridge mode is active and a `speak` command arrives with base64 WAV audio:
- Decode base64 to buffer
- Write to temp file
- Play via `paplay --device=tts_sink /tmp/tts_audio.wav`

This reuses the existing PulseAudio pipeline — audio goes to tts_sink → virtual-mic → Chrome → Meet.

**Step 5: Test bridge mode**

```bash
# Terminal 1: Run Rust bridge
cd jarvis && cargo run -- --openai-key sk-test

# Terminal 2: Run vexa-bot in bridge mode
cd services/vexa-bot && BRIDGE_URL=ws://localhost:9090/ws npm run dev
```

Verify: vexa-bot connects to bridge, audio chunks flow.

**Step 6: Commit**

```bash
git add services/vexa-bot/core/src/services/bridge-client.ts
git add services/vexa-bot/core/src/index.ts
git add services/vexa-bot/core/src/platforms/googlemeet/recording.ts
git commit -m "feat: add bridge mode to vexa-bot for direct WebSocket connection to Rust core"
```

---

### Task 4: Transcription — Local (whisper-rs) and Cloud (OpenAI API)

**Files:**
- Create: `jarvis/src/transcription/mod.rs`
- Create: `jarvis/src/transcription/local.rs`
- Create: `jarvis/src/transcription/cloud.rs`
- Modify: `jarvis/src/main.rs`

**Step 1: Create trait Transcriber**

```rust
// jarvis/src/transcription/mod.rs
pub mod cloud;
pub mod local;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct TranscriptSegment {
    pub text: String,
    pub language: Option<String>,
}

#[async_trait]
pub trait Transcriber: Send + Sync {
    /// Transcribe a chunk of Float32 PCM audio at 16kHz mono
    async fn transcribe(&self, audio: &[f32]) -> anyhow::Result<Option<TranscriptSegment>>;
}
```

Add `async-trait = "0.1"` to Cargo.toml.

**Step 2: Create cloud.rs (OpenAI Whisper API)**

```rust
// jarvis/src/transcription/cloud.rs
use super::{TranscriptSegment, Transcriber};
use async_trait::async_trait;
use reqwest::multipart;

pub struct CloudTranscriber {
    client: reqwest::Client,
    api_key: String,
}

impl CloudTranscriber {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl Transcriber for CloudTranscriber {
    async fn transcribe(&self, audio: &[f32]) -> anyhow::Result<Option<TranscriptSegment>> {
        if audio.len() < 8000 {
            // Less than 0.5s of audio, skip
            return Ok(None);
        }

        // Convert Float32 PCM to WAV bytes
        let wav = pcm_to_wav(audio, 16000);

        let part = multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")?;

        let form = multipart::Form::new()
            .text("model", "whisper-1")
            .text("response_format", "json")
            .part("file", part);

        let resp = self.client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        let text = body["text"].as_str().unwrap_or("").trim().to_string();

        if text.is_empty() {
            return Ok(None);
        }

        Ok(Some(TranscriptSegment {
            text,
            language: body["language"].as_str().map(|s| s.to_string()),
        }))
    }
}

fn pcm_to_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len();
    let byte_rate = sample_rate * 2; // 16-bit mono
    let data_size = (num_samples * 2) as u32;
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + num_samples * 2);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes());  // PCM
    buf.extend_from_slice(&1u16.to_le_bytes());  // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());  // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());

    for &s in samples {
        let i = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&i.to_le_bytes());
    }
    buf
}
```

**Step 3: Create local.rs (whisper-rs)**

```rust
// jarvis/src/transcription/local.rs
use super::{TranscriptSegment, Transcriber};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct LocalTranscriber {
    ctx: Mutex<WhisperContext>,
}

impl LocalTranscriber {
    pub fn new(model_path: &PathBuf) -> anyhow::Result<Self> {
        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().unwrap(),
            params,
        ).map_err(|e| anyhow::anyhow!("whisper init failed: {:?}", e))?;

        Ok(Self { ctx: Mutex::new(ctx) })
    }

    /// Download model if not present. Returns path.
    pub async fn ensure_model(data_dir: &PathBuf, model_name: &str) -> anyhow::Result<PathBuf> {
        let models_dir = data_dir.join("models");
        std::fs::create_dir_all(&models_dir)?;

        let filename = format!("ggml-{}.bin", model_name);
        let model_path = models_dir.join(&filename);

        if model_path.exists() {
            return Ok(model_path);
        }

        let url = format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            filename
        );
        tracing::info!("Downloading whisper model {}...", model_name);

        let resp = reqwest::get(&url).await?;
        let bytes = resp.bytes().await?;
        std::fs::write(&model_path, &bytes)?;

        tracing::info!("Model downloaded to {:?}", model_path);
        Ok(model_path)
    }
}

#[async_trait]
impl Transcriber for LocalTranscriber {
    async fn transcribe(&self, audio: &[f32]) -> anyhow::Result<Option<TranscriptSegment>> {
        if audio.len() < 8000 {
            return Ok(None);
        }

        let audio = audio.to_vec();
        let ctx = &self.ctx;

        // whisper-rs is not async, run in blocking thread
        tokio::task::spawn_blocking(move || {
            let ctx = ctx.lock().unwrap();
            let mut state = ctx.create_state()
                .map_err(|e| anyhow::anyhow!("whisper state: {:?}", e))?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(4);
            params.set_language(Some("auto"));
            params.set_token_timestamps(false);

            state.full(params, &audio)
                .map_err(|e| anyhow::anyhow!("whisper full: {:?}", e))?;

            let num_segments = state.full_n_segments()
                .map_err(|e| anyhow::anyhow!("segments: {:?}", e))?;

            let mut text = String::new();
            for i in 0..num_segments {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    text.push_str(&seg);
                }
            }

            let text = text.trim().to_string();
            if text.is_empty() {
                return Ok(None);
            }

            Ok(Some(TranscriptSegment {
                text,
                language: None,
            }))
        }).await?
    }
}
```

**Step 4: Wire transcription into main.rs**

```rust
mod transcription;

// In main(), after bridge setup:
let transcriber: Arc<dyn transcription::Transcriber> = match cfg.transcription_mode {
    config::TranscriptionMode::Cloud => {
        Arc::new(transcription::cloud::CloudTranscriber::new(&cfg.openai_key))
    }
    config::TranscriptionMode::Local => {
        let model_path = transcription::local::LocalTranscriber::ensure_model(
            &cfg.data_dir, &cfg.whisper_model
        ).await?;
        Arc::new(transcription::local::LocalTranscriber::new(&model_path)?)
    }
};
```

**Step 5: Audio processing loop**

Create a task that reads audio from bridge, accumulates chunks (~3 seconds), transcribes:

```rust
// In main.rs, spawn transcription loop
let bridge_for_tx = bridge_state.clone();
let transcriber_clone = transcriber.clone();
tokio::spawn(async move {
    let mut audio_rx = bridge_for_tx.audio_rx.lock().await;
    let mut buffer: Vec<f32> = Vec::new();
    let chunk_size = 16000 * 3; // 3 seconds at 16kHz

    while let Some(samples) = audio_rx.recv().await {
        buffer.extend_from_slice(&samples);
        if buffer.len() >= chunk_size {
            let chunk = buffer.drain(..chunk_size).collect::<Vec<_>>();
            match transcriber_clone.transcribe(&chunk).await {
                Ok(Some(seg)) => {
                    tracing::info!("[transcript] {}", seg.text);
                    // TODO: feed to LLM (Task 5)
                }
                Ok(None) => {}
                Err(e) => tracing::error!("[transcript] error: {}", e),
            }
        }
    }
});
```

**Step 6: Verify it compiles**

```bash
cd jarvis && cargo build 2>&1
```

**Step 7: Commit**

```bash
git add jarvis/src/transcription/
git commit -m "feat: add local (whisper-rs) and cloud (OpenAI) transcription"
```

---

### Task 5: LLM Intent Detection + Responses + TTS

**Files:**
- Create: `jarvis/src/llm.rs`
- Create: `jarvis/src/tts.rs`
- Modify: `jarvis/src/main.rs`

**Step 1: Create llm.rs**

Port the Go LLM logic to Rust. Intent detection with GPT-4o-mini, responses with GPT-4o.

```rust
// jarvis/src/llm.rs
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

pub struct LlmAgent {
    client: Client,
    api_key: String,
    model: String,
    bot_name: String,
    trigger_phrase: String,
    history: Mutex<Vec<ChatMessage>>,
    transcript: Mutex<Vec<String>>,
}

impl LlmAgent {
    pub fn new(api_key: &str, model: &str, bot_name: &str, trigger_phrase: &str) -> Self {
        let system_msg = format!(
            "You are {}, an AI meeting assistant in a Google Meet call.\n\
             You respond when participants address you directly.\n\
             Keep responses concise (1-3 sentences).\n\
             IMPORTANT: Respond ONLY in English or Ukrainian. NEVER respond in Russian.",
            bot_name
        );

        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            bot_name: bot_name.to_string(),
            trigger_phrase: trigger_phrase.to_string(),
            history: Mutex::new(vec![ChatMessage {
                role: "system".to_string(),
                content: system_msg,
            }]),
            transcript: Mutex::new(Vec::new()),
        }
    }

    pub fn add_transcript(&self, text: &str) {
        let mut t = self.transcript.lock().unwrap();
        t.push(text.to_string());
        if t.len() > 50 {
            let drain = t.len() - 50;
            t.drain(..drain);
        }
    }

    pub async fn should_respond(&self, text: &str) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.len() < 5 {
            return None;
        }

        let recent = {
            let t = self.transcript.lock().unwrap();
            t.iter().rev().take(5).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
        };

        let prompt = format!(
            "You are an intent detector for a meeting bot named \"{}\".\n\
             The trigger phrase is \"{}\" but speech recognition often misheard it.\n\n\
             Given the transcript line below, determine if the speaker is addressing the bot.\n\
             Consider any phrase that sounds like the trigger, mentioning the bot by name, or asking a question directed at an AI.\n\n\
             Recent context:\n{}\n\n\
             New line: \"{}\"\n\n\
             Respond with EXACTLY:\n\
             - \"YES: <question>\" if addressing the bot\n\
             - \"NO\" if not",
            self.bot_name, self.trigger_phrase, recent, trimmed
        );

        let resp = self.chat_once("gpt-4o-mini", &prompt, 0.0, 60).await.ok()?;
        let answer = resp.trim();

        if let Some(question) = answer.strip_prefix("YES:") {
            let q = question.trim();
            Some(if q.is_empty() { trimmed.to_string() } else { q.to_string() })
        } else if answer == "YES" {
            Some(trimmed.to_string())
        } else {
            None
        }
    }

    pub async fn respond(&self, question: &str) -> anyhow::Result<String> {
        let recent = {
            let t = self.transcript.lock().unwrap();
            t.iter().rev().take(10).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
        };

        let user_msg = format!(
            "[Recent meeting context]:\n{}\n\n[Question to you]: {}",
            recent, question
        );

        let mut history = self.history.lock().unwrap();
        history.push(ChatMessage { role: "user".to_string(), content: user_msg });

        let messages = history.clone();
        drop(history);

        let req = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
            max_tokens: 150,
        };

        let resp: ChatResponse = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?
            .json()
            .await?;

        let answer = resp.choices.first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default();

        let mut history = self.history.lock().unwrap();
        history.push(ChatMessage { role: "assistant".to_string(), content: answer.clone() });
        if history.len() > 41 {
            let keep = history.split_off(history.len() - 40);
            let system = history[0].clone();
            *history = vec![system];
            history.extend(keep);
        }

        Ok(answer)
    }

    pub async fn summary(&self) -> anyhow::Result<String> {
        let transcript = {
            let t = self.transcript.lock().unwrap();
            t.join("\n")
        };

        let prompt = format!(
            "Provide a brief meeting summary (3-5 bullet points) based on:\n{}",
            transcript
        );

        self.chat_once(&self.model, &prompt, 0.7, 300).await
    }

    async fn chat_once(&self, model: &str, prompt: &str, temp: f32, max_tokens: u32) -> anyhow::Result<String> {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage { role: "user".to_string(), content: prompt.to_string() }],
            temperature: temp,
            max_tokens,
        };

        let resp: ChatResponse = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?
            .json()
            .await?;

        Ok(resp.choices.first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default())
    }
}
```

**Step 2: Create tts.rs**

```rust
// jarvis/src/tts.rs
use reqwest::Client;
use serde::Serialize;

#[derive(Serialize)]
struct TtsRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

pub struct TtsService {
    client: Client,
    api_key: String,
    voice: String,
}

impl TtsService {
    pub fn new(api_key: &str, voice: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            voice: voice.to_string(),
        }
    }

    /// Synthesize text to WAV audio bytes
    pub async fn synthesize(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        let req = TtsRequest {
            model: "tts-1".to_string(),
            input: text.to_string(),
            voice: self.voice.clone(),
            response_format: "wav".to_string(),
        };

        let resp = self.client
            .post("https://api.openai.com/v1/audio/speech")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await?;
            anyhow::bail!("TTS error {}: {}", status, body);
        }

        Ok(resp.bytes().await?.to_vec())
    }
}
```

**Step 3: Wire LLM + TTS into transcription loop in main.rs**

After transcription produces text:
1. `agent.add_transcript(text)`
2. `agent.should_respond(text)` → if Some(question)
3. `agent.respond(question)` → answer
4. `tts.synthesize(answer)` → wav bytes
5. Send wav to vexa-bot via bridge: `CoreMessage::Speak { audio: base64(wav) }`

**Step 4: Commit**

```bash
git add jarvis/src/llm.rs jarvis/src/tts.rs jarvis/src/main.rs
git commit -m "feat: add LLM intent detection, GPT-4o responses, and OpenAI TTS"
```

---

### Task 6: Web UI — Axum HTTP Server

**Files:**
- Create: `jarvis/src/server.rs`
- Create: `jarvis/src/assets/index.html` (port from current Go project)
- Modify: `jarvis/src/main.rs`

**Step 1: Create server.rs**

```rust
// jarvis/src/server.rs
use axum::{
    extract::{ws::WebSocket, ws::WebSocketUpgrade, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Embed)]
#[folder = "src/assets/"]
struct Assets;

pub struct AppState {
    pub config: Arc<crate::config::Config>,
    pub transcript_tx: broadcast::Sender<String>,
    // TODO: references to LLM agent, bridge, etc.
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/config", get(get_config).post(update_config))
        .route("/api/status", get(get_status))
        .route("/api/join", post(join_meeting))
        .route("/api/leave", post(leave_meeting))
        .route("/api/transcript", get(transcript_ws))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => Html(String::from_utf8_lossy(content.data.as_ref()).to_string()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ... endpoint implementations similar to current Go web/server.go
```

**Step 2: Port index.html**

Copy `internal/web/index.html` to `jarvis/src/assets/index.html`. Update API endpoints to match new routes.

**Step 3: Wire into main.rs**

```rust
let (transcript_tx, _) = broadcast::channel(256);
let app_state = Arc::new(server::AppState {
    config: Arc::new(cfg.clone()),
    transcript_tx: transcript_tx.clone(),
});

let app = server::router(app_state);
let addr = format!("0.0.0.0:{}", cfg.port);
tokio::spawn(async move {
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
});

// Open browser
open::that(format!("http://localhost:{}", cfg.port)).ok();
```

Add `open = "5"` to Cargo.toml.

**Step 4: Commit**

```bash
git add jarvis/src/server.rs jarvis/src/assets/
git commit -m "feat: add Web UI with axum server and embedded HTML"
```

---

### Task 7: Process Manager — Launch vexa-bot

**Files:**
- Create: `jarvis/src/process.rs`
- Modify: `jarvis/src/main.rs`

**Step 1: Create process.rs**

```rust
// jarvis/src/process.rs
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub struct VexaBotProcess {
    child: Option<Child>,
}

impl VexaBotProcess {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub async fn start(
        &mut self,
        node_path: &PathBuf,
        vexa_bot_dir: &PathBuf,
        bridge_url: &str,
        meet_url: &str,
        bot_name: &str,
    ) -> anyhow::Result<()> {
        let child = Command::new(node_path)
            .arg(vexa_bot_dir.join("core/dist/index.js"))
            .env("BRIDGE_URL", bridge_url)
            .env("MEETING_URL", meet_url)
            .env("BOT_NAME", bot_name)
            .env("PLATFORM", "google_meet")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        tracing::info!("[process] vexa-bot started (pid: {:?})", child.id());
        self.child = Some(child);
        Ok(())
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(ref mut child) = self.child {
            child.kill().await?;
            tracing::info!("[process] vexa-bot stopped");
        }
        self.child = None;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for VexaBotProcess {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
        }
    }
}
```

**Step 2: Resolve node path**

Look for node in: `./node`, system PATH, or bundled location.

```rust
pub fn find_node() -> anyhow::Result<PathBuf> {
    // Check bundled node next to binary
    let exe_dir = std::env::current_exe()?.parent().unwrap().to_path_buf();
    let bundled = exe_dir.join("node");
    if bundled.exists() {
        return Ok(bundled);
    }

    // Check system PATH
    if let Ok(output) = std::process::Command::new("which").arg("node").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok(PathBuf::from(path));
        }
    }

    anyhow::bail!("Node.js not found. Install Node.js or place 'node' binary next to jarvis.")
}
```

**Step 3: Wire into main.rs**

```rust
mod process;

// After bridge and web server are started:
if let Some(ref meet_url) = cfg.meet_url {
    let node_path = process::find_node()?;
    let vexa_bot_dir = std::env::current_exe()?.parent().unwrap().join("vexa-bot");
    let mut bot_process = process::VexaBotProcess::new();
    bot_process.start(
        &node_path,
        &vexa_bot_dir,
        &format!("ws://localhost:{}/ws", cfg.bridge_port),
        meet_url,
        &cfg.bot_name,
    ).await?;
}
```

**Step 4: Create packaging script**

```bash
#!/bin/bash
# scripts/package.sh
set -e

TARGET=${1:-"current"}
VERSION=$(cargo metadata --format-version 1 | jq -r '.packages[] | select(.name=="jarvis") | .version')

cargo build --release

DIST="dist/jarvis-v${VERSION}"
rm -rf "$DIST"
mkdir -p "$DIST"

cp target/release/jarvis "$DIST/"
cp -r services/vexa-bot "$DIST/vexa-bot"

echo "Packaged to $DIST"
echo "To run: cd $DIST && ./jarvis --openai-key sk-... --meet https://meet.google.com/..."
```

**Step 5: Commit**

```bash
git add jarvis/src/process.rs scripts/package.sh
git commit -m "feat: add process manager for vexa-bot and packaging script"
```

---

## Summary

| Task | What | Key Files |
|------|------|-----------|
| 1 | Scaffold: Cargo.toml, config, CLI, SQLite | `jarvis/src/{main,config,db}.rs` |
| 2 | Bot Bridge: WS server :9090 | `jarvis/src/bot_bridge.rs` |
| 3 | Modify vexa-bot: bridge mode | `services/vexa-bot/core/src/services/bridge-client.ts` |
| 4 | Transcription: whisper-rs + OpenAI API | `jarvis/src/transcription/{mod,local,cloud}.rs` |
| 5 | LLM + TTS: intent, respond, summary | `jarvis/src/{llm,tts}.rs` |
| 6 | Web UI: axum + rust-embed | `jarvis/src/{server.rs,assets/index.html}` |
| 7 | Process Manager: launch vexa-bot | `jarvis/src/process.rs`, `scripts/package.sh` |
