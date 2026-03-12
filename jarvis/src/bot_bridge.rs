use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    response::IntoResponse,
    routing::get,
    Router,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
    /// Track number of active vexa-bot connections
    pub connection_count: AtomicU32,
    /// Current active speaker name
    pub current_speaker: Mutex<Option<String>>,
    /// True while bot is speaking (suppresses audio capture to prevent self-echo)
    pub is_speaking: AtomicBool,
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
            connection_count: AtomicU32::new(0),
            current_speaker: Mutex::new(None),
            is_speaking: AtomicBool::new(false),
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

    state.connection_count.fetch_add(1, Ordering::Relaxed);
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
                            // Skip audio while bot is speaking to prevent self-echo
                            if state.is_speaking.load(Ordering::Relaxed) {
                                continue;
                            }
                            if let Ok(bytes) =
                                base64::engine::general_purpose::STANDARD.decode(&data)
                            {
                                let samples: Vec<f32> = bytes
                                    .chunks_exact(4)
                                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                    .collect();
                                let _ = state.audio_tx.send(samples).await;
                            }
                        }
                        BotMessage::Event { event, data } => {
                            tracing::info!("[bridge] event: {} {:?}", event, data);

                            // Track speak state to suppress self-echo
                            if event == "speak.started" {
                                state.is_speaking.store(true, Ordering::Relaxed);
                                tracing::debug!("[bridge] speaking=true (suppressing audio capture)");
                            } else if event == "speak.completed" {
                                // Delay clearing to let residual echo fade
                                let state_speak = state.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    state_speak.is_speaking.store(false, Ordering::Relaxed);
                                    tracing::debug!("[bridge] speaking=false (audio capture resumed)");
                                });
                            }

                            // Track current speaker from speaker_activity events
                            // Only update on SPEAKER_START — don't clear on END,
                            // so the last speaker persists through buffered transcription
                            if event == "speaker_activity" {
                                if let Some(event_type) = data.get("event_type").and_then(|v| v.as_str()) {
                                    if event_type == "SPEAKER_START" {
                                        if let Some(name) = data.get("participant_name").and_then(|v| v.as_str()) {
                                            *state.current_speaker.lock().await = Some(name.to_string());
                                        }
                                    }
                                }
                            }

                            let _ = state.event_tx.send((event, data));
                        }
                    }
                }
            }
            Message::Binary(bytes) => {
                // Skip audio while bot is speaking to prevent self-echo
                if state.is_speaking.load(Ordering::Relaxed) {
                    continue;
                }
                // Raw binary audio: Float32 PCM directly
                let samples: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                // Log first few audio frames and periodically for diagnostics
                static BINARY_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let count = BINARY_COUNT.fetch_add(1, Ordering::Relaxed);
                if count < 5 || count % 1000 == 0 {
                    let rms: f32 = if samples.is_empty() { 0.0 } else {
                        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
                    };
                    tracing::info!("[bridge] binary audio frame #{}: {} bytes, {} samples, RMS={:.6}", count, bytes.len(), samples.len(), rms);
                }
                if let Err(e) = state.audio_tx.send(samples).await {
                    tracing::warn!("[bridge] audio_tx send failed: {}", e);
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
    state.connection_count.fetch_sub(1, Ordering::Relaxed);
    tracing::info!("[bridge] vexa-bot disconnected");
}
