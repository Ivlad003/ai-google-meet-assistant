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

    /// Synthesize text to WAV audio bytes using OpenAI TTS API.
    pub async fn synthesize(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        let req = TtsRequest {
            model: "tts-1".to_string(),
            input: text.to_string(),
            voice: self.voice.clone(),
            response_format: "wav".to_string(),
        };

        let resp = self
            .client
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
