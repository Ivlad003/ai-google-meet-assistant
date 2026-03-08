use base64::Engine;
use clap::Parser;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

mod bot_bridge;
mod config;
mod db;
mod llm;
mod process;
mod server;
mod tools;
mod transcription;
mod tts;

#[derive(Parser, Debug)]
#[command(name = "jarvis", about = "AI Meeting Assistant")]
struct Args {
    /// Path to JSON config file (required)
    #[arg(long, env = "JARVIS_CONFIG", default_value = "jarvis.config.json")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config_file = config::ConfigFile::load(&args.config).unwrap_or_else(|e| {
        eprintln!("Error: failed to load config '{}': {}", args.config, e);
        eprintln!("Create one from jarvis.config.example.json");
        std::process::exit(1);
    });
    let cfg = config::Config::from_file(&config_file);

    // Set up file logging
    let logs_dir = cfg.data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).ok();
    let file_appender = tracing_appender::rolling::daily(&logs_dir, "jarvis.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("jarvis=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking),
        )
        .init();

    tracing::info!("Jarvis v{} starting...", env!("CARGO_PKG_VERSION"));
    tracing::info!("Logs dir: {}", logs_dir.display());
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
        &cfg.intent_model,
        cfg.system_prompt.as_deref(),
        cfg.intent_prompt.clone(),
        cfg.max_response_tokens,
        cfg.temperature,
        &cfg.tools,
    ));
    let tts_service = Arc::new(tts::TtsService::new(&cfg.openai_key, &cfg.tts_voice));

    // Create transcript broadcast channel
    let (transcript_tx, _) = tokio::sync::broadcast::channel::<String>(256);

    // Create session transcript and audio files
    let sessions_dir = cfg.data_dir.join("sessions");
    std::fs::create_dir_all(&sessions_dir).ok();
    let session_ts = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
    let session_transcript_path = sessions_dir.join(format!("{}.txt", session_ts));
    let session_audio_path = sessions_dir.join(format!("{}.wav", session_ts));
    tracing::info!("Session transcript: {}", session_transcript_path.display());
    tracing::info!("Session audio: {}", session_audio_path.display());
    let session_file = Arc::new(tokio::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&session_transcript_path)?,
    ));

    // WAV writer for audio recording (16kHz mono 16-bit PCM)
    let wav_spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let wav_writer = Arc::new(tokio::sync::Mutex::new(
        hound::WavWriter::create(&session_audio_path, wav_spec)?,
    ));

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
    let session_file_clone = session_file.clone();
    let wav_writer_clone = wav_writer.clone();
    let tools_list = Arc::new(cfg.tools.clone());
    let http_client = Arc::new(reqwest::Client::new());
    tokio::spawn(async move {
        let mut audio_rx = bridge_for_tx.audio_rx.lock().await;
        let mut buffer: Vec<f32> = Vec::new();
        let chunk_size = 16000 * 3; // 3 seconds at 16kHz

        while let Some(samples) = audio_rx.recv().await {
            // Write all audio samples to WAV file
            {
                let mut writer = wav_writer_clone.lock().await;
                for &s in &samples {
                    let sample_i16 = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    let _ = writer.write_sample(sample_i16);
                }
            }

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
                        // Get current speaker name
                        let speaker = bridge_for_tx.current_speaker.lock().await.clone();
                        let speaker_label = speaker.as_deref().unwrap_or("Unknown");

                        tracing::info!("[transcript] [{}] {}", speaker_label, seg.text);

                        // Write to session transcript file
                        {
                            use std::io::Write;
                            let ts = chrono::Local::now().format("%H:%M:%S");
                            let mut f = session_file_clone.lock().await;
                            let _ = writeln!(f, "[{}] [{}] {}", ts, speaker_label, seg.text);
                            let _ = f.flush();
                        }

                        // Broadcast transcript line to WebSocket clients
                        let line = format!("[{}] {}", speaker_label, seg.text);
                        let _ = transcript_tx_clone.send(line);

                        // Feed transcript to LLM agent with speaker info
                        agent_clone.add_transcript(&speaker_label, &seg.text);

                        // Check if the speaker is addressing the bot
                        if let Some(question) = agent_clone.should_respond(&speaker_label, &seg.text).await {
                            tracing::info!("[llm] detected question: {}", question);

                            match agent_clone.respond(&question).await {
                                Ok(answer) => {
                                    tracing::info!("[llm] response: {}", answer);

                                    // Check if LLM wants to use a tool
                                    if let Some((tool_name, params)) = tools::parse_tool_call(&answer) {
                                        if let Some(tool) = tools_list.iter().find(|t| t.name == tool_name) {
                                            tracing::info!("[tools] executing: {} with {:?}", tool_name, params);

                                            // Spawn tool execution in a separate task so it
                                            // doesn't block the transcription loop
                                            let tool = tool.clone();
                                            let http_client = http_client.clone();
                                            let agent_bg = agent_clone.clone();
                                            let tts_bg = tts_clone.clone();
                                            let session_file_bg = session_file_clone.clone();
                                            let bridge_bg = bridge_for_tx.clone();
                                            tokio::spawn(async move {
                                                let result = tools::execute_tool(&tool, &params, &http_client).await;
                                                tracing::info!("[tools] result: success={}, output={}", result.success, result.output);

                                                let spoken_text = match agent_bg.summarize_tool_result(&result.tool_name, result.success, &result.output).await {
                                                    Ok(summary) => {
                                                        agent_bg.add_tool_context(&result.tool_name, &summary);
                                                        summary
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("[tools] summarize error: {}", e);
                                                        format!("Tool {} completed but I couldn't summarize the result.", result.tool_name)
                                                    }
                                                };

                                                // Write bot response to session file
                                                {
                                                    use std::io::Write;
                                                    let ts = chrono::Local::now().format("%H:%M:%S");
                                                    let mut f = session_file_bg.lock().await;
                                                    let _ = writeln!(f, "[{}] [Jarvis] {}", ts, spoken_text);
                                                    let _ = f.flush();
                                                }

                                                // Synthesize TTS audio
                                                match tts_bg.synthesize(&spoken_text).await {
                                                    Ok(wav_bytes) => {
                                                        let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&wav_bytes);
                                                        let msg = bot_bridge::CoreMessage::Speak { audio: audio_b64 };
                                                        if let Err(e) = bridge_bg.command_tx.send(msg) {
                                                            tracing::error!("[tts] failed to send audio to bridge: {}", e);
                                                        }
                                                    }
                                                    Err(e) => tracing::error!("[tts] synthesis error: {}", e),
                                                }
                                            });
                                        } else {
                                            tracing::warn!("[tools] unknown tool: {}", tool_name);
                                            let spoken_text = format!("I don't have a tool called {}.", tool_name);

                                            // Write + TTS for unknown tool
                                            {
                                                use std::io::Write;
                                                let ts = chrono::Local::now().format("%H:%M:%S");
                                                let mut f = session_file_clone.lock().await;
                                                let _ = writeln!(f, "[{}] [Jarvis] {}", ts, spoken_text);
                                                let _ = f.flush();
                                            }
                                            match tts_clone.synthesize(&spoken_text).await {
                                                Ok(wav_bytes) => {
                                                    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&wav_bytes);
                                                    let msg = bot_bridge::CoreMessage::Speak { audio: audio_b64 };
                                                    let _ = bridge_for_tx.command_tx.send(msg);
                                                }
                                                Err(e) => tracing::error!("[tts] synthesis error: {}", e),
                                            }
                                        }
                                    } else {
                                        // Normal response (no tool call)
                                        // Track bot response in transcript for context
                                        agent_clone.add_bot_response_to_transcript(&answer);

                                        // Write bot response to session file
                                        {
                                            use std::io::Write;
                                            let ts = chrono::Local::now().format("%H:%M:%S");
                                            let mut f = session_file_clone.lock().await;
                                            let _ = writeln!(f, "[{}] [Jarvis] {}", ts, answer);
                                            let _ = f.flush();
                                        }

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

    // Finalize WAV file (write header with correct data length)
    if let Ok(writer) = Arc::try_unwrap(wav_writer) {
        let writer = writer.into_inner();
        let _ = writer.finalize();
    }

    // Print session file paths to terminal
    println!();
    println!("=== Session Complete ===");
    println!("Transcript: {}", session_transcript_path.display());
    println!("Audio:      {}", session_audio_path.display());
    println!("Logs:       {}", logs_dir.display());
    println!("========================");

    Ok(())
}
