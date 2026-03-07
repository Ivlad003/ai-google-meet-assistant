use super::{TranscriptSegment, Transcriber};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct LocalTranscriber {
    ctx: Mutex<WhisperContext>,
    language: String,
}

impl LocalTranscriber {
    pub fn new(model_path: &PathBuf, language: &str) -> anyhow::Result<Self> {
        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(model_path.to_str().unwrap(), params)
            .map_err(|e| anyhow::anyhow!("whisper init failed: {:?}", e))?;

        Ok(Self {
            ctx: Mutex::new(ctx),
            language: language.to_string(),
        })
    }

    /// Download model if not present. Returns path.
    pub async fn ensure_model(data_dir: &PathBuf, model_name: &str) -> anyhow::Result<PathBuf> {
        let models_dir = data_dir.join("models");
        std::fs::create_dir_all(&models_dir)?;

        let filename = format!("ggml-{}.bin", model_name);
        let model_path = models_dir.join(&filename);

        if model_path.exists() {
            return Ok(model_path);
        }

        let url = format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            filename
        );
        tracing::info!("Downloading whisper model {}...", model_name);

        let resp = reqwest::get(&url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to download whisper model '{}': HTTP {}", model_name, resp.status());
        }
        let bytes = resp.bytes().await?;
        std::fs::write(&model_path, &bytes)?;

        tracing::info!("Model downloaded to {:?}", model_path);
        Ok(model_path)
    }
}

#[async_trait]
impl Transcriber for LocalTranscriber {
    async fn transcribe(&self, audio: &[f32]) -> anyhow::Result<Option<TranscriptSegment>> {
        if audio.len() < 8000 {
            return Ok(None);
        }

        let audio = audio.to_vec();
        let ctx = &self.ctx;
        let language = self.language.clone();

        // whisper-rs is not async, run in blocking thread
        // We need to get a reference that can be sent across threads.
        // Since Mutex<WhisperContext> is behind &self and we can't send &Mutex across,
        // we lock here and do the work in spawn_blocking with the state.
        let mut state = {
            let ctx_guard = ctx.lock().unwrap();
            ctx_guard
                .create_state()
                .map_err(|e| anyhow::anyhow!("whisper state: {:?}", e))?
        };

        tokio::task::spawn_blocking(move || {
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(4);
            params.set_language(Some(&language));
            params.set_token_timestamps(false);

            state
                .full(params, &audio)
                .map_err(|e| anyhow::anyhow!("whisper full: {:?}", e))?;

            let num_segments = state
                .full_n_segments()
                .map_err(|e| anyhow::anyhow!("segments: {:?}", e))?;

            let mut text = String::new();
            for i in 0..num_segments {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    text.push_str(&seg);
                }
            }

            let text = text.trim().to_string();
            if text.is_empty() {
                return Ok(None);
            }

            Ok(Some(TranscriptSegment {
                text,
                language: None,
            }))
        })
        .await?
    }
}
