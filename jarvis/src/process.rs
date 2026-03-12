use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub struct VexaBotProcess {
    child: Option<Child>,
}

impl VexaBotProcess {
    pub fn new() -> Self {
        Self { child: None }
    }

    /// Start vexa-bot as a child process. Kills any existing process first.
    pub fn start(
        &mut self,
        node_path: &PathBuf,
        vexa_bot_dir: &PathBuf,
        bridge_url: &str,
        meet_url: &str,
        bot_name: &str,
        record_video: bool,
        video_output_path: &str,
        ffmpeg_available: bool,
    ) -> anyhow::Result<()> {
        if self.is_running() {
            self.stop()?;
        }

        // vexa-bot's entry point is docker.js which reads BOT_CONFIG JSON env var
        let entry_point = vexa_bot_dir.join("core/dist/docker.js");
        if !entry_point.exists() {
            anyhow::bail!(
                "vexa-bot entry point not found at {:?}. Make sure vexa-bot is built (npx tsc in core/).",
                entry_point
            );
        }

        // Extract native meeting ID from URL (e.g. "abc-defg-hij" from Google Meet URL)
        let native_id = meet_url
            .rsplit('/')
            .next()
            .unwrap_or("unknown")
            .split('?')
            .next()
            .unwrap_or("unknown");

        // Construct BOT_CONFIG JSON matching the Zod schema in docker.ts
        let bot_config = serde_json::json!({
            "platform": "google_meet",
            "meetingUrl": meet_url,
            "botName": bot_name,
            "token": "bridge-mode",
            "connectionId": format!("bridge-{}", uuid::Uuid::new_v4()),
            "nativeMeetingId": native_id,
            "redisUrl": "redis://unused:6379",
            "automaticLeave": {
                "waitingRoomTimeout": 120,
                "noOneJoinedTimeout": 300,
                "everyoneLeftTimeout": 60
            },
            "meeting_id": 1,
            "voiceAgentEnabled": true,
            "recordingEnabled": true,
            "captureModes": ["audio"]
        });

        let headless = std::env::var("HEADLESS").unwrap_or_else(|_| "false".to_string());
        let mut child = Command::new(node_path)
            .arg(&entry_point)
            .env("BOT_CONFIG", bot_config.to_string())
            .env("BRIDGE_URL", bridge_url)
            .env("HEADLESS", &headless)
            .env("RECORD_VIDEO", if record_video { "true" } else { "false" })
            .env("VIDEO_OUTPUT_PATH", video_output_path)
            .env("FFMPEG_AVAILABLE", if ffmpeg_available { "true" } else { "false" })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        tracing::info!("[process] vexa-bot started (pid: {:?})", child.id());

        // Relay child stdout/stderr to tracing so it shows in Jarvis logs
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::info!("[vexa-bot] {}", line);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::warn!("[vexa-bot] {}", line);
                }
            });
        }

        self.child = Some(child);
        Ok(())
    }

    /// Stop the vexa-bot process by sending a kill signal.
    pub fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(ref mut child) = self.child {
            child.start_kill()?;
            tracing::info!("[process] vexa-bot stopped");
        }
        self.child = None;
        Ok(())
    }

    /// Check if the vexa-bot process is still running.
    pub fn is_running(&mut self) -> bool {
        match self.child {
            Some(ref mut child) => {
                // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running
                match child.try_wait() {
                    Ok(Some(_)) => {
                        self.child = None;
                        false
                    }
                    Ok(None) => true,
                    Err(_) => false,
                }
            }
            None => false,
        }
    }
}

impl Drop for VexaBotProcess {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
        }
    }
}

/// Find Node.js binary. Checks:
/// 1. Bundled path next to the jarvis binary
/// 2. System PATH (using `which` on Unix, `where` on Windows)
pub fn find_node() -> anyhow::Result<PathBuf> {
    // Check bundled node next to binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let bundled = exe_dir.join("node");
            if bundled.exists() {
                tracing::info!("[process] using bundled node at {:?}", bundled);
                return Ok(bundled);
            }
        }
    }

    // Check system PATH
    let which_cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };

    if let Ok(output) = std::process::Command::new(which_cmd).arg("node").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path.is_empty() {
                let node_path = PathBuf::from(&path);
                tracing::info!("[process] using system node at {:?}", node_path);
                return Ok(node_path);
            }
        }
    }

    anyhow::bail!(
        "Node.js not found. Install Node.js or place a 'node' binary next to the jarvis executable."
    )
}

/// Find vexa-bot directory. Checks:
/// 1. Next to the jarvis binary (packaged mode)
/// 2. Relative to project root (development mode)
pub fn find_vexa_bot_dir() -> anyhow::Result<PathBuf> {
    // Check next to binary (packaged mode)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let packaged = exe_dir.join("vexa-bot");
            if packaged.exists() {
                tracing::info!("[process] using packaged vexa-bot at {:?}", packaged);
                return Ok(packaged);
            }
        }
    }

    // Development fallback: ../services/vexa-bot relative to working directory
    let dev_path = PathBuf::from("../services/vexa-bot");
    if dev_path.exists() {
        let canonical = dev_path.canonicalize()?;
        tracing::info!("[process] using dev vexa-bot at {:?}", canonical);
        return Ok(canonical);
    }

    // Also try services/vexa-bot from project root
    let alt_path = PathBuf::from("services/vexa-bot");
    if alt_path.exists() {
        let canonical = alt_path.canonicalize()?;
        tracing::info!("[process] using dev vexa-bot at {:?}", canonical);
        return Ok(canonical);
    }

    anyhow::bail!(
        "vexa-bot directory not found. Place it next to the jarvis binary or run from the project root."
    )
}
