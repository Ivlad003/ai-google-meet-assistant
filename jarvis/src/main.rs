use base64::Engine;
use clap::Parser;
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;

mod bot_bridge;
mod config;
mod db;
mod llm;
mod process;
mod server;
mod transcription;
mod tts;

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

    /// Transcription language (ISO 639-1 code, e.g. "en", "uk", "auto")
    #[arg(long, env = "LANGUAGE", default_value = "auto")]
    language: String,
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

    // Start bot bridge WebSocket server
    let bridge_state = bot_bridge::BridgeState::new();
    let bridge_router = bot_bridge::router(bridge_state.clone());

    let bridge_addr = format!("0.0.0.0:{}", cfg.bridge_port);
    tracing::info!("Bot bridge listening on {}", bridge_addr);
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&bridge_addr).await.unwrap();
        axum::serve(listener, bridge_router).await.unwrap();
    });

    // Create transcriber based on config
    let transcriber: Arc<dyn transcription::Transcriber> = match cfg.transcription_mode {
        config::TranscriptionMode::Cloud => {
            tracing::info!("Using cloud transcription (OpenAI Whisper API)");
            Arc::new(transcription::cloud::CloudTranscriber::new(&cfg.openai_key, &cfg.language))
        }
        config::TranscriptionMode::Local => {
            tracing::info!("Using local transcription (whisper-rs)");
            let model_path = transcription::local::LocalTranscriber::ensure_model(
                &cfg.data_dir,
                &cfg.whisper_model,
            )
            .await?;
            Arc::new(transcription::local::LocalTranscriber::new(&model_path, &cfg.language)?)
        }
    };

    // Create LLM agent and TTS service
    let agent = Arc::new(llm::LlmAgent::new(
        &cfg.openai_key,
        &cfg.openai_model,
        &cfg.bot_name,
        &cfg.trigger_phrase,
    ));
    let tts_service = Arc::new(tts::TtsService::new(&cfg.openai_key, &cfg.tts_voice));

    // Create transcript broadcast channel
    let (transcript_tx, _) = tokio::sync::broadcast::channel::<String>(256);

    // Create process manager
    let bot_process = Arc::new(Mutex::new(process::VexaBotProcess::new()));

    // Create AppState and start web server
    let app_state = Arc::new(server::AppState {
        config: tokio::sync::RwLock::new(cfg.clone()),
        transcript_tx: transcript_tx.clone(),
        bridge_state: bridge_state.clone(),
        agent: agent.clone(),
        bot_process: bot_process.clone(),
    });

    let web_router = server::router(app_state);
    let web_addr = format!("0.0.0.0:{}", cfg.port);
    tracing::info!("Web UI listening on {}", web_addr);
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&web_addr).await.unwrap();
        axum::serve(listener, web_router).await.unwrap();
    });

    // Open browser to Web UI
    open::that(format!("http://localhost:{}", cfg.port)).ok();

    // Spawn audio processing / transcription loop
    let bridge_for_tx = bridge_state.clone();
    let transcriber_clone = transcriber.clone();
    let agent_clone = agent.clone();
    let tts_clone = tts_service.clone();
    let transcript_tx_clone = transcript_tx.clone();
    tokio::spawn(async move {
        let mut audio_rx = bridge_for_tx.audio_rx.lock().await;
        let mut buffer: Vec<f32> = Vec::new();
        let chunk_size = 16000 * 3; // 3 seconds at 16kHz

        while let Some(samples) = audio_rx.recv().await {
            buffer.extend_from_slice(&samples);
            if buffer.len() >= chunk_size {
                let chunk = buffer.drain(..chunk_size).collect::<Vec<_>>();

                // Skip silent chunks to avoid Whisper hallucinations
                let rms = transcription::audio_rms(&chunk);
                if rms < transcription::SILENCE_RMS_THRESHOLD {
                    continue;
                }

                match transcriber_clone.transcribe(&chunk).await {
                    Ok(Some(seg)) => {
                        // Filter Whisper hallucinations (repeated words on low audio)
                        if transcription::is_hallucination(&seg.text) {
                            tracing::debug!("[transcript] filtered hallucination: {}", seg.text);
                            continue;
                        }
                        tracing::info!("[transcript] {}", seg.text);

                        // Broadcast transcript line to WebSocket clients
                        let _ = transcript_tx_clone.send(seg.text.clone());

                        // Feed transcript to LLM agent
                        agent_clone.add_transcript(&seg.text);

                        // Check if the speaker is addressing the bot
                        if let Some(question) = agent_clone.should_respond(&seg.text).await {
                            tracing::info!("[llm] detected question: {}", question);

                            match agent_clone.respond(&question).await {
                                Ok(answer) => {
                                    tracing::info!("[llm] response: {}", answer);

                                    // Synthesize TTS audio
                                    match tts_clone.synthesize(&answer).await {
                                        Ok(wav_bytes) => {
                                            let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&wav_bytes);
                                            let msg = bot_bridge::CoreMessage::Speak { audio: audio_b64 };
                                            if let Err(e) = bridge_for_tx.command_tx.send(msg) {
                                                tracing::error!("[tts] failed to send audio to bridge: {}", e);
                                            }
                                        }
                                        Err(e) => tracing::error!("[tts] synthesis error: {}", e),
                                    }
                                }
                                Err(e) => tracing::error!("[llm] response error: {}", e),
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => tracing::error!("[transcript] error: {}", e),
                }
            }
        }
    });

    // Start vexa-bot process if meet_url is provided on startup
    if let Some(ref meet_url) = cfg.meet_url {
        match process::find_node() {
            Ok(node_path) => {
                match process::find_vexa_bot_dir() {
                    Ok(vexa_bot_dir) => {
                        let bridge_url = format!("ws://localhost:{}/ws", cfg.bridge_port);
                        let mut proc = bot_process.lock().unwrap();
                        if let Err(e) = proc.start(
                            &node_path,
                            &vexa_bot_dir,
                            &bridge_url,
                            meet_url,
                            &cfg.bot_name,
                        ) {
                            tracing::error!("[process] failed to start vexa-bot: {}", e);
                        }
                    }
                    Err(e) => tracing::warn!("[process] vexa-bot dir not found: {}", e),
                }
            }
            Err(e) => tracing::warn!("[process] node not found: {}", e),
        }
    }

    tracing::info!("Jarvis running. Press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down...");

    // Stop vexa-bot on shutdown
    if let Ok(mut proc) = bot_process.lock() {
        let _ = proc.stop();
    }

    Ok(())
}
