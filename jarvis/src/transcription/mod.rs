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
/// Whisper hallucinates on silence/noise: YouTube outros, repetitive words, music notations, etc.
pub fn is_hallucination(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_lowercase();

    // --- Exact match hallucinations ---
    let exact = [
        "you", "the", "a", "i", ".", "..", "...", "bye", "bye bye",
        "thank you", "thanks", "thanks for watching", "subscribe",
        "like and subscribe", "see you next time", "see you",
        "okay", "ok", "so", "yeah", "yes", "no", "hmm", "um", "uh",
        // Ukrainian
        "дякую", "дякую за перегляд", "дякую за перегляд!",
        "до зустрічі", "до побачення", "бувайте",
        "до зустрічі в наступному відео", "до зустрічі в наступному відео!",
        "підписуйтесь", "підписуйтесь на канал",
        "дякую за увагу", "дякую за увагу!",
        // Russian (Whisper sometimes outputs Russian for Ukrainian audio)
        "спасибо за просмотр", "спасибо", "до свидания",
        "подписывайтесь", "подписывайтесь на канал",
        "до встречи", "до встречи в следующем видео",
    ];
    if exact.contains(&lower.as_str()) {
        return true;
    }

    // --- Substring patterns (YouTube/podcast outros) ---
    let substrings = [
        "thanks for watching", "thank you for watching",
        "like and subscribe", "subscribe to", "hit the bell",
        "see you in the next", "see you next time",
        "дякую за перегляд", "до зустрічі в наступному",
        "підписуйтесь на канал", "ставте лайк",
        "спасибо за просмотр", "до встречи в следующем",
        "подписывайтесь на канал", "ставьте лайк",
        "©", "♪", "♫", "🎵",
        "subtitles by", "captions by", "transcribed by",
        "music playing", "music]", "[music",
    ];
    for pat in &substrings {
        if lower.contains(pat) {
            return true;
        }
    }

    // --- Repetitive single-word pattern: "you you you", "the the the" ---
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() >= 2 {
        let first = words[0];
        if words.iter().all(|w| *w == first) {
            return true;
        }
    }

    // --- Too short after stripping punctuation (likely noise artifact) ---
    let alpha_chars: usize = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
    if alpha_chars < 2 {
        return true;
    }

    false
}
