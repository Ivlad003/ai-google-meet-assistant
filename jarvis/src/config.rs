use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// JSON config file structure — all fields optional, CLI/env override these
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
    pub trigger_phrase: Option<String>,
    pub language: Option<String>,
    pub intent_model: Option<String>,
    pub system_prompt: Option<String>,
    pub intent_prompt: Option<String>,
    pub max_response_tokens: Option<u32>,
    pub temperature: Option<f32>,
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
    pub tools: Vec<super::tools::ToolDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptionMode {
    Local,
    Cloud,
}

impl Config {
    /// Build config with priority: CLI args/env > JSON config file > defaults
    pub fn from_args(args: &super::Args, config_file: Option<&ConfigFile>) -> Self {
        let data_dir = dirs_or_default();
        std::fs::create_dir_all(&data_dir).ok();

        let cf = config_file.cloned().unwrap_or_default();

        // CLI arg wins, then JSON config, then panic
        let openai_key = args.openai_key.clone()
            .or(cf.openai_key.clone())
            .expect("OpenAI API key required: set OPENAI_API_KEY, --openai-key, or openai_key in config JSON");
        let meet_url = args.meet.clone().or(cf.meet_url);
        let bot_name = if args.bot_name != "Jarvis" {
            args.bot_name.clone()
        } else {
            cf.bot_name.unwrap_or_else(|| args.bot_name.clone())
        };
        let transcription = if args.transcription != "cloud" {
            args.transcription.clone()
        } else {
            cf.transcription_mode.unwrap_or_else(|| args.transcription.clone())
        };
        let whisper_model = if args.whisper_model != "small" {
            args.whisper_model.clone()
        } else {
            cf.whisper_model.unwrap_or_else(|| args.whisper_model.clone())
        };
        let port = if args.port != 8080 {
            args.port
        } else {
            cf.port.unwrap_or(args.port)
        };
        let bridge_port = cf.bridge_port.unwrap_or(9090);
        let tts_voice = if args.tts_voice != "nova" {
            args.tts_voice.clone()
        } else {
            cf.tts_voice.unwrap_or_else(|| args.tts_voice.clone())
        };
        let openai_model = if args.model != "gpt-5.4" {
            args.model.clone()
        } else {
            cf.openai_model.unwrap_or_else(|| args.model.clone())
        };
        let language = if args.language != "auto" {
            args.language.clone()
        } else {
            cf.language.unwrap_or_else(|| args.language.clone())
        };
        let intent_model = cf.intent_model.unwrap_or_else(|| "gpt-5".to_string());
        let max_response_tokens = cf.max_response_tokens.unwrap_or(150);
        let temperature = cf.temperature.unwrap_or(0.7);
        let tools = cf.tools.unwrap_or_default();

        Self {
            openai_key,
            meet_url,
            bot_name,
            transcription_mode: match transcription.as_str() {
                "local" => TranscriptionMode::Local,
                _ => TranscriptionMode::Cloud,
            },
            whisper_model,
            port,
            bridge_port,
            tts_voice,
            openai_model,
            db_path: data_dir.join("jarvis.db"),
            data_dir,
            language,
            intent_model,
            system_prompt: cf.system_prompt,
            intent_prompt: cf.intent_prompt,
            max_response_tokens,
            temperature,
            tools,
        }
    }
}

fn dirs_or_default() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jarvis")
}
