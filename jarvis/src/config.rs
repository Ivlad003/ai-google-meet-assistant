use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// JSON config file structure — all settings live here
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    pub openai_key: Option<String>,
    pub meet_url: Option<String>,
    pub bot_name: Option<String>,
    pub transcription_mode: Option<String>,
    pub whisper_model: Option<String>,
    pub port: Option<u16>,
    pub bridge_port: Option<u16>,
    pub tts_voice: Option<String>,
    pub openai_model: Option<String>,
    pub language: Option<String>,
    pub intent_model: Option<String>,
    pub system_prompt: Option<String>,
    pub intent_prompt: Option<String>,
    pub max_response_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub response_mode: Option<String>,
    pub record_video: Option<bool>,
    #[serde(default)]
    pub tools: Option<Vec<super::tools::ToolDef>>,
}

impl ConfigFile {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let mut config: Self = serde_json::from_str(&contents)?;
        config.override_from_env();
        Ok(config)
    }

    /// Load config purely from environment variables (no JSON file needed).
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.override_from_env();
        config
    }

    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Override config fields from environment variables.
    /// Convention: JARVIS_<FIELD_NAME> in uppercase.
    /// Env vars take priority over JSON file values.
    fn override_from_env(&mut self) {
        fn env_str(key: &str) -> Option<String> {
            std::env::var(key).ok().filter(|s| !s.is_empty())
        }
        fn env_u16(key: &str) -> Option<u16> {
            std::env::var(key).ok().and_then(|s| s.parse().ok())
        }
        fn env_u32(key: &str) -> Option<u32> {
            std::env::var(key).ok().and_then(|s| s.parse().ok())
        }
        fn env_f32(key: &str) -> Option<f32> {
            std::env::var(key).ok().and_then(|s| s.parse().ok())
        }
        fn env_bool(key: &str) -> Option<bool> {
            std::env::var(key).ok().map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
        }

        // Also support OPENAI_API_KEY as a common convention
        if let Some(v) = env_str("JARVIS_OPENAI_KEY").or_else(|| env_str("OPENAI_API_KEY")) {
            self.openai_key = Some(v);
        }
        if let Some(v) = env_str("JARVIS_MEET_URL") { self.meet_url = Some(v); }
        if let Some(v) = env_str("JARVIS_BOT_NAME") { self.bot_name = Some(v); }
        if let Some(v) = env_str("JARVIS_TRANSCRIPTION_MODE") { self.transcription_mode = Some(v); }
        if let Some(v) = env_str("JARVIS_WHISPER_MODEL") { self.whisper_model = Some(v); }
        if let Some(v) = env_u16("JARVIS_PORT") { self.port = Some(v); }
        if let Some(v) = env_u16("JARVIS_BRIDGE_PORT") { self.bridge_port = Some(v); }
        if let Some(v) = env_str("JARVIS_TTS_VOICE") { self.tts_voice = Some(v); }
        if let Some(v) = env_str("JARVIS_OPENAI_MODEL") { self.openai_model = Some(v); }
        if let Some(v) = env_str("JARVIS_LANGUAGE") { self.language = Some(v); }
        if let Some(v) = env_str("JARVIS_INTENT_MODEL") { self.intent_model = Some(v); }
        if let Some(v) = env_str("JARVIS_SYSTEM_PROMPT") { self.system_prompt = Some(v); }
        if let Some(v) = env_str("JARVIS_INTENT_PROMPT") { self.intent_prompt = Some(v); }
        if let Some(v) = env_u32("JARVIS_MAX_RESPONSE_TOKENS") { self.max_response_tokens = Some(v); }
        if let Some(v) = env_f32("JARVIS_TEMPERATURE") { self.temperature = Some(v); }
        if let Some(v) = env_str("JARVIS_RESPONSE_MODE") { self.response_mode = Some(v); }
        if let Some(v) = env_bool("JARVIS_RECORD_VIDEO") { self.record_video = Some(v); }
        // Tools from env: JSON array string
        if let Some(v) = env_str("JARVIS_TOOLS") {
            if let Ok(tools) = serde_json::from_str(&v) {
                self.tools = Some(tools);
            }
        }
    }
}

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
    pub db_path: PathBuf,
    pub data_dir: PathBuf,
    pub language: String,
    pub intent_model: String,
    pub system_prompt: Option<String>,
    pub intent_prompt: Option<String>,
    pub max_response_tokens: u32,
    pub temperature: f32,
    pub response_mode: ResponseMode,
    pub record_video: bool,
    pub tools: Vec<super::tools::ToolDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptionMode {
    Local,
    Cloud,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseMode {
    Smart,
    NameOnly,
}

impl Config {
    /// Build config from JSON config file with sensible defaults
    pub fn from_file(cf: &ConfigFile) -> Self {
        let data_dir = dirs_or_default();
        std::fs::create_dir_all(&data_dir).ok();

        let openai_key = cf.openai_key.clone()
            .unwrap_or_else(|| {
                eprintln!("Error: 'openai_key' is required in config JSON");
                std::process::exit(1);
            });
        let transcription = cf.transcription_mode.clone().unwrap_or_else(|| "cloud".to_string());

        Self {
            openai_key,
            meet_url: cf.meet_url.clone(),
            bot_name: cf.bot_name.clone().unwrap_or_else(|| "Jarvis".to_string()),
            transcription_mode: match transcription.as_str() {
                "local" => TranscriptionMode::Local,
                _ => TranscriptionMode::Cloud,
            },
            whisper_model: cf.whisper_model.clone().unwrap_or_else(|| "small".to_string()),
            port: cf.port.unwrap_or(8080),
            bridge_port: cf.bridge_port.unwrap_or(9090),
            tts_voice: cf.tts_voice.clone().unwrap_or_else(|| "nova".to_string()),
            openai_model: cf.openai_model.clone().unwrap_or_else(|| "gpt-5.4".to_string()),
            db_path: data_dir.join("jarvis.db"),
            data_dir,
            language: cf.language.clone().unwrap_or_else(|| "auto".to_string()),
            intent_model: cf.intent_model.clone().unwrap_or_else(|| "gpt-5".to_string()),
            system_prompt: cf.system_prompt.clone(),
            intent_prompt: cf.intent_prompt.clone(),
            max_response_tokens: cf.max_response_tokens.unwrap_or(150),
            temperature: cf.temperature.unwrap_or(0.7),
            response_mode: match cf.response_mode.as_deref() {
                Some("name_only") => ResponseMode::NameOnly,
                _ => ResponseMode::Smart,
            },
            record_video: cf.record_video.unwrap_or(false),
            tools: cf.tools.clone().unwrap_or_default(),
        }
    }
}

fn dirs_or_default() -> PathBuf {
    // JARVIS_DATA_DIR env var takes priority (Docker deployments)
    if let Ok(dir) = std::env::var("JARVIS_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jarvis")
}

/// Check if a model name is a reasoning model (no temperature, uses reasoning_effort).
pub fn is_reasoning_model(model: &str) -> bool {
    model.starts_with("gpt-5")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
}
