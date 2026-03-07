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
