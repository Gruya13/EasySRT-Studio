//! Lifecycle manager for local `llama-server` (`llama.cpp` OpenAI-compatible server).
//!
//! Guarantees that `llama-server` is only active during the translation/polishing stage
//! (never overlapping with Whisper ASR) and is unconditionally terminated and reaped on
//! `stop()` or `Drop` so 100% of GPU VRAM is released.

use anyhow::{anyhow, Context, Result};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct LlamaServerGuard {
    child: Option<Child>,
    port: u16,
    base_url: String,
}

/// Allocates an ephemeral free local TCP port on `127.0.0.1`.
pub fn allocate_free_port() -> Result<u16> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("Failed to bind ephemeral localhost TCP port")?;
    let port = listener
        .local_addr()
        .context("Failed to query ephemeral local address")?
        .port();
    drop(listener);
    Ok(port)
}

fn configure_std_command_env(cmd: &mut Command, binary_path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if let Some(parent) = binary_path.parent() {
        if !parent.as_os_str().is_empty() {
            cmd.current_dir(parent);
            let sep = if cfg!(windows) { ";" } else { ":" };
            let mut dirs = vec![parent.to_string_lossy().to_string()];
            if let Some(grandparent) = parent.parent() {
                if !grandparent.as_os_str().is_empty() && grandparent != parent {
                    dirs.push(grandparent.to_string_lossy().to_string());
                }
            }
            let prefix = dirs.join(sep);

            if let Ok(existing_path) = std::env::var("PATH") {
                cmd.env("PATH", format!("{prefix}{sep}{existing_path}"));
            } else {
                cmd.env("PATH", &prefix);
            }

            #[cfg(target_os = "linux")]
            {
                if let Ok(existing_ld) = std::env::var("LD_LIBRARY_PATH") {
                    cmd.env("LD_LIBRARY_PATH", format!("{prefix}:{existing_ld}"));
                } else {
                    cmd.env("LD_LIBRARY_PATH", &prefix);
                }
            }
        }
    }
}

impl LlamaServerGuard {
    /// Spawns `llama-server` on an ephemeral localhost port and polls `/health` until HTTP 200.
    pub async fn start(
        llama_bin: &Path,
        llm_model_path: &Path,
        context_window: u32,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::start_with_timeout(
            llama_bin,
            llm_model_path,
            context_window,
            cancel_flag,
            Duration::from_secs(60),
        )
        .await
    }

    pub async fn start_with_timeout(
        llama_bin: &Path,
        llm_model_path: &Path,
        context_window: u32,
        cancel_flag: &Arc<AtomicBool>,
        startup_timeout: Duration,
    ) -> Result<Self> {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Job cancelled"));
        }

        if !llama_bin.exists() {
            return Err(anyhow!(
                "llama-server binary not found at {}",
                llama_bin.to_string_lossy()
            ));
        }
        if !llm_model_path.exists() {
            return Err(anyhow!(
                "Local LLM GGUF model not found at {}",
                llm_model_path.to_string_lossy()
            ));
        }

        let port = allocate_free_port()?;
        let ctx_size = context_window.clamp(2048, 32768);

        let mut cmd = Command::new(llama_bin);
        configure_std_command_env(&mut cmd, llama_bin);
        cmd.arg("-m")
            .arg(llm_model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("-ngl")
            .arg("99")
            .arg("-c")
            .arg(ctx_size.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd.spawn().with_context(|| {
            format!(
                "Failed to spawn llama-server at {}",
                llama_bin.to_string_lossy()
            )
        })?;

        let base_url = format!("http://127.0.0.1:{port}/v1");
        let health_url = format!("http://127.0.0.1:{port}/health");
        let mut guard = Self {
            child: Some(child),
            port,
            base_url,
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .context("Failed to build HTTP client for llama-server health check")?;

        let deadline = Instant::now() + startup_timeout;

        loop {
            if cancel_flag.load(Ordering::SeqCst) {
                guard.stop();
                return Err(anyhow!("Job cancelled while starting llama-server"));
            }

            if let Some(child_ref) = guard.child.as_mut() {
                if let Some(status) = child_ref
                    .try_wait()
                    .context("Failed polling llama-server process")?
                {
                    guard.child = None;
                    return Err(anyhow!(
                        "llama-server exited prematurely during startup with status {status}"
                    ));
                }
            }

            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() {
                    return Ok(guard);
                }
            }

            if Instant::now() >= deadline {
                guard.stop();
                return Err(anyhow!(
                    "Timed out waiting for llama-server /health on port {}",
                    guard.port
                ));
            }

            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    /// Returns the OpenAI-compatible v1 base URL (`http://127.0.0.1:<port>/v1`).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the bound localhost TCP port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Explicitly kills and reaps the `llama-server` child process to free VRAM immediately.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for LlamaServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_valid_ephemeral_port() {
        let port = allocate_free_port().unwrap();
        assert!(port > 0);
    }

    #[cfg(unix)]
    #[test]
    fn guard_drop_kills_and_reaps_child_process() {
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();

        let mut guard = LlamaServerGuard {
            child: Some(child),
            port: 12345,
            base_url: "http://127.0.0.1:12345/v1".to_string(),
        };
        guard.stop();
        assert!(guard.child.is_none());

        // Process should no longer be running (or already reaped)
        let proc_stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        assert!(proc_stat.is_err());
    }
}
