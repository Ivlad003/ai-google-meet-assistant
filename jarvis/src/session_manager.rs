use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::Mutex;

/// Information about an active session, returned to callers that need to know
/// the session id / file paths (e.g. for spawning the bot with a video path).
#[derive(Clone, Debug)]
pub struct ActiveSessionInfo {
    pub id: String,
    pub transcript_path: PathBuf,
    pub audio_path: PathBuf,
    pub video_path: Option<PathBuf>,
}

/// Internal representation of an open session. Holds live file/writer handles.
struct ActiveSession {
    info: ActiveSessionInfo,
    transcript_file: std::fs::File,
    wav_writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
}

impl ActiveSession {
    /// Flush transcript and finalize WAV writer. Idempotent.
    fn finalize(&mut self) {
        if let Some(writer) = self.wav_writer.take() {
            if let Err(e) = writer.finalize() {
                tracing::warn!(
                    "[session] failed to finalize WAV for {}: {}",
                    self.info.id,
                    e
                );
            }
        }
        use std::io::Write;
        let _ = self.transcript_file.flush();
    }
}

/// Manages the lifecycle of recording sessions. Each call to `start_new`
/// finalizes the previous session and opens fresh transcript/audio/video paths
/// keyed off a new timestamp.
pub struct SessionManager {
    sessions_dir: PathBuf,
    current: Mutex<Option<ActiveSession>>,
}

impl SessionManager {
    pub fn new(sessions_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            sessions_dir,
            current: Mutex::new(None),
        })
    }

    /// Start a new session. Finalizes any previous session first.
    pub async fn start_new(&self, record_video: bool) -> anyhow::Result<ActiveSessionInfo> {
        let mut guard = self.current.lock().await;
        if let Some(mut old) = guard.take() {
            tracing::info!("[session] finalizing previous session: {}", old.info.id);
            old.finalize();
        }

        std::fs::create_dir_all(&self.sessions_dir).ok();

        let ts = chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string();
        let transcript_path = self.sessions_dir.join(format!("{}.txt", ts));
        let audio_path = self.sessions_dir.join(format!("{}.wav", ts));
        let video_path = if record_video {
            Some(self.sessions_dir.join(format!("{}.webm", ts)))
        } else {
            None
        };

        let transcript_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&transcript_path)
            .with_context(|| format!("Failed to open transcript {:?}", transcript_path))?;

        let wav_spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav_writer = hound::WavWriter::create(&audio_path, wav_spec)
            .with_context(|| format!("Failed to create WAV {:?}", audio_path))?;

        let info = ActiveSessionInfo {
            id: ts.clone(),
            transcript_path,
            audio_path,
            video_path,
        };

        tracing::info!("[session] started new session: {}", ts);
        if let Some(ref vp) = info.video_path {
            tracing::info!("[session]   video target: {}", vp.display());
        }

        *guard = Some(ActiveSession {
            info: info.clone(),
            transcript_file,
            wav_writer: Some(wav_writer),
        });

        Ok(info)
    }

    /// End the current session, finalizing files. Returns the finalized session info.
    pub async fn end_current(&self) -> Option<ActiveSessionInfo> {
        let mut guard = self.current.lock().await;
        if let Some(mut old) = guard.take() {
            tracing::info!("[session] ending session: {}", old.info.id);
            let info = old.info.clone();
            old.finalize();
            return Some(info);
        }
        None
    }

    /// Append PCM samples to the active session's WAV file. No-op if no session.
    pub async fn write_audio_samples(&self, samples: &[f32]) {
        let mut guard = self.current.lock().await;
        if let Some(s) = guard.as_mut() {
            if let Some(w) = s.wav_writer.as_mut() {
                for &sample in samples {
                    let i16_sample = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    let _ = w.write_sample(i16_sample);
                }
            }
        }
    }

    /// Append a single line to the active session's transcript file.
    pub async fn write_transcript_line(&self, line: &str) {
        let mut guard = self.current.lock().await;
        if let Some(s) = guard.as_mut() {
            use std::io::Write;
            let _ = writeln!(s.transcript_file, "{}", line);
            let _ = s.transcript_file.flush();
        }
    }

    /// Snapshot of the active session, if any.
    pub async fn current_info(&self) -> Option<ActiveSessionInfo> {
        self.current.lock().await.as_ref().map(|s| s.info.clone())
    }
}
