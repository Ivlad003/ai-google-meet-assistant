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
    pub language: String,
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
            language: args.language.clone(),
        }
    }
}

fn dirs_or_default() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jarvis")
}
