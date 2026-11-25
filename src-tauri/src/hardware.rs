//! GPU & VRAM detection (NVIDIA nvidia-smi probing, CUDA 12.4 driver validation,
//! and VRAM tier classification).

use crate::models::{GpuInfo, VramTier};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const NVIDIA_SMI_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_STDOUT_BYTES: u64 = 256 * 1024;

/// Minimum NVIDIA driver version required for CUDA 12.4 (`550.54`).
pub const MIN_CUDA_12_4_DRIVER_MAJOR: u32 = 550;
pub const MIN_CUDA_12_4_DRIVER_MINOR: u32 = 54;

/// Classify a VRAM capacity in MiB into the recommended hardware tier.
pub fn classify_vram_tier(vram_mb: u64) -> VramTier {
    if vram_mb >= 11000 {
        VramTier::Vram12GbPlus
    } else if vram_mb >= 5500 {
        VramTier::Vram8Gb
    } else if vram_mb >= 3500 {
        VramTier::Vram4Gb
    } else {
        VramTier::CpuOnly
    }
}

/// Parse an NVIDIA driver version string such as `"550.54"`, `"551.86"`, or `"560.35.03"`
/// into `(major, minor)`.
pub fn parse_driver_version(raw: &str) -> Option<(u32, u32)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split('.');
    let major = parts.next()?.trim().parse::<u32>().ok()?;
    let minor = parts.next()?.trim().parse::<u32>().ok()?;
    Some((major, minor))
}

/// Return `true` when `driver_version` meets or exceeds the CUDA 12.4 minimum (`550.54`).
pub fn is_cuda_12_4_driver_supported(driver_version: &str) -> bool {
    matches!(
        parse_driver_version(driver_version),
        Some((major, minor))
            if major > MIN_CUDA_12_4_DRIVER_MAJOR
                || (major == MIN_CUDA_12_4_DRIVER_MAJOR && minor >= MIN_CUDA_12_4_DRIVER_MINOR)
    )
}

fn parse_vram_mb(raw: &str) -> Option<u64> {
    let first_token = raw.split_whitespace().next()?;
    let parsed = first_token
        .parse::<u64>()
        .ok()
        .or_else(|| first_token.parse::<f64>().ok().map(|v| v.round() as u64))?;
    if parsed == 0 {
        None
    } else {
        Some(parsed)
    }
}

fn normalize_driver_field(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "n/a" || lower == "[n/a]" || lower.contains("not supported") {
        return None;
    }
    Some(trimmed.to_string())
}

/// Build strict Windows absolute candidate paths for `nvidia-smi.exe` using `%SystemRoot%` and
/// `%ProgramFiles%` (never a bare relative `"nvidia-smi"` susceptible to CWD/PATH hijacking).
pub fn build_windows_nvidia_smi_candidates(
    system_root: Option<&str>,
    program_files: Option<&str>,
) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(2);

    let sys_root = system_root
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(r"C:\Windows");
    let sys_root_clean = sys_root.trim_end_matches(['\\', '/']);
    out.push(PathBuf::from(format!(
        r"{sys_root_clean}\System32\nvidia-smi.exe"
    )));

    let prog_files = program_files
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(r"C:\Program Files");
    let prog_files_clean = prog_files.trim_end_matches(['\\', '/']);
    out.push(PathBuf::from(format!(
        r"{prog_files_clean}\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
    )));

    out
}

fn resolve_nvidia_smi_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let sys_root = std::env::var("SystemRoot")
            .ok()
            .or_else(|| std::env::var("WINDIR").ok());
        let prog_files = std::env::var("ProgramFiles").ok();
        build_windows_nvidia_smi_candidates(sys_root.as_deref(), prog_files.as_deref())
    }

    #[cfg(not(target_os = "windows"))]
    {
        vec![
            PathBuf::from("/usr/bin/nvidia-smi"),
            PathBuf::from("/usr/local/bin/nvidia-smi"),
            PathBuf::from("nvidia-smi"),
        ]
    }
}

