//! Lifecycle manager for local `llama-server` (`llama.cpp` OpenAI-compatible server).
//!
//! Guarantees that `llama-server` is only active during the translation/polishing stage
//! (never overlapping with Whisper ASR), authenticates requests with an ephemeral per-spawn
//! `--api-key` token, logs to a private `0600` temp file outside `/tmp`, retries port bind
//! collisions, and terminates automatically on `stop()`, `Drop`, or parent process crash.

use anyhow::{anyhow, Context, Result};
use std::fs::{self, File, OpenOptions};
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

const MAX_PORT_BIND_ATTEMPTS: usize = 3;
const LLAMA_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(target_os = "linux")]
mod linux_pdeathsig {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGKILL: i32 = 9;
    const EPIPE: i32 = 32;

    extern "C" {
        fn getpid() -> i32;
        fn getppid() -> i32;
        fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    }

    pub fn configure_kill_on_parent_death(cmd: &mut Command) {
        // Capture parent PID before fork() to detect parent exit between fork() and prctl().
        let parent_pid = unsafe { getpid() };
        // SAFETY: `prctl`, `getppid`, and `io::Error::from_raw_os_error` are async-signal-safe
        // and perform zero heap allocations inside the post-fork `pre_exec` closure.
        unsafe {
            cmd.pre_exec(move || {
                if prctl(PR_SET_PDEATHSIG, SIGKILL as usize, 0, 0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if getppid() != parent_pid {
                    return Err(io::Error::from_raw_os_error(EPIPE));
                }
                Ok(())
            });
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr;

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: u32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(lp_job_attributes: *mut c_void, lp_name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            h_job: *mut c_void,
            job_object_information_class: u32,
            lp_job_object_information: *const c_void,
            cb_job_object_information_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(h_job: *mut c_void, h_process: *mut c_void) -> i32;
        fn CloseHandle(h_object: *mut c_void) -> i32;
    }

    pub struct JobObjectGuard(*mut c_void);

    // SAFETY: Windows kernel HANDLEs may be safely transferred between threads.
    unsafe impl Send for JobObjectGuard {}
    unsafe impl Sync for JobObjectGuard {}

    impl JobObjectGuard {
        pub fn attach_child(child: &Child) -> Option<Self> {
            unsafe {
                let job = CreateJobObjectW(ptr::null_mut(), ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info = JobObjectExtendedLimitInformation::default();
                info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&info as *const JobObjectExtendedLimitInformation).cast::<c_void>(),
                    std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
                );
                if ok == 0 {
                    CloseHandle(job);
                    return None;
                }
                let assigned = AssignProcessToJobObject(job, child.as_raw_handle());
                if assigned == 0 {
                    CloseHandle(job);
                    return None;
                }
                Some(Self(job))
            }
        }
    }

    impl Drop for JobObjectGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
                self.0 = ptr::null_mut();
            }
        }
    }
}

pub struct LlamaServerGuard {
    child: Option<Child>,
    port: u16,
    base_url: String,
    api_key: String,
    log_path: Option<PathBuf>,
    #[cfg(target_os = "windows")]
    job_object: Option<windows_job::JobObjectGuard>,
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

/// Resolves the private log directory for `llama-server` stderr output (never world-shared `/tmp`).
pub fn resolve_private_log_dir(log_dir: Option<&Path>) -> PathBuf {
    match log_dir {
        Some(dir) => dir.to_path_buf(),
        None => crate::storage::default_installed_root_dir()
            .join("data")
            .join("temp"),
    }
}

fn create_private_log_file(log_dir: Option<&Path>) -> Result<(PathBuf, File)> {
    let dir = resolve_private_log_dir(log_dir);
    fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Failed to create private llama-server log directory: {}",
            dir.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }

    let log_path = dir.join(format!("llama-server-{}.log", uuid::Uuid::new_v4()));
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let file = opts.open(&log_path).with_context(|| {
        format!(
            "Failed to create private llama-server log file: {}",
            log_path.display()
        )
    })?;
    Ok((log_path, file))
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

fn is_port_bind_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("address already in use")
        || lower.contains("eaddrinuse")
        || lower.contains("failed to bind")
        || lower.contains("bind failed")
        || lower.contains("failed to listen")
        || lower.contains("only one usage of each socket address")
        || lower.contains("cannot assign requested address")
}

