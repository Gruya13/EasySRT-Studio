//! Lifecycle manager for local `llama-server` (`llama.cpp` OpenAI-compatible server).
//!
//! Guarantees that `llama-server` is only active during the translation/polishing stage
//! (never overlapping with Whisper ASR) and is unconditionally terminated and reaped on
//! `stop()` or `Drop` so 100% of GPU VRAM is released.

use anyhow::{anyhow, Context, Result};
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
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
    log_path: Option<PathBuf>,
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

    let (dir, path_val, ld_val) = crate::pipeline::ffmpeg::build_binary_env_strings(binary_path);
    if let Some(parent) = dir {
        cmd.current_dir(parent);
    }
    if let Some(path) = path_val {
        cmd.env("PATH", path);
    }
    if let Some(ld) = ld_val {
        cmd.env("LD_LIBRARY_PATH", ld);
    }
}

fn read_last_non_empty_lines(path: &Path, max_lines: usize) -> String {
    let Ok(raw) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join(" | ")
}

async fn spawn_and_wait_healthy(
    llama_bin: &Path,
    llm_model_path: &Path,
    ctx_size: u32,
    force_cpu: bool,
    cancel_flag: &Arc<AtomicBool>,
    startup_timeout: Duration,
) -> Result<LlamaServerGuard> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    let port = allocate_free_port()?;
    let log_path = std::env::temp_dir().join(format!("easysrt-llama-server-{port}.log"));
    let stderr_stdio = File::create(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(4, 16))
        .unwrap_or(4);
    let threads_str = threads.to_string();

    let mut cmd = Command::new(llama_bin);
    configure_std_command_env(&mut cmd, llama_bin);
    cmd.arg("-m")
        .arg(llm_model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("-np")
        .arg("1")
        .arg("-c")
        .arg(ctx_size.to_string())
        .arg("-ctk")
        .arg("q8_0")
        .arg("-ctv")
        .arg("q8_0")
        .arg("-t")
        .arg(&threads_str)
        .arg("-tb")
        .arg(&threads_str);

    if force_cpu {
        cmd.arg("-ngl").arg("0");
    } else {
        cmd.arg("-fitc")
            .arg("2048")
            .arg("-fitt")
            .arg("128");
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr_stdio);

    let child = cmd.spawn().with_context(|| {
        let _ = fs::remove_file(&log_path);
        format!(
            "Failed to spawn llama-server at {}",
            llama_bin.to_string_lossy()
        )
    })?;

    let base_url = format!("http://127.0.0.1:{port}/v1");
    let health_url = format!("http://127.0.0.1:{port}/health");
    let mut guard = LlamaServerGuard {
        child: Some(child),
        port,
        base_url,
        log_path: Some(log_path.clone()),
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
                let tail = read_last_non_empty_lines(&log_path, 8);
                let _ = fs::remove_file(&log_path);
                guard.log_path = None;
                return if tail.is_empty() {
                    Err(anyhow!(
                        "llama-server exited prematurely during startup with status {status}"
                    ))
                } else {
                    Err(anyhow!(
                        "llama-server exited prematurely during startup with status {status}: {tail}"
                    ))
                };
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
            Duration::from_secs(90),
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

        let ctx_size = context_window.clamp(2048, 32768);

        match spawn_and_wait_healthy(
            llama_bin,
            llm_model_path,
            ctx_size,
            false,
            cancel_flag,
            startup_timeout,
        )
        .await
        {
            Ok(guard) => Ok(guard),
            Err(gpu_err) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    return Err(gpu_err);
                }
                tracing::warn!(
                    "llama-server GPU auto-fit startup failed ({gpu_err}); retrying in CPU-only mode (-ngl 0 -np 1)"
                );
                spawn_and_wait_healthy(
                    llama_bin,
                    llm_model_path,
                    ctx_size,
                    true,
                    cancel_flag,
                    startup_timeout,
                )
                .await
                .map_err(|cpu_err| {
                    let msg = format!(
                        "llama-server GPU startup failed ({gpu_err}); CPU fallback also failed ({cpu_err})"
                    );
                    if !cancel_flag.load(Ordering::SeqCst) {
                        crate::diagnostics::record_subprocess_failure_async("llama-server", &msg);
                    }
                    anyhow!(msg)
                })
            }
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
        if let Some(log_path) = self.log_path.take() {
            let _ = fs::remove_file(log_path);
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
            log_path: None,
        };
        guard.stop();
        assert!(guard.child.is_none());

        // Process should no longer be running (or already reaped)
        let proc_stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        assert!(proc_stat.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn falls_back_to_cpu_mode_when_auto_fit_gpu_startup_fails() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt-llama-fallback-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let mock_model = temp_dir.join("dummy.gguf");
        fs::write(&mock_model, b"GGUF").unwrap();

        let args_log = temp_dir.join("invocations.log");
        let mock_bin = temp_dir.join("mock-llama-server");

        let script = format!(
            r#"#!/usr/bin/env python3
import http.server
import sys

args = sys.argv[1:]
with open({args_log:?}, "a", encoding="utf-8") as f:
    f.write(" ".join(args) + "\n")

if "-ngl" not in args:
    sys.stderr.write("ggml_vulkan: vk::Device::allocateMemory: ErrorOutOfDeviceMemory\n")
    sys.exit(1)

idx = args.index("-ngl")
if idx + 1 >= len(args) or args[idx + 1] != "0":
    sys.stderr.write("expected -ngl 0 on fallback\n")
    sys.exit(2)

port_idx = args.index("--port")
port = int(args[port_idx + 1])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b'{{"status":"ok"}}')
        else:
            self.send_response(404)
            self.end_headers()
    def log_message(self, format, *args):
        pass

http.server.HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#,
            args_log = args_log.to_string_lossy().to_string(),
        );

        fs::write(&mock_bin, script).unwrap();
        let mut perms = fs::metadata(&mock_bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_bin, perms).unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let mut guard = LlamaServerGuard::start_with_timeout(
            &mock_bin,
            &mock_model,
            4096,
            &cancel,
            Duration::from_secs(10),
        )
        .await
        .expect("CPU fallback should succeed when GPU auto-fit exits with code 1");

        let logged = fs::read_to_string(&args_log).unwrap();
        let lines: Vec<&str> = logged.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 startup attempts, got: {logged}");
        assert!(lines[0].contains("-np 1"), "first call missing -np 1: {}", lines[0]);
        assert!(!lines[0].contains("-ngl"), "first call must omit -ngl: {}", lines[0]);
        assert!(lines[1].contains("-np 1"), "fallback missing -np 1: {}", lines[1]);
        assert!(lines[1].contains("-ngl 0"), "fallback missing -ngl 0: {}", lines[1]);

        guard.stop();
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