fn windows_nvcuda_dll_exists() -> bool {
    #[cfg(target_os = "windows")]
    {
        let sys_root = std::env::var("SystemRoot")
            .ok()
            .or_else(|| std::env::var("WINDIR").ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| r"C:\Windows".to_string());
        let sys_root_clean = sys_root.trim_end_matches(['\\', '/']);
        PathBuf::from(format!(r"{sys_root_clean}\System32\nvcuda.dll")).is_file()
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

/// Pure parser for `nvidia-smi --query-gpu=name,memory.total[,driver_version] --format=csv,noheader,nounits`
/// with an explicit `nvcuda_present` flag (allowing deterministic testing of the Windows `nvcuda.dll` check).
pub fn parse_nvidia_smi_output_with_nvcuda(raw: &str, nvcuda_present: bool) -> GpuInfo {
    let mut best_gpu: Option<(String, u64, Option<String>)> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split(',').collect();
        if parts.len() < 2 {
            continue;
        }

        // Try 3-column format (`name, memory.total, driver_version`) first when >= 3 comma-separated
        // fields exist and the second-to-last column parses cleanly as VRAM MiB.
        let parsed_row: Option<(String, u64, Option<String>)> = if parts.len() >= 3 {
            if let Some(vram_mb) = parse_vram_mb(parts[parts.len() - 2]) {
                let gpu_name = parts[..parts.len() - 2].join(",").trim().to_string();
                let driver = normalize_driver_field(parts[parts.len() - 1]);
                if gpu_name.is_empty() {
                    None
                } else {
                    Some((gpu_name, vram_mb, driver))
                }
            } else if let Some(vram_mb) = parse_vram_mb(parts[parts.len() - 1]) {
                // Legacy 2-column fallback where GPU name itself contained a comma
                let gpu_name = parts[..parts.len() - 1].join(",").trim().to_string();
                if gpu_name.is_empty() {
                    None
                } else {
                    Some((gpu_name, vram_mb, None))
                }
            } else {
                None
            }
        } else if let Some(vram_mb) = parse_vram_mb(parts[1]) {
            let gpu_name = parts[0].trim().to_string();
            if gpu_name.is_empty() {
                None
            } else {
                Some((gpu_name, vram_mb, None))
            }
        } else {
            None
        };

        let Some((gpu_name, vram_mb, driver_version)) = parsed_row else {
            continue;
        };

        match &best_gpu {
            Some((_, best_vram, _)) if *best_vram >= vram_mb => {}
            _ => {
                best_gpu = Some((gpu_name, vram_mb, driver_version));
            }
        }
    }

    let Some((name, vram_mb, driver_version)) = best_gpu else {
        return GpuInfo {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: 0,
            recommended_tier: VramTier::CpuOnly,
            driver_version: None,
            cuda_supported: false,
            cuda_status_note: None,
        };
    };

    if vram_mb < 3500 {
        return GpuInfo {
            has_nvidia: true,
            gpu_name: Some(name),
            vram_mb,
            recommended_tier: VramTier::CpuOnly,
            driver_version,
            cuda_supported: false,
            cuda_status_note: Some(format!(
                "NVIDIA GPU has {} MB VRAM (< 3500 MB required for CUDA 12.4); using CPU mode.",
                vram_mb
            )),
        };
    }

    if let Some(ref drv) = driver_version {
        if !is_cuda_12_4_driver_supported(drv) {
            return GpuInfo {
                has_nvidia: true,
                gpu_name: Some(name),
                vram_mb,
                recommended_tier: VramTier::CpuOnly,
                driver_version: Some(drv.clone()),
                cuda_supported: false,
                cuda_status_note: Some(format!(
                    "NVIDIA driver {} is older than 550.54 required for CUDA 12.4; using CPU mode.",
                    drv
                )),
            };
        }
    }

    if !nvcuda_present {
        return GpuInfo {
            has_nvidia: true,
            gpu_name: Some(name),
            vram_mb,
            recommended_tier: VramTier::CpuOnly,
            driver_version,
            cuda_supported: false,
            cuda_status_note: Some(
                "NVIDIA CUDA driver library (C:\\Windows\\System32\\nvcuda.dll) was not found; using CPU mode."
                    .to_string(),
            ),
        };
    }

    GpuInfo {
        has_nvidia: true,
        gpu_name: Some(name),
        vram_mb,
        recommended_tier: classify_vram_tier(vram_mb),
        driver_version,
        cuda_supported: true,
        cuda_status_note: None,
    }
}

/// Pure parser for `nvidia-smi --query-gpu=name,memory.total[,driver_version] --format=csv,noheader,nounits` output.
/// When multiple NVIDIA GPUs are listed, selects the GPU with the highest VRAM capacity.
pub fn parse_nvidia_smi_output(raw: &str) -> GpuInfo {
    parse_nvidia_smi_output_with_nvcuda(raw, true)
}

fn run_command_stdout_with_timeout(
    binary: &Path,
    args: &[&str],
    timeout: Duration,
) -> Option<String> {
    let mut cmd = Command::new(binary);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().ok()?;
    let stdout_pipe = child.stdout.take();
    let reader_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe {
            let _ = pipe.take(MAX_PROBE_STDOUT_BYTES).read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader_handle.join();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader_handle.join();
                return None;
            }
        }
    };

    let stdout_bytes = reader_handle.join().ok()?;
    if !status.success() {
        return None;
    }

    String::from_utf8(stdout_bytes).ok()
}

