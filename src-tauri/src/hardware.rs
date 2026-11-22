//! GPU & VRAM detection (NVIDIA nvidia-smi probing and tier classification).

use crate::models::{GpuInfo, VramTier};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

/// Pure parser for `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits` output.
/// When multiple NVIDIA GPUs are listed, selects the GPU with the highest VRAM capacity.
pub fn parse_nvidia_smi_output(raw: &str) -> GpuInfo {
    let mut best_gpu: Option<(String, u64)> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some((name_part, mem_part)) = trimmed.rsplit_once(',') else {
            continue;
        };

        let gpu_name = name_part.trim();
        if gpu_name.is_empty() {
            continue;
        }

        let Some(first_token) = mem_part.trim().split_whitespace().next() else {
            continue;
        };

        let parsed_vram = first_token
            .parse::<u64>()
            .ok()
            .or_else(|| first_token.parse::<f64>().ok().map(|v| v.round() as u64));

        let Some(vram_mb) = parsed_vram else {
            continue;
        };

        if vram_mb == 0 {
            continue;
        }

        match &best_gpu {
            Some((_, best_vram)) if *best_vram >= vram_mb => {}
            _ => {
                best_gpu = Some((gpu_name.to_string(), vram_mb));
            }
        }
    }

    match best_gpu {
        Some((name, vram_mb)) => GpuInfo {
            has_nvidia: true,
            gpu_name: Some(name),
            vram_mb,
            recommended_tier: classify_vram_tier(vram_mb),
        },
        None => GpuInfo {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: 0,
            recommended_tier: VramTier::CpuOnly,
        },
    }
}

fn run_nvidia_smi_command(binary: &str) -> Option<String> {
    let mut cmd = Command::new(binary);
    cmd.args([
        "--query-gpu=name,memory.total",
        "--format=csv,noheader,nounits",
    ]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout).ok()
}

/// Probe the system for an NVIDIA GPU using `nvidia-smi` and classify its VRAM tier.
pub fn detect_gpu() -> GpuInfo {
    let candidates: &[&str] = &[
        "nvidia-smi",
        #[cfg(windows)]
        r"C:\Windows\System32\nvidia-smi.exe",
        #[cfg(windows)]
        r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
    ];

    for bin in candidates {
        if let Some(stdout) = run_nvidia_smi_command(bin) {
            let info = parse_nvidia_smi_output(&stdout);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_4gb_rtx_3050_laptop_gpu() {
        let raw = "NVIDIA GeForce RTX 3050 Laptop GPU, 4096\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(
            info.gpu_name.as_deref(),
            Some("NVIDIA GeForce RTX 3050 Laptop GPU")
        );
        assert_eq!(info.vram_mb, 4096);
        assert_eq!(info.recommended_tier, VramTier::Vram4Gb);
    }

    #[test]
    fn parses_8gb_rtx_4060_gpu() {
        let raw = "NVIDIA GeForce RTX 4060, 8188\r\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce RTX 4060"));
        assert_eq!(info.vram_mb, 8188);
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
        assert_eq!(info_4070.recommended_tier, VramTier::Vram12GbPlus);

        let raw_3090 = "NVIDIA GeForce RTX 3090, 24576";
        let info_3090 = parse_nvidia_smi_output(raw_3090);
        assert!(info_3090.has_nvidia);
        assert_eq!(info_3090.vram_mb, 24576);
        assert_eq!(info_3090.recommended_tier, VramTier::Vram12GbPlus);
    }

    #[test]
    fn selects_max_vram_gpu_on_multi_gpu_output() {
        let raw = "\
NVIDIA GeForce GTX 1650, 4096
NVIDIA GeForce RTX 3080, 10240
NVIDIA GeForce RTX 3090, 24576
";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce RTX 3090"));
        assert_eq!(info.vram_mb, 24576);
        assert_eq!(info.recommended_tier, VramTier::Vram12GbPlus);
    }

    #[test]
    fn handles_low_vram_nvidia_as_cpu_only_tier() {
        let raw = "NVIDIA GeForce MX150, 2048\n";
        let info = parse_nvidia_smi_output(raw);
        assert!(info.has_nvidia);
        assert_eq!(info.gpu_name.as_deref(), Some("NVIDIA GeForce MX150"));
        assert_eq!(info.vram_mb, 2048);
        assert_eq!(info.recommended_tier, VramTier::CpuOnly);
    }

    #[test]
    fn handles_empty_or_invalid_output_gracefully() {
        let empty = parse_nvidia_smi_output("");
        assert!(!empty.has_nvidia);
        assert_eq!(empty.gpu_name, None);
        assert_eq!(empty.vram_mb, 0);
        assert_eq!(empty.recommended_tier, VramTier::CpuOnly);

        let err = parse_nvidia_smi_output(
            "NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.",
        );
        assert!(!err.has_nvidia);
        assert_eq!(err.gpu_name, None);
        assert_eq!(err.vram_mb, 0);
        assert_eq!(err.recommended_tier, VramTier::CpuOnly);
    }
}
