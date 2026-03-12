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
    #[serde(default)]
    pub tools: Option<Vec<super::tools::ToolDef>>,
}

impl ConfigFile {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Self = serde_json::from_str(&contents)?;
        Ok(config)
    }

    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
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
            tools: cf.tools.clone().unwrap_or_default(),
        }
    }
}

fn dirs_or_default() -> PathBuf {
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