/// Probes `llama-server --version` with a strict `5s` timeout (killing child on timeout).
pub async fn probe_llama_server_version(llama_bin: &Path) -> Result<String> {
    if !llama_bin.is_file() {
        return Err(anyhow!(
            "llama-server binary not found at {}",
            llama_bin.display()
        ));
    }

    let mut cmd = tokio::process::Command::new(llama_bin);
    crate::pipeline::ffmpeg::configure_command_env(&mut cmd, llama_bin);
    cmd.kill_on_drop(true)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = tokio::time::timeout(LLAMA_VERSION_PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out probing llama-server --version at {} after {}s",
                llama_bin.display(),
                LLAMA_VERSION_PROBE_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| {
            format!(
                "Failed to execute llama-server --version at {}",
                llama_bin.display()
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(anyhow!(
            "llama-server --version failed (status {}): {}",
            output.status,
            detail
        ));
    }

    let summary = if !stdout.is_empty() { stdout } else { stderr };
    Ok(summary
        .lines()
        .next()
        .unwrap_or("llama-server")
        .trim()
        .to_string())
}

#[allow(clippy::too_many_arguments)]
async fn spawn_and_wait_healthy_once(
    llama_bin: &Path,
    llm_model_path: &Path,
    ctx_size: u32,
    force_cpu: bool,
    cancel_flag: &Arc<AtomicBool>,
    startup_timeout: Duration,
    log_dir: Option<&Path>,
) -> Result<LlamaServerGuard> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    let port = allocate_free_port()?;
    let api_key = uuid::Uuid::new_v4().to_string();
    let (log_path, log_file) = create_private_log_file(log_dir)?;
    let stderr_stdio = Stdio::from(log_file);

    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(4, 16))
        .unwrap_or(4);
    let threads_str = threads.to_string();

    let mut cmd = Command::new(llama_bin);
    configure_std_command_env(&mut cmd, llama_bin);
    #[cfg(target_os = "linux")]
    linux_pdeathsig::configure_kill_on_parent_death(&mut cmd);

    cmd.arg("-m")
        .arg(llm_model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--api-key")
        .arg(&api_key)
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

    let spawn_res = cmd.spawn();
    // Drop `cmd` (and its stderr file handle) before removing `log_path` on error so Windows does
    // not reject deletion with ERROR_SHARING_VIOLATION.
    drop(cmd);
    let child = match spawn_res {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_file(&log_path);
            return Err(err).with_context(|| {
                format!(
                    "Failed to spawn llama-server at {}",
                    llama_bin.to_string_lossy()
                )
            });
        }
    };

    #[cfg(target_os = "windows")]
    let job_object = windows_job::JobObjectGuard::attach_child(&child);

    let base_url = format!("http://127.0.0.1:{port}/v1");
    let health_url = format!("http://127.0.0.1:{port}/health");
    let mut guard = LlamaServerGuard {
        child: Some(child),
        port,
        base_url,
        api_key: api_key.clone(),
        log_path: Some(log_path.clone()),
        #[cfg(target_os = "windows")]
        job_object,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("Failed to build HTTP client for llama-server health check")?;

    let auth_header = format!("Bearer {api_key}");
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

        if let Ok(resp) = client
            .get(&health_url)
            .header(reqwest::header::AUTHORIZATION, &auth_header)
            .send()
            .await
        {
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

#[allow(clippy::too_many_arguments)]
async fn spawn_and_wait_healthy(
    llama_bin: &Path,
    llm_model_path: &Path,
    ctx_size: u32,
    force_cpu: bool,
    cancel_flag: &Arc<AtomicBool>,
    startup_timeout: Duration,
    log_dir: Option<&Path>,
) -> Result<LlamaServerGuard> {
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..MAX_PORT_BIND_ATTEMPTS {
        match spawn_and_wait_healthy_once(
            llama_bin,
            llm_model_path,
            ctx_size,
            force_cpu,
            cancel_flag,
            startup_timeout,
            log_dir,
        )
        .await
        {
            Ok(guard) => return Ok(guard),
            Err(err) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    return Err(err);
                }
                let err_text = err.to_string();
                if attempt + 1 < MAX_PORT_BIND_ATTEMPTS && is_port_bind_failure(&err_text) {
                    tracing::warn!(
                        "llama-server port bind collision on attempt {}/{} ({err_text}); retrying on a new ephemeral port",
                        attempt + 1,
                        MAX_PORT_BIND_ATTEMPTS
                    );
                    last_err = Some(err);
                    continue;
                }
                return Err(err);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("Failed to start llama-server after port retries")))
}

impl LlamaServerGuard {
    /// Spawns `llama-server` on an ephemeral localhost port and polls `/health` until HTTP 200.
    pub async fn start(
        llama_bin: &Path,
        llm_model_path: &Path,
        context_window: u32,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::start_with_timeout_and_log_dir(
            llama_bin,
            llm_model_path,
            context_window,
            cancel_flag,
            Duration::from_secs(90),
            None,
        )
        .await
    }

    /// Spawns `llama-server` with an explicit private `log_dir` and default 90s startup timeout.
    pub async fn start_in_log_dir(
        llama_bin: &Path,
        llm_model_path: &Path,
        context_window: u32,
        cancel_flag: &Arc<AtomicBool>,
        log_dir: &Path,
    ) -> Result<Self> {
        Self::start_with_timeout_and_log_dir(
            llama_bin,
            llm_model_path,
            context_window,
            cancel_flag,
            Duration::from_secs(90),
            Some(log_dir),
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
        Self::start_with_timeout_and_log_dir(
            llama_bin,
            llm_model_path,
            context_window,
            cancel_flag,
            startup_timeout,
            None,
        )
        .await
    }

    pub async fn start_with_timeout_and_log_dir(
        llama_bin: &Path,
        llm_model_path: &Path,
        context_window: u32,
        cancel_flag: &Arc<AtomicBool>,
        startup_timeout: Duration,
        log_dir: Option<&Path>,
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
            log_dir,
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
                    log_dir,
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

    /// Returns the ephemeral per-spawn `--api-key` bearer token for authenticating local LLM calls.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the active private log path if present.
    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    /// Explicitly kills and reaps the `llama-server` child process to free VRAM immediately.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(target_os = "windows")]
        {
            self.job_object = None;
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

    #[test]
    fn default_private_log_dir_lives_outside_system_tmp() {
        let dir = resolve_private_log_dir(None);
        assert!(dir.ends_with(Path::new("data").join("temp")));
        assert_ne!(dir, std::env::temp_dir());
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
            api_key: "test-token".to_string(),
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
        let log_dir = temp_dir.join("private_logs");
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
expected_key = args[args.index("--api-key") + 1]

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            if self.headers.get("Authorization") != f"Bearer {{expected_key}}":
                self.send_response(401)
                self.end_headers()
                return
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
        let mut guard = LlamaServerGuard::start_with_timeout_and_log_dir(
            &mock_bin,
            &mock_model,
            4096,
            &cancel,
            Duration::from_secs(10),
            Some(&log_dir),
        )
        .await
        .expect("CPU fallback should succeed when GPU auto-fit exits with code 1");

        // Verify ephemeral API key & private 0600 log file inside log_dir
        assert!(!guard.api_key().is_empty());
        let active_log = guard.log_path().expect("expected active log path").to_path_buf();
        assert_eq!(active_log.parent(), Some(log_dir.as_path()));
        let mode = fs::metadata(&active_log).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "Expected 0600 mode on llama-server log, got {:o}", mode);

        let logged = fs::read_to_string(&args_log).unwrap();
        let lines: Vec<&str> = logged.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 startup attempts, got: {logged}");
        assert!(lines[0].contains("-np 1"), "first call missing -np 1: {}", lines[0]);
        assert!(!lines[0].contains("-ngl"), "first call must omit -ngl: {}", lines[0]);
        assert!(lines[0].contains("--api-key"), "first call missing --api-key: {}", lines[0]);
        assert!(lines[1].contains("-np 1"), "fallback missing -np 1: {}", lines[1]);
        assert!(lines[1].contains("-ngl 0"), "fallback missing -ngl 0: {}", lines[1]);
        assert!(
            lines[1].contains(&format!("--api-key {}", guard.api_key())),
            "fallback --api-key did not match guard.api_key()"
        );

        guard.stop();
        assert!(!active_log.exists(), "log file should be removed on stop()");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retries_port_bind_collision_up_to_three_times_and_probes_version_with_timeout() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt-llama-retry-test-{}",
            uuid::Uuid::new_v4()
        ));
        let log_dir = temp_dir.join("logs");
        fs::create_dir_all(&temp_dir).unwrap();

        let mock_model = temp_dir.join("dummy.gguf");
        fs::write(&mock_model, b"GGUF").unwrap();

        let counter_file = temp_dir.join("attempt_count");
        let mock_bin = temp_dir.join("mock-llama-retry");

        let script = format!(
            r#"#!/usr/bin/env python3
import http.server
import os
import sys

args = sys.argv[1:]
if "--version" in args:
    print("version: b12633-test")
    sys.exit(0)

count_path = {counter_file:?}
n = int(open(count_path).read().strip()) if os.path.exists(count_path) else 0
n += 1
with open(count_path, "w") as f:
    f.write(str(n))

if n < 3:
    sys.stderr.write("error: Address already in use (EADDRINUSE)\n")
    sys.exit(1)

port = int(args[args.index("--port") + 1])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{{"status":"ok"}}')
    def log_message(self, format, *args):
        pass

http.server.HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#,
            counter_file = counter_file.to_string_lossy().to_string(),
        );

        fs::write(&mock_bin, script).unwrap();
        fs::set_permissions(&mock_bin, fs::Permissions::from_mode(0o755)).unwrap();

        let ver = probe_llama_server_version(&mock_bin).await.unwrap();
        assert_eq!(ver, "version: b12633-test");

        let cancel = Arc::new(AtomicBool::new(false));
        let mut guard = LlamaServerGuard::start_with_timeout_and_log_dir(
            &mock_bin,
            &mock_model,
            2048,
            &cancel,
            Duration::from_secs(10),
            Some(&log_dir),
        )
        .await
        .expect("Should succeed on 3rd port allocation retry");

        let attempts: usize = fs::read_to_string(&counter_file).unwrap().trim().parse().unwrap();
        assert_eq!(attempts, 3);

        guard.stop();
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
