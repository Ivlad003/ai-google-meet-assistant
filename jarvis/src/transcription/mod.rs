pub mod cloud;
pub mod local;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct TranscriptSegment {
    pub text: String,
    pub language: Option<String>,
}

#[async_trait]
pub trait Transcriber: Send + Sync {
    /// Transcribe a chunk of Float32 PCM audio at 16kHz mono
    async fn transcribe(&self, audio: &[f32]) -> anyhow::Result<Option<TranscriptSegment>>;
}

/// Compute RMS (root mean square) of audio samples.
pub fn audio_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Silence threshold — below this RMS, audio is considered silence.
/// Typical speech RMS is 0.01–0.1; background noise is <0.005.
pub const SILENCE_RMS_THRESHOLD: f32 = 0.005;

/// Check if transcription text is a Whisper hallucination.
/// Whisper hallucinates repetitive single words on silence: "you you you", "the the the", etc.
pub fn is_hallucination(text: &str) -> bool {
    let trimmed = text.trim().to_lowercase();
    if trimmed.is_empty() {
        return true;
    }

    // Common Whisper hallucination phrases
    let hallucination_phrases = [
        "thank you", "thanks for watching", "subscribe",
        "like and subscribe", "see you next time", "bye",
        "you", "the", "a", "i", ".", "..",
    ];

    // Check exact match with known hallucinations
    if hallucination_phrases.contains(&trimmed.as_str()) {
        return true;
    }

    // Check for repetitive single-word pattern: "you you you", "the the the"
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() >= 2 {
        let first = words[0];
        if words.iter().all(|w| *w == first) {
            return true;
        }
    }

    false
}
