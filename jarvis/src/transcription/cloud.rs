use super::{TranscriptSegment, Transcriber};
use async_trait::async_trait;
use reqwest::multipart;

pub struct CloudTranscriber {
    client: reqwest::Client,
    api_key: String,
    language: Option<String>,
}

impl CloudTranscriber {
    pub fn new(api_key: &str, language: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            language: if language == "auto" { None } else { Some(language.to_string()) },
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

        let mut form = multipart::Form::new()
            .text("model", "whisper-1")
            .text("response_format", "json")
            .part("file", part);

        if let Some(ref lang) = self.language {
            form = form.text("language", lang.clone());
        }

        let resp = self
            .client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI Whisper API error ({}): {}", status, error_body);
        }

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
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());

    for &s in samples {
        let i = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&i.to_le_bytes());
    }
    buf
}
