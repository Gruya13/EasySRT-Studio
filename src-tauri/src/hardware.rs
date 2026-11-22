//! GPU & VRAM detection (NVIDIA nvidia-smi probing and tier classification).
//! Stub module to be implemented in Wave 2.

use crate::models::{GpuInfo, VramTier};

pub fn detect_gpu() -> GpuInfo {
    GpuInfo {
        has_nvidia: false,
        gpu_name: None,
        vram_mb: 0,
        recommended_tier: VramTier::CpuOnly,
    }
}