fn run_nvidia_smi_query(binary: &Path, query: &str) -> Option<String> {
    run_command_stdout_with_timeout(
        binary,
        &[query, "--format=csv,noheader,nounits"],
        NVIDIA_SMI_PROBE_TIMEOUT,
    )
}

fn run_nvidia_smi_command(binary: &Path) -> Option<String> {
    if binary.is_absolute() && !binary.is_file() {
        return None;
    }
    run_nvidia_smi_query(binary, "--query-gpu=name,memory.total,driver_version")
        .filter(|out| !out.trim().is_empty())
        .or_else(|| run_nvidia_smi_query(binary, "--query-gpu=name,memory.total"))
}

/// Probe the system for an NVIDIA GPU using `nvidia-smi` (with strict Windows system paths
/// and a 5s execution timeout), validate CUDA 12.4 driver support, and classify its VRAM tier.
pub fn detect_gpu() -> GpuInfo {
    let candidates = resolve_nvidia_smi_candidates();
    let nvcuda_present = windows_nvcuda_dll_exists();

    for bin in &candidates {
        if let Some(stdout) = run_nvidia_smi_command(bin) {
            let info = parse_nvidia_smi_output_with_nvcuda(&stdout, nvcuda_present);
            if info.has_nvidia {
                return info;
            }
        }
    }

    GpuInfo {
        has_nvidia: false,
        gpu_name: None,
        vram_mb: 0,
        recommended_tier: VramTier::CpuOnly,
        driver_version: None,
        cuda_supported: false,
        cuda_status_note: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_nvidia_smi_candidates_use_strict_absolute_system_paths_only() {
        let cands = build_windows_nvidia_smi_candidates(
            Some(r"D:\WinSys"),
            Some(r"E:\Program Files\"),
        );
        assert_eq!(cands.len(), 2);
        assert_eq!(
            cands[0].to_string_lossy(),
            r"D:\WinSys\System32\nvidia-smi.exe"
        );
        assert_eq!(
            cands[1].to_string_lossy(),
            r"E:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
        );
        assert!(
            !cands
                .iter()
                .any(|p| p.to_string_lossy().eq_ignore_ascii_case("nvidia-smi")
                    || p.to_string_lossy().eq_ignore_ascii_case("nvidia-smi.exe")),
            "Must never include bare relative nvidia-smi on Windows"
        );

        let defaults = build_windows_nvidia_smi_candidates(None, Some("   "));
        assert_eq!(
            defaults[0].to_string_lossy(),
            r"C:\Windows\System32\nvidia-smi.exe"
        );
        assert_eq!(
            defaults[1].to_string_lossy(),
            r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_probe_timeout_kills_hanging_child_promptly() {
        let t0 = Instant::now();
        let res = run_command_stdout_with_timeout(
            Path::new("/bin/sleep"),
            &["10"],
            Duration::from_millis(120),
        );
        assert!(res.is_none(), "Hanging probe must time out and return None");
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "Timeout took too long: {:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn parses_driver_versions_and_validates_cuda_12_4_threshold() {
        assert_eq!(parse_driver_version("550.54"), Some((550, 54)));
        assert_eq!(parse_driver_version("560.35.03"), Some((560, 35)));
        assert_eq!(parse_driver_version("535.98"), Some((535, 98)));
        assert_eq!(parse_driver_version("invalid"), None);
        assert_eq!(parse_driver_version(""), None);

        assert!(is_cuda_12_4_driver_supported("550.54"));
        assert!(is_cuda_12_4_driver_supported("550.90.07"));
        assert!(is_cuda_12_4_driver_supported("551.86"));
        assert!(is_cuda_12_4_driver_supported("560.35.03"));

        assert!(!is_cuda_12_4_driver_supported("550.53"));
        assert!(!is_cuda_12_4_driver_supported("535.98"));
        assert!(!is_cuda_12_4_driver_supported("512.15"));
        assert!(!is_cuda_12_4_driver_supported("not-a-version"));
    }

    #[test]
    fn accepts_cuda_12_4_supported_driver_550_54_plus() {
        let raw = "NVIDIA GeForce RTX 4060, 8188, 550.54\r\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce RTX 4060"));
        assert_eq!(info.vram_mb, 8188);
        assert_eq!(info.driver_version.as_deref(), Some("550.54"));
        assert!(info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::Vram8Gb);
        assert_eq!(info.cuda_status_note, None);

        let raw_newer = "NVIDIA GeForce RTX 4090, 24564, 560.35.03\n";
        let info_newer = parse_nvidia_smi_output(raw_newer);
        assert!(info_newer.cuda_supported);
        assert_eq!(info_newer.driver_version.as_deref(), Some("560.35.03"));
        assert_eq!(info_newer.recommended_tier, VramTier::Vram12GbPlus);
        assert_eq!(info_newer.cuda_status_note, None);
    }

    #[test]
    fn downgrades_old_driver_535_98_to_cpu_only_with_note() {
        let raw = "NVIDIA GeForce RTX 3060, 12288, 535.98\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce RTX 3060"));
        assert_eq!(info.vram_mb, 12288);
        assert_eq!(info.driver_version.as_deref(), Some("535.98"));
        assert!(!info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::CpuOnly);
        let note = info
            .cuda_status_note
            .as_deref()
            .expect("Expected cuda_status_note for old driver");
        assert!(note.contains("535.98"));
        assert!(note.contains("550.54"));
    }

    #[test]
    fn downgrades_when_windows_nvcuda_dll_is_missing() {
        let raw = "NVIDIA GeForce RTX 4070, 12282, 555.85\n";
        let info = parse_nvidia_smi_output_with_nvcuda(raw, false);
        assert!(info.has_nvidia);
        assert!(!info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::CpuOnly);
        assert!(info
            .cuda_status_note
            .as_deref()
            .unwrap_or("")
            .contains("nvcuda.dll"));
    }

    #[test]
    fn parses_4gb_rtx_3050_laptop_gpu_legacy_two_column() {
        let raw = "NVIDIA GeForce RTX 3050 Laptop GPU, 4096\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(
            info.gpu_name.as_deref(),
            Some("NVIDIA GeForce RTX 3050 Laptop GPU")
        );
        assert_eq!(info.vram_mb, 4096);
        assert_eq!(info.driver_version, None);
        assert!(info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::Vram4Gb);
        assert_eq!(info.cuda_status_note, None);
    }

    #[test]
    fn parses_8gb_rtx_4060_gpu() {
        let raw = "NVIDIA GeForce RTX 4060, 8188\r\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce RTX 4060"));
        assert_eq!(info.vram_mb, 8188);
        assert!(info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::Vram8Gb);
    }

    #[test]
    fn parses_12gb_plus_rtx_4070_ti_and_3090() {
        let raw_4070 = "NVIDIA GeForce RTX 4070 Ti, 12282";
        let info_4070 = parse_nvidia_smi_output(raw_4070);
        assert!(info_4070.has_nvidia);
        assert_eq!(
            info_4070.gpu_name.as_deref(),
            Some("NVIDIA GeForce RTX 4070 Ti")
        );
        assert_eq!(info_4070.vram_mb, 12282);
        assert!(info_4070.cuda_supported);
        assert_eq!(info_4070.recommended_tier, VramTier::Vram12GbPlus);

        let raw_3090 = "NVIDIA GeForce RTX 3090, 24576";
        let info_3090 = parse_nvidia_smi_output(raw_3090);
        assert!(info_3090.has_nvidia);
        assert_eq!(info_3090.vram_mb, 24576);
        assert!(info_3090.cuda_supported);
        assert_eq!(info_3090.recommended_tier, VramTier::Vram12GbPlus);
    }

    #[test]
    fn selects_max_vram_gpu_on_multi_gpu_output() {
        let raw = "\
NVIDIA GeForce GTX 1650, 4096, 551.86
NVIDIA GeForce RTX 3080, 10240, 551.86
NVIDIA GeForce RTX 3090, 24576, 551.86
";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce RTX 3090"));
        assert_eq!(info.vram_mb, 24576);
        assert_eq!(info.driver_version.as_deref(), Some("551.86"));
        assert!(info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::Vram12GbPlus);
    }

    #[test]
    fn handles_low_vram_nvidia_as_cpu_only_tier() {
        let raw = "NVIDIA GeForce MX150, 2048, 551.86\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce MX150"));
        assert_eq!(info.vram_mb, 2048);
        assert!(!info.cuda_supported);
        assert_eq!(info.recommended_tier, VramTier::CpuOnly);
        assert!(info.cuda_status_note.is_some());
    }

    #[test]
    fn handles_empty_or_invalid_output_gracefully() {
        let empty = parse_nvidia_smi_output("");
        assert!(!empty.has_nvidia);
        assert_eq!(empty.gpu_name, None);
        assert_eq!(empty.vram_mb, 0);
        assert!(!empty.cuda_supported);
        assert_eq!(empty.recommended_tier, VramTier::CpuOnly);

        let err = parse_nvidia_smi_output(
            "NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.",
        );
        assert!(!err.has_nvidia);
        assert_eq!(err.gpu_name, None);
        assert_eq!(err.vram_mb, 0);
        assert!(!err.cuda_supported);
        assert_eq!(err.recommended_tier, VramTier::CpuOnly);
    }
}
