//! Catalog loading, component installation, checksum verification, extraction,
//! binary/model path resolution, and custom model import/deletion.

use crate::hardware;
use crate::models::{
    AppConfig, ArchiveType, CatalogItem, CatalogItemKind, CatalogManifest, CustomModelEntry,
    DownloadProgressEvent, GpuInfo, InstalledComponentStatus, SetupStatus, StoragePaths, VramTier,
    EVENT_DOWNLOAD_PROGRESS,
};
use crate::storage;
use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const EMBEDDED_CATALOG_JSON: &str = include_str!("../resources/catalog.json");
const CUDA_RUNTIME_DLLS: &[&str] = &["cublas64_12.dll", "cublasLt64_12.dll", "cudart64_12.dll"];

const ALLOWED_CATALOG_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "huggingface.co",
    "cdn-lfs.huggingface.co",
    "cas-bridge.xethub.hf.co",
];

const MAX_ARCHIVE_FILES: usize = 2_000;
const MAX_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB
const DOWNLOAD_CHUNK_STALL_TIMEOUT: Duration = Duration::from_secs(45);
const ABSOLUTE_MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
const DOWNLOAD_SIZE_MARGIN_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB
const CUDA_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Validate that a catalog item's `sha256` field is present and is a valid 64-character hex digest.
pub fn validate_sha256_hex(sha256: Option<&str>) -> Result<String> {
    let raw = sha256
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Catalog item is missing required SHA-256 checksum"))?;
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "Invalid SHA-256 checksum '{}': expected 64 hexadecimal characters",
            raw
        ));
    }
    Ok(raw.to_ascii_lowercase())
}

fn is_allowed_redirect_host(host: &str) -> bool {
    ALLOWED_CATALOG_HOSTS.contains(&host)
        || host.ends_with(".githubusercontent.com")
        || host.ends_with(".huggingface.co")
        || host.ends_with(".hf.co")
}

/// Validate that a catalog download URL uses `https://`, contains no embedded credentials,
/// and targets an explicitly allowlisted download host.
pub fn validate_catalog_download_url(raw_url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(raw_url.trim())
        .with_context(|| format!("Invalid catalog download URL: '{}'", raw_url))?;
    if parsed.scheme() != "https" {
        return Err(anyhow!(
            "Catalog download URL must use https:// (got '{}')",
            parsed.scheme()
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(anyhow!(
            "Catalog download URL must not contain embedded credentials"
        ));
    }
    let host = parsed
        .host_str()
        .map(|h| h.to_ascii_lowercase())
        .ok_or_else(|| anyhow!("Catalog download URL is missing a hostname"))?;
    if !ALLOWED_CATALOG_HOSTS.contains(&host.as_str()) {
        return Err(anyhow!(
            "Catalog download host '{}' is not in the allowed hosts list",
            host
        ));
    }
    Ok(parsed)
}

/// Validate both the download URL allowlist and mandatory 64-hex SHA-256 digest of a catalog item.
pub fn validate_catalog_item_for_download(item: &CatalogItem) -> Result<(reqwest::Url, String)> {
    let url = validate_catalog_download_url(&item.url)
        .with_context(|| format!("Rejected catalog item '{}'", item.id))?;
    let sha256 = validate_sha256_hex(item.sha256.as_deref())
        .with_context(|| format!("Rejected catalog item '{}'", item.id))?;
    Ok((url, sha256))
}

/// Return the platform identifier matching `CatalogItem.platform`.
pub fn current_platform_tag() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows-x64"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "linux-x64"
    }
}

/// Parse the compile-time embedded `resources/catalog.json`.
pub fn load_embedded_catalog() -> Result<CatalogManifest> {
    let manifest: CatalogManifest = serde_json::from_str(EMBEDDED_CATALOG_JSON)
        .context("Failed to parse embedded resources/catalog.json")?;
    Ok(manifest)
}

/// Load catalog from compile-time embedded `catalog.json` in release builds, or check explicit
/// directories / executable siblings first in debug builds before falling back to embedded.
pub fn load_catalog_from_dirs(resource_dir: Option<&Path>) -> Result<CatalogManifest> {
    #[cfg(not(debug_assertions))]
    {
        let _ = resource_dir;
        load_embedded_catalog()
    }

    #[cfg(debug_assertions)]
    {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(res_dir) = resource_dir {
            candidates.push(res_dir.join("resources").join("catalog.json"));
            candidates.push(res_dir.join("catalog.json"));
        }
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                candidates.push(exe_dir.join("resources").join("catalog.json"));
                candidates.push(exe_dir.join("catalog.json"));
            }
        }

        for candidate in candidates {
            if candidate.is_file() {
                if let Ok(raw) = fs::read_to_string(&candidate) {
                    if let Ok(manifest) = serde_json::from_str::<CatalogManifest>(&raw) {
                        if !manifest.items.is_empty() {
                            return Ok(manifest);
                        }
                    }
                }
            }
        }

        load_embedded_catalog()
    }
}

/// Load the component catalog, checking `<resource_dir>/resources/catalog.json` first if overridden
/// on disk, and falling back to the compile-time embedded `catalog.json`.
pub fn load_catalog(app: &AppHandle) -> Result<CatalogManifest> {
    let res_dir = app.path().resource_dir().ok();
    load_catalog_from_dirs(res_dir.as_deref())
}

/// Find the directory containing bundled MSVC runtime `.dll` files (`resources/vcruntime`).
pub fn find_bundled_vcruntime_dir(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(res_dir) = resource_dir {
        candidates.push(res_dir.join("resources").join("vcruntime"));
        candidates.push(res_dir.join("vcruntime"));
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join("resources").join("vcruntime"));
            candidates.push(exe_dir.join("vcruntime"));
        }
    }

    for dir in candidates {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            let has_dll = entries.flatten().any(|entry| {
                let p = entry.path();
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("dll"))
                        .unwrap_or(false)
            });
            if has_dll {
                return Some(dir);
            }
        }
    }
    None
}

/// Copy any MSVC runtime `.dll` files from `src_dir` into `target_dir` when missing or mismatched.
pub fn copy_vcruntime_dlls_from_dir(src_dir: &Path, target_dir: &Path) -> Result<usize> {
    if !src_dir.is_dir() {
        return Ok(0);
    }
    fs::create_dir_all(target_dir).with_context(|| {
        format!(
            "Failed to create vcruntime destination directory: {}",
            target_dir.display()
        )
    })?;

    let mut copied = 0usize;
    for entry in fs::read_dir(src_dir).with_context(|| {
        format!(
            "Failed to read bundled vcruntime directory: {}",
            src_dir.display()
        )
    })? {
        let entry = entry?;
        let src_path = entry.path();
        let is_dll = src_path.is_file()
            && src_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("dll"))
                .unwrap_or(false);
        if !is_dll {
            continue;
        }

        let Some(file_name) = src_path.file_name() else {
            continue;
        };
        let dst_path = target_dir.join(file_name);
        let src_len = fs::metadata(&src_path).map(|m| m.len()).unwrap_or(0);
        let dst_len = fs::metadata(&dst_path).map(|m| m.len()).unwrap_or(u64::MAX);
        if !dst_path.is_file() || dst_len != src_len {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
            copied += 1;
        }
    }

    Ok(copied)
}

/// Ensure bundled Windows MSVC runtime DLLs sit directly next to an extracted executable.
pub fn ensure_windows_vcruntime_in_dir(
    target_dir: &Path,
    resource_dir: Option<&Path>,
) -> Result<usize> {
    let Some(src_dir) = find_bundled_vcruntime_dir(resource_dir) else {
        return Ok(0);
    };
    copy_vcruntime_dlls_from_dir(&src_dir, target_dir)
}

/// Extract the filename segment from a download URL.
pub fn filename_from_url(url: &str) -> String {
    let without_query = url.split('?').next().unwrap_or(url);
    without_query
        .rsplit('/')
        .find(|seg| !seg.trim().is_empty())
        .unwrap_or("downloaded.bin")
        .to_string()
}

/// Return true if a catalog item is a binary/runtime component installed under `<bin_dir>`.
pub fn is_binary_kind(kind: CatalogItemKind) -> bool {
    matches!(
        kind,
        CatalogItemKind::Ffmpeg
            | CatalogItemKind::CudaRuntime
            | CatalogItemKind::WhisperBin
            | CatalogItemKind::LlamaBin
    )
}

fn marker_file_path(bin_subdir: &Path, item_id: &str) -> PathBuf {
    bin_subdir.join(format!(".installed-{}", item_id))
}

/// Resolve the expected primary file path for a catalog item on disk.
pub fn expected_item_path(item: &CatalogItem, paths: &StoragePaths) -> PathBuf {
    if is_binary_kind(item.kind) {
        let subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
        match &item.executable_name {
            Some(rel) if !rel.trim().is_empty() => subdir.join(rel),
            _ => marker_file_path(&subdir, &item.id),
        }
    } else {
        let subdir = Path::new(&paths.models_dir).join(&item.target_subdir);
        match &item.executable_name {
            Some(rel) if !rel.trim().is_empty() => subdir.join(rel),
            _ => subdir.join(filename_from_url(&item.url)),
        }
    }
}

/// Search the system `PATH` for an executable by name.
pub fn find_in_system_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    #[cfg(windows)]
    let candidates: Vec<String> = if binary_name.to_ascii_lowercase().ends_with(".exe") {
        vec![binary_name.to_string()]
    } else {
        vec![format!("{}.exe", binary_name), binary_name.to_string()]
    };
    #[cfg(not(windows))]
    let candidates: Vec<String> = vec![binary_name.to_string()];

    for dir in std::env::split_paths(&path_var) {
        for cand in &candidates {
            let full = dir.join(cand);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Check whether a specific `CatalogItem` is installed in `paths` (considering marker files when
/// multiple binary items share a `target_subdir` and `executable_name`, plus system `ffmpeg` fallback on Linux).
pub fn check_component_installed(
    item: &CatalogItem,
    all_items: &[CatalogItem],
    paths: &StoragePaths,
) -> InstalledComponentStatus {
    let target_path = expected_item_path(item, paths);

    if is_binary_kind(item.kind) {
        let subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
        let own_marker = marker_file_path(&subdir, &item.id);

        // Check if another catalog item of the SAME kind in the same subdir has an explicit marker
        let sibling_markers: Vec<&CatalogItem> = all_items
            .iter()
            .filter(|other| {
                other.id != item.id
                    && other.kind == item.kind
                    && other.target_subdir == item.target_subdir
                    && marker_file_path(&subdir, &other.id).is_file()
            })
            .collect();

        if own_marker.is_file() && target_path.is_file() {
            let size = fs::metadata(&target_path)
                .map(|m| m.len())
                .unwrap_or(item.size_bytes);
            return InstalledComponentStatus {
                id: item.id.clone(),
                installed: true,
                path: Some(target_path.to_string_lossy().into_owned()),
                size_bytes: if size > 0 { size } else { item.size_bytes },
            };
        }

        // For CudaRuntime when executable_name is None or on Windows checking cudart64_12.dll
        if item.kind == CatalogItemKind::CudaRuntime {
            let dll_path = subdir.join("cudart64_12.dll");
            if own_marker.is_file() || dll_path.is_file() {
                let effective = if dll_path.is_file() {
                    dll_path
                } else {
                    own_marker
                };
                return InstalledComponentStatus {
                    id: item.id.clone(),
                    installed: true,
                    path: Some(effective.to_string_lossy().into_owned()),
                    size_bytes: item.size_bytes,
                };
            }
        }

        // If no marker exists at all for this kind/subdir, accept target_path if present for the primary/required variant
        if sibling_markers.is_empty() && !own_marker.is_file() && target_path.is_file() {
            let has_required_sibling = all_items.iter().any(|other| {
                other.id != item.id
                    && other.kind == item.kind
                    && other.platform == item.platform
                    && other.target_subdir == item.target_subdir
                    && other.executable_name == item.executable_name
                    && other.required
            });
            if item.required || !has_required_sibling {
                return InstalledComponentStatus {
                    id: item.id.clone(),
                    installed: true,
                    path: Some(target_path.to_string_lossy().into_owned()),
                    size_bytes: item.size_bytes,
                };
            }
        }

        // Special case: Linux FFmpeg can use system `ffmpeg` on PATH when not downloaded locally
        if item.id == "ffmpeg-linux-x64" && current_platform_tag() == "linux-x64" {
            if let Some(sys_ffmpeg) = find_in_system_path("ffmpeg") {
                let size = fs::metadata(&sys_ffmpeg).map(|m| m.len()).unwrap_or(0);
                return InstalledComponentStatus {
                    id: item.id.clone(),
                    installed: true,
                    path: Some(sys_ffmpeg.to_string_lossy().into_owned()),
                    size_bytes: size,
                };
            }
        }

        InstalledComponentStatus {
            id: item.id.clone(),
            installed: false,
            path: None,
            size_bytes: 0,
        }
    } else {
        // Model items (VadModel, WhisperModel, LlmModel)
        if let Ok(meta) = fs::metadata(&target_path) {
            if meta.is_file() {
                let len = meta.len();
                let size_ok = if item.size_bytes > 0 {
                    len == item.size_bytes
                } else {
                    len > 0
                };
                if size_ok {
                    return InstalledComponentStatus {
                        id: item.id.clone(),
                        installed: true,
                        path: Some(target_path.to_string_lossy().into_owned()),
                        size_bytes: len,
                    };
                }
            }
        }

        InstalledComponentStatus {
            id: item.id.clone(),
            installed: false,
            path: None,
            size_bytes: 0,
        }
    }
}

/// Select only the core AI runtime components (FFmpeg, CUDA/CPU binaries, Whisper, llama.cpp, VAD,
/// and default Whisper ASR model) without downloading a heavy local LLM translation model.
pub fn select_core_runtime_bundle_ids(gpu: &GpuInfo, platform: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let use_gpu_binaries =
        gpu.has_nvidia && gpu.cuda_supported && gpu.recommended_tier != VramTier::CpuOnly;

    if platform == "windows-x64" {
        ids.push("ffmpeg-win-x64".to_string());
        if use_gpu_binaries {
            ids.push("whisper-win-cuda".to_string());
            ids.push("cudart-win-cuda12.4".to_string());
            ids.push("llama-win-cuda".to_string());
        } else {
            ids.push("whisper-win-cpu".to_string());
            ids.push("llama-win-cpu".to_string());
        }
    } else {
        ids.push("ffmpeg-linux-x64".to_string());
        ids.push("whisper-linux-x64".to_string());
        if use_gpu_binaries {
            ids.push("llama-linux-vulkan".to_string());
        } else {
            ids.push("llama-linux-cpu".to_string());
        }
    }

    ids.push("vad-silero-v6.2.0".to_string());
    ids.push("whisper-large-v3-turbo-q5_0".to_string());

    ids
}

/// Select the recommended local LLM translation model ID for a given GPU profile.
pub fn select_recommended_llm_id(gpu: &GpuInfo) -> &'static str {
    match gpu.recommended_tier {
        VramTier::Vram12GbPlus => "llm-gemma4-12b-qat",
        VramTier::Vram8Gb => "llm-gemma4-e4b-qat",
        VramTier::Vram4Gb | VramTier::CpuOnly => "llm-gemma3-4b-q4km",
    }
}

/// Select the complete recommended offline bundle (core runtime + recommended LLM translator).
pub fn select_recommended_bundle_ids(gpu: &GpuInfo, platform: &str) -> Vec<String> {
    let mut ids = select_core_runtime_bundle_ids(gpu, platform);
    ids.push(select_recommended_llm_id(gpu).to_string());
    ids
}

/// Compute setup status from an explicit catalog, GPU info, storage paths, and custom models.
pub fn compute_setup_status(
    catalog: &CatalogManifest,
    gpu: GpuInfo,
    storage: StoragePaths,
    custom_models: &[CustomModelEntry],
    platform: &str,
) -> SetupStatus {
    let components: Vec<InstalledComponentStatus> = catalog
        .items
        .iter()
        .map(|item| check_component_installed(item, &catalog.items, &storage))
        .collect();

    let is_installed = |id: &str| -> bool {
        components
            .iter()
            .any(|c| c.id == id && c.installed)
    };

    let has_kind_for_platform = |kind: CatalogItemKind| -> bool {
        catalog
            .items
            .iter()
            .filter(|i| i.kind == kind && (i.platform == platform || i.platform == "all"))
            .any(|i| is_installed(&i.id))
            || custom_models
                .iter()
                .any(|cm| cm.kind == kind && Path::new(&cm.path).is_file())
    };

    let is_item_satisfied = |item: &CatalogItem| -> bool {
        match item.kind {
            CatalogItemKind::Ffmpeg => {
                has_kind_for_platform(CatalogItemKind::Ffmpeg)
                    || find_in_system_path("ffmpeg").is_some()
            }
            CatalogItemKind::WhisperBin => {
                has_kind_for_platform(CatalogItemKind::WhisperBin)
                    || find_in_system_path("whisper-cli").is_some()
            }
            CatalogItemKind::CudaRuntime => {
                // Only required when llama CUDA is present or CPU llama is not installed
                is_installed("cudart-win-cuda12.4") || is_installed("llama-win-cpu")
            }
            CatalogItemKind::LlamaBin => {
                has_kind_for_platform(CatalogItemKind::LlamaBin)
                    || find_in_system_path("llama-server").is_some()
            }
            CatalogItemKind::VadModel => has_kind_for_platform(CatalogItemKind::VadModel),
            CatalogItemKind::WhisperModel => has_kind_for_platform(CatalogItemKind::WhisperModel),
            CatalogItemKind::LlmModel => has_kind_for_platform(CatalogItemKind::LlmModel),
        }
    };

    let core_ids = select_core_runtime_bundle_ids(&gpu, platform);
    let mut missing_required = Vec::new();
    let mut core_runtime_ready = true;

    for core_id in &core_ids {
        let Some(item) = catalog.items.iter().find(|i| i.id == *core_id) else {
            continue;
        };
        if !is_item_satisfied(item) {
            core_runtime_ready = false;
            missing_required.push(core_id.clone());
        }
    }

    let has_llm_model = has_kind_for_platform(CatalogItemKind::LlmModel);
    if !has_llm_model {
        missing_required.push(select_recommended_llm_id(&gpu).to_string());
    }

    let ready_for_offline = core_runtime_ready && has_llm_model;

    SetupStatus {
        platform: platform.to_string(),
        gpu,
        storage,
        components,
        core_runtime_ready,
        has_llm_model,
        ready_for_offline,
        missing_required,
    }
}

/// Evaluate current hardware, storage paths, and installed components.
pub fn get_setup_status(app: &AppHandle) -> Result<SetupStatus> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    let gpu = hardware::detect_gpu();
    Ok(compute_setup_status(
        &catalog,
        gpu,
        paths,
        &config.custom_models,
        current_platform_tag(),
    ))
}

/// Compute lowercase hex SHA-256 digest of a file on disk.
pub fn compute_file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open file for SHA-256: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("Failed reading file for SHA-256: {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Verify a file's SHA-256 matches `expected_hex` (case-insensitive, mandatory 64 hex chars).
pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<()> {
    let expected = validate_sha256_hex(Some(expected_hex))?;
    let actual = compute_file_sha256(path)?;
    if actual != expected {
        return Err(anyhow!(
            "SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable_permission(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if path.is_file() {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable_permission(_path: &Path) -> Result<()> {
    Ok(())
}

fn sanitize_archive_rel_path(raw: &Path, label: &str) -> Result<Option<PathBuf>> {
    use std::path::Component;

    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!(
                    "{label} archive contains unsafe path traversal entry: {}",
                    raw.display()
                ));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(clean))
    }
}

fn ensure_no_symlink_components_under(root: &Path, rel: &Path) -> Result<()> {
    let mut cursor = root.to_path_buf();
    for comp in rel.components() {
        cursor.push(comp);
        if let Ok(meta) = fs::symlink_metadata(&cursor) {
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "Refusing to extract through symlink component: {}",
                    cursor.display()
                ));
            }
        }
    }
    Ok(())
}

fn extract_zip_archive_with_limits(
    archive_path: &Path,
    target_dir: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<()> {
    fs::create_dir_all(target_dir).with_context(|| {
        format!(
            "Failed to create zip target directory: {}",
            target_dir.display()
        )
    })?;

    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open zip archive: {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("Invalid zip archive: {}", archive_path.display()))?;

    if archive.len() > max_files {
        return Err(anyhow!(
            "Zip archive exceeds maximum allowed entry count ({} > {})",
            archive.len(),
            max_files
        ));
    }

    let mut file_count = 0usize;
    let mut total_bytes = 0u64;

    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .with_context(|| format!("Failed to read zip entry #{}", idx))?;

        let is_unix_symlink = entry
            .unix_mode()
            .is_some_and(|mode| (mode & 0o170000) == 0o120000);
        if entry.is_symlink() || is_unix_symlink {
            return Err(anyhow!(
                "Zip archive contains disallowed symlink entry: {}",
                entry.name()
            ));
        }

        let Some(enclosed) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            return Err(anyhow!(
                "Zip archive contains unsafe path traversal entry: {}",
                entry.name()
            ));
        };
        let Some(rel_path) = sanitize_archive_rel_path(&enclosed, "Zip")? else {
            continue;
        };
        ensure_no_symlink_components_under(target_dir, &rel_path)?;

        let out_path = target_dir.join(&rel_path);
        if entry.is_dir() || entry.name().ends_with('/') {
            fs::create_dir_all(&out_path)?;
        } else {
            file_count += 1;
            if file_count > max_files {
                return Err(anyhow!(
                    "Zip archive exceeds maximum allowed file count ({max_files})"
                ));
            }
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let remaining = max_bytes.saturating_sub(total_bytes);
            let mut out_file = File::create(&out_path).with_context(|| {
                format!("Failed to create extracted file: {}", out_path.display())
            })?;
            let mut limited = (&mut entry).take(remaining.saturating_add(1));
            let copied = std::io::copy(&mut limited, &mut out_file).with_context(|| {
                let _ = fs::remove_file(&out_path);
                format!("Failed to extract zip entry to {}", out_path.display())
            })?;
            out_file.flush()?;
            drop(out_file);

            if copied > remaining {
                let _ = fs::remove_file(&out_path);
                return Err(anyhow!(
                    "Zip archive exceeds maximum decompressed size limit ({max_bytes} bytes)"
                ));
            }
            total_bytes = total_bytes.saturating_add(copied);

            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                if mode & 0o111 != 0 {
                    let _ = set_executable_permission(&out_path);
                }
            }
        }
    }

    Ok(())
}

/// Extract a `.zip` archive safely into `target_dir` (rejecting symlinks, zip-slip traversal,
/// and archive bombs exceeding file-count or decompressed-byte caps).
pub fn extract_zip_archive(archive_path: &Path, target_dir: &Path) -> Result<()> {
    extract_zip_archive_with_limits(archive_path, target_dir, MAX_ARCHIVE_FILES, MAX_ARCHIVE_BYTES)
}

fn is_shared_lib_soname(name: &str) -> bool {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return false;
    }
    if let Some(stem) = name.strip_suffix(".so") {
        return !stem.is_empty();
    }
    if let Some((prefix, suffix)) = name.split_once(".so.") {
        !prefix.is_empty()
            && !suffix.is_empty()
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    } else {
        false
    }
}

fn validate_same_dir_soname_symlink(rel_path: &Path, link_target: &Path) -> Result<String> {
    use std::path::Component;

    let leaf_name = rel_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            anyhow!(
                "Tar.gz archive contains disallowed symlink with invalid name: {}",
                rel_path.display()
            )
        })?;
    if !is_shared_lib_soname(leaf_name) {
        return Err(anyhow!(
            "Tar.gz archive contains disallowed non-shared-library symlink: {}",
            rel_path.display()
        ));
    }

    let mut comps = link_target.components();
    let target_str = match (comps.next(), comps.next()) {
        (Some(Component::Normal(single)), None) => single.to_str().ok_or_else(|| {
            anyhow!(
                "Tar.gz archive contains disallowed symlink with non-UTF-8 target: {}",
                rel_path.display()
            )
        })?,
        _ => {
            return Err(anyhow!(
                "Tar.gz archive contains disallowed symlink target '{}' for {}",
                link_target.display(),
                rel_path.display()
            ));
        }
    };

    if !is_shared_lib_soname(target_str) || target_str == leaf_name {
        return Err(anyhow!(
            "Tar.gz archive contains disallowed symlink target '{}' for {}",
            target_str,
            rel_path.display()
        ));
    }

    Ok(target_str.to_string())
}

fn extract_tar_gz_archive_with_limits(
    archive_path: &Path,
    target_dir: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<()> {
    fs::create_dir_all(target_dir).with_context(|| {
        format!(
            "Failed to create tar.gz target directory: {}",
            target_dir.display()
        )
    })?;

    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open tar.gz archive: {}", archive_path.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);

    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    let mut extracted_non_symlink_paths: HashSet<PathBuf> = HashSet::new();
    let mut seen_symlink_paths: HashSet<PathBuf> = HashSet::new();
    let mut pending_so_symlinks: Vec<(PathBuf, String)> = Vec::new();

    for entry_res in archive
        .entries()
        .context("Failed to read entries from tar.gz archive")?
    {
        let mut entry = entry_res.context("Corrupt entry in tar.gz archive")?;
        let entry_type = entry.header().entry_type();
        let entry_path = entry
            .path()
            .context("Invalid path in tar.gz archive")?
            .into_owned();

        if entry_type == tar::EntryType::Link {
            return Err(anyhow!(
                "Tar.gz archive contains disallowed hardlink entry: {}",
                entry_path.display()
            ));
        }

        if entry_type == tar::EntryType::Symlink {
            let Some(rel_path) = sanitize_archive_rel_path(&entry_path, "Tar.gz")? else {
                return Err(anyhow!(
                    "Tar.gz archive contains disallowed root symlink entry"
                ));
            };
            let link_target = entry
                .link_name()
                .context("Invalid symlink target in tar.gz archive")?
                .ok_or_else(|| {
                    anyhow!(
                        "Tar.gz archive contains symlink without target: {}",
                        entry_path.display()
                    )
                })?
                .into_owned();
            let validated_target = validate_same_dir_soname_symlink(&rel_path, &link_target)?;
            if !seen_symlink_paths.insert(rel_path.clone())
                || extracted_non_symlink_paths.contains(&rel_path)
            {
                return Err(anyhow!(
                    "Tar.gz archive contains duplicate or colliding symlink entry: {}",
                    rel_path.display()
                ));
            }
            file_count += 1;
            if file_count > max_files {
                return Err(anyhow!(
                    "Tar.gz archive exceeds maximum allowed file count ({max_files})"
                ));
            }
            // Defer creation until after all regular files/dirs are extracted so no file can
            // ever extract through a symlink.
            pending_so_symlinks.push((rel_path, validated_target));
            continue;
        }

        if !matches!(
            entry_type,
            tar::EntryType::Regular
                | tar::EntryType::Continuous
                | tar::EntryType::Directory
                | tar::EntryType::GNULongName
                | tar::EntryType::GNULongLink
                | tar::EntryType::XHeader
                | tar::EntryType::XGlobalHeader
        ) {
            return Err(anyhow!(
                "Tar.gz archive contains unsupported entry type {:?} for {}",
                entry_type,
                entry_path.display()
            ));
        }

        let Some(rel_path) = sanitize_archive_rel_path(&entry_path, "Tar.gz")? else {
            continue;
        };
        if entry_type.is_dir() || entry_type.is_file() {
            if rel_path
                .ancestors()
                .any(|anc| !anc.as_os_str().is_empty() && seen_symlink_paths.contains(anc))
            {
                return Err(anyhow!(
                    "Tar.gz archive contains colliding symlink and file/directory entry: {}",
                    rel_path.display()
                ));
            }
            for anc in rel_path.ancestors() {
                if !anc.as_os_str().is_empty() {
                    extracted_non_symlink_paths.insert(anc.to_path_buf());
                }
            }
        }
        ensure_no_symlink_components_under(target_dir, &rel_path)?;

        let out_path = target_dir.join(&rel_path);
        if entry_type.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if !entry_type.is_file() {
            continue;
        }

        file_count += 1;
        if file_count > max_files {
            return Err(anyhow!(
                "Tar.gz archive exceeds maximum allowed file count ({max_files})"
            ));
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let remaining = max_bytes.saturating_sub(total_bytes);
        let mut out_file = File::create(&out_path).with_context(|| {
            format!("Failed to create extracted file: {}", out_path.display())
        })?;
        let mode = entry.header().mode().ok();
        let mut limited = (&mut entry).take(remaining.saturating_add(1));
        let copied = std::io::copy(&mut limited, &mut out_file).with_context(|| {
            let _ = fs::remove_file(&out_path);
            format!("Failed to extract tar.gz entry to {}", out_path.display())
        })?;
        out_file.flush()?;
        drop(out_file);

        if copied > remaining {
            let _ = fs::remove_file(&out_path);
            return Err(anyhow!(
                "Tar.gz archive exceeds maximum decompressed size limit ({max_bytes} bytes)"
            ));
        }
        total_bytes = total_bytes.saturating_add(copied);

        #[cfg(unix)]
        if let Some(m) = mode {
            if m & 0o111 != 0 {
                let _ = set_executable_permission(&out_path);
            }
        }
        #[cfg(not(unix))]
        let _ = mode;
    }

    // Create validated same-directory ELF shared-library SONAME symlinks strictly after all
    // regular files/directories have been extracted.
    #[cfg(unix)]
    for (rel_path, target_name) in pending_so_symlinks {
        if let Some(parent_rel) = rel_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            ensure_no_symlink_components_under(target_dir, parent_rel)?;
        }
        let link_path = target_dir.join(&rel_path);
        match fs::symlink_metadata(&link_path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                // Allow replacing an existing symlink on reinstall or Vulkan <-> CPU backend switch.
                fs::remove_file(&link_path).with_context(|| {
                    format!(
                        "Failed to replace existing shared-library symlink: {}",
                        link_path.display()
                    )
                })?;
            }
            Ok(_) => {
                return Err(anyhow!(
                    "Tar.gz archive symlink destination already exists as a non-symlink: {}",
                    link_path.display()
                ));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "Failed to inspect symlink destination metadata: {}",
                        link_path.display()
                    )
                });
            }
        }
        if let Some(parent) = link_path.parent() {
            fs::create_dir_all(parent)?;
        }
        std::os::unix::fs::symlink(&target_name, &link_path).with_context(|| {
            format!(
                "Failed to create shared-library symlink {} -> {}",
                link_path.display(),
                target_name
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = pending_so_symlinks;

    Ok(())
}

/// Extract a `.tar.gz` archive safely into `target_dir` (rejecting hardlinks, unsafe symlinks,
/// path traversal, and archive bombs, while safely deferring same-directory `.so` SONAME symlinks).
pub fn extract_tar_gz_archive(archive_path: &Path, target_dir: &Path) -> Result<()> {
    extract_tar_gz_archive_with_limits(
        archive_path,
        target_dir,
        MAX_ARCHIVE_FILES,
        MAX_ARCHIVE_BYTES,
    )
}

fn move_file_across_fs(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(src, dst).is_err() {
        fs::copy(src, dst).with_context(|| {
            format!(
                "Failed to copy downloaded file from {} to {}",
                src.display(),
                dst.display()
            )
        })?;
        let _ = fs::remove_file(src);
    }
    Ok(())
}

fn make_progress_event(
    item_id: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
    stage: &str,
    message: String,
) -> DownloadProgressEvent {
    let percent = if total_bytes > 0 {
        ((downloaded_bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0)
    } else if stage == "completed" {
        100.0
    } else {
        0.0
    };

    DownloadProgressEvent {
        item_id: item_id.to_string(),
        downloaded_bytes,
        total_bytes,
        percent,
        stage: stage.to_string(),
        message,
    }
}

/// Reuse `cublas64_12.dll`, `cublasLt64_12.dll`, and `cudart64_12.dll` already extracted by
/// `whisper-win-cuda` so Windows CUDA installs do not download the 391 MB runtime ZIP twice.
pub fn try_reuse_cuda_dlls_from_whisper(paths: &StoragePaths) -> Result<bool> {
    let whisper_release = Path::new(&paths.bin_dir).join("whisper").join("Release");
    let llama_dir = Path::new(&paths.bin_dir).join("llama");

    let all_present = CUDA_RUNTIME_DLLS.iter().all(|dll| {
        let p = whisper_release.join(dll);
        fs::metadata(&p).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
    });
    if !all_present {
        return Ok(false);
    }

    fs::create_dir_all(&llama_dir).with_context(|| {
        format!(
            "Failed to create llama binary directory: {}",
            llama_dir.display()
        )
    })?;

    for dll in CUDA_RUNTIME_DLLS {
        let src = whisper_release.join(dll);
        let dst = llama_dir.join(dll);
        let src_len = fs::metadata(&src)?.len();
        let dst_len = fs::metadata(&dst).map(|m| m.len()).unwrap_or(u64::MAX);
        if !dst.is_file() || dst_len != src_len {
            fs::copy(&src, &dst).with_context(|| {
                format!("Failed copying {} to {}", src.display(), dst.display())
            })?;
        }
    }

    Ok(true)
}

fn ensure_vcruntime_for_item_dir(
    item: &CatalogItem,
    paths: &StoragePaths,
    resource_dir: Option<&Path>,
) {
    if item.platform != "windows-x64"
        || !matches!(
            item.kind,
            CatalogItemKind::WhisperBin | CatalogItemKind::LlamaBin
        )
    {
        return;
    }

    let exe_path = expected_item_path(item, paths);
    if let Some(exe_parent) = exe_path.parent() {
        let _ = ensure_windows_vcruntime_in_dir(exe_parent, resource_dir);
    }
}

fn remove_stale_cpu_cuda_dlls(item: &CatalogItem, paths: &StoragePaths) {
    if item.id == "whisper-win-cpu" || item.id == "llama-win-cpu" {
        let subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
        for rel in ["ggml-cuda.dll", "Release/ggml-cuda.dll"] {
            let _ = fs::remove_file(subdir.join(rel));
        }
    }
}

/// Download, checksum-verify, and extract/install a single catalog component into explicit storage
/// paths, reporting progress through `on_progress` (shared by Tauri UI and headless CLI bootstrap).
pub async fn install_component_to_paths<F>(
    catalog: &CatalogManifest,
    paths: &StoragePaths,
    config_path: &Path,
    resource_dir: Option<&Path>,
    item_id: &str,
    mut on_progress: F,
) -> Result<InstalledComponentStatus>
where
    F: FnMut(DownloadProgressEvent),
{
    let item = catalog
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| anyhow!("Catalog item '{}' not found", item_id))?
        .clone();

    // Enforce HTTPS allowlist and mandatory 64-hex SHA-256 upfront before any download/install.
    let (validated_url, expected_sha) = validate_catalog_item_for_download(&item)?;

    if item.id == "cudart-win-cuda12.4" && try_reuse_cuda_dlls_from_whisper(paths)? {
        let target_subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
        let marker = marker_file_path(&target_subdir, &item.id);
        fs::write(&marker, item.version.as_bytes())?;
        let status = check_component_installed(&item, &catalog.items, paths);
        if status.installed {
            on_progress(make_progress_event(
                &item.id,
                item.size_bytes,
                item.size_bytes,
                "completed",
                format!("{} reused from local Whisper CUDA package.", item.name),
            ));
            return Ok(status);
        }
    }

    let downloads_dir = Path::new(&paths.data_dir).join("downloads");
    fs::create_dir_all(&downloads_dir).with_context(|| {
        format!(
            "Failed to create downloads staging directory: {}",
            downloads_dir.display()
        )
    })?;

    let part_path = downloads_dir.join(format!("{}.part", item.id));
    let mut existing_len = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
    if item.size_bytes > 0 && existing_len > item.size_bytes {
        let _ = fs::remove_file(&part_path);
        existing_len = 0;
    }

    let max_allowed_bytes = if item.size_bytes > 0 {
        item.size_bytes
            .saturating_add(DOWNLOAD_SIZE_MARGIN_BYTES)
            .min(ABSOLUTE_MAX_DOWNLOAD_BYTES)
    } else {
        ABSOLUTE_MAX_DOWNLOAD_BYTES
    };

    on_progress(make_progress_event(
        &item.id,
        existing_len,
        item.size_bytes,
        "downloading",
        format!("Downloading {}...", item.name),
    ));

    if item.size_bytes == 0 || existing_len < item.size_bytes {
        let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("Too many redirects while downloading catalog component");
            }
            let url = attempt.url();
            if url.scheme() != "https" {
                return attempt.error("Refusing non-HTTPS redirect during catalog download");
            }
            let host = url
                .host_str()
                .map(|h| h.to_ascii_lowercase())
                .unwrap_or_default();
            if !is_allowed_redirect_host(&host) {
                return attempt.error(format!(
                    "Refusing redirect to non-allowlisted host '{host}'"
                ));
            }
            attempt.follow()
        });

        let client = reqwest::Client::builder()
            .user_agent(concat!("EasySRT-Studio/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .redirect(redirect_policy)
            .build()
            .context("Failed to initialize HTTP download client")?;

        let mut req = client.get(validated_url.clone());
        if existing_len > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", existing_len));
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}", validated_url))?;

        let status = resp.status();
        let (mut part_file, mut downloaded_bytes) =
            if status == reqwest::StatusCode::PARTIAL_CONTENT && existing_len > 0 {
                let f = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part_path)
                    .with_context(|| {
                        format!(
                            "Failed to open partial file for resume: {}",
                            part_path.display()
                        )
                    })?;
                (f, existing_len)
            } else if status.is_success() {
                let f = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&part_path)
                    .with_context(|| {
                        format!("Failed to create download file: {}", part_path.display())
                    })?;
                (f, 0u64)
            } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
                && item.size_bytes > 0
                && existing_len == item.size_bytes
            {
                let f = OpenOptions::new().write(true).open(&part_path)?;
                (f, existing_len)
            } else {
                return Err(anyhow!(
                    "Download failed for '{}' ({}): HTTP {}",
                    item.name,
                    validated_url,
                    status
                ));
            };

        if let Some(cl) = resp.content_length() {
            if downloaded_bytes.saturating_add(cl) > max_allowed_bytes {
                drop(part_file);
                let _ = fs::remove_file(&part_path);
                return Err(anyhow!(
                    "Download for '{}' exceeds maximum allowed size ({} > {} bytes)",
                    item.id,
                    downloaded_bytes.saturating_add(cl),
                    max_allowed_bytes
                ));
            }
        }

        let total_bytes = if item.size_bytes > 0 {
            item.size_bytes
        } else {
            resp.content_length()
                .map(|cl| cl.saturating_add(downloaded_bytes))
                .unwrap_or(0)
        };

        let mut stream = resp.bytes_stream();
        let mut last_emit = Instant::now();

        loop {
            let next_chunk = tokio::time::timeout(DOWNLOAD_CHUNK_STALL_TIMEOUT, stream.next())
                .await
                .map_err(|_| {
                    anyhow!(
                        "Download stalled for '{}' (no data received for {}s)",
                        item.id,
                        DOWNLOAD_CHUNK_STALL_TIMEOUT.as_secs()
                    )
                })?;

            let Some(chunk_res) = next_chunk else {
                break;
            };

            let chunk = chunk_res
                .with_context(|| format!("Network stream interrupted for {}", item.id))?;
            let next_total = downloaded_bytes.saturating_add(chunk.len() as u64);
            if next_total > max_allowed_bytes {
                drop(part_file);
                let _ = fs::remove_file(&part_path);
                return Err(anyhow!(
                    "Download for '{}' exceeded maximum allowed byte ceiling ({} bytes)",
                    item.id,
                    max_allowed_bytes
                ));
            }
            part_file
                .write_all(&chunk)
                .with_context(|| format!("Disk write error for {}", part_path.display()))?;
            downloaded_bytes = next_total;

            if last_emit.elapsed() >= Duration::from_millis(100) {
                on_progress(make_progress_event(
                    &item.id,
                    downloaded_bytes,
                    total_bytes,
                    "downloading",
                    format!("Downloading {}...", item.name),
                ));
                last_emit = Instant::now();
            }
        }
        part_file.flush()?;
    }

    // Mandatory SHA-256 checksum verification (never skipped)
    on_progress(make_progress_event(
        &item.id,
        item.size_bytes,
        item.size_bytes,
        "verifying",
        format!("Verifying SHA-256 checksum for {}...", item.name),
    ));
    if let Err(err) = verify_file_sha256(&part_path, &expected_sha) {
        let _ = fs::remove_file(&part_path);
        return Err(err);
    }

    // Extract or move into target location
    on_progress(make_progress_event(
        &item.id,
        item.size_bytes,
        item.size_bytes,
        "extracting",
        format!("Installing {}...", item.name),
    ));

    if is_binary_kind(item.kind) {
        let target_subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
        fs::create_dir_all(&target_subdir)?;
        remove_stale_cpu_cuda_dlls(&item, paths);

        match item.archive_type {
            ArchiveType::Zip => {
                extract_zip_archive(&part_path, &target_subdir)?;
                let _ = fs::remove_file(&part_path);
            }
            ArchiveType::TarGz => {
                extract_tar_gz_archive(&part_path, &target_subdir)?;
                let _ = fs::remove_file(&part_path);
            }
            ArchiveType::Raw => {
                let dest_path = expected_item_path(&item, paths);
                move_file_across_fs(&part_path, &dest_path)?;
                set_executable_permission(&dest_path)?;
            }
        }

        if let Some(ref rel_exe) = item.executable_name {
            let exe_path = target_subdir.join(rel_exe);
            let _ = set_executable_permission(&exe_path);
        }

        ensure_vcruntime_for_item_dir(&item, paths, resource_dir);

        // Remove competing markers of the same CatalogItemKind in this subdir
        for other in &catalog.items {
            if other.id != item.id
                && other.kind == item.kind
                && other.target_subdir == item.target_subdir
            {
                let stale_marker = marker_file_path(&target_subdir, &other.id);
                let _ = fs::remove_file(stale_marker);
            }
        }

        // Write installation marker
        let marker = marker_file_path(&target_subdir, &item.id);
        fs::write(&marker, item.version.as_bytes())?;
    } else {
        let dest_path = expected_item_path(&item, paths);
        move_file_across_fs(&part_path, &dest_path)?;

        // Auto-select newly installed model if none is currently active (serialized RMW)
        if matches!(
            item.kind,
            CatalogItemKind::WhisperModel | CatalogItemKind::LlmModel
        ) {
            let item_kind = item.kind;
            let item_id_clone = item.id.clone();
            let _ = storage::update_config_at_path(config_path, |cfg| {
                if item_kind == CatalogItemKind::WhisperModel
                    && cfg.active_whisper_model_id.is_none()
                {
                    cfg.active_whisper_model_id = Some(item_id_clone);
                } else if item_kind == CatalogItemKind::LlmModel
                    && cfg.active_llm_model_id.is_none()
                {
                    cfg.active_llm_model_id = Some(item_id_clone);
                }
            });
        }
    }

    let status = check_component_installed(&item, &catalog.items, paths);
    if !status.installed {
        return Err(anyhow!(
            "Component '{}' failed post-install verification at expected path {}",
            item.id,
            expected_item_path(&item, paths).display()
        ));
    }

    on_progress(make_progress_event(
        &item.id,
        item.size_bytes,
        item.size_bytes,
        "completed",
        format!("{} installed successfully.", item.name),
    ));

    Ok(status)
}

/// Download, checksum-verify, and extract/install a single catalog component by `item_id`.
pub async fn install_component(
    app: &AppHandle,
    item_id: &str,
) -> Result<InstalledComponentStatus> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    let resource_dir = app.path().resource_dir().ok();
    let app_handle = app.clone();

    install_component_to_paths(
        &catalog,
        &paths,
        Path::new(&paths.config_path),
        resource_dir.as_deref(),
        item_id,
        |evt| {
            let _ = app_handle.emit(EVENT_DOWNLOAD_PROGRESS, evt);
        },
    )
    .await
}

fn check_cuda_probe_output(
    label: &str,
    code: Option<i32>,
    success: bool,
    stdout: &str,
    stderr: &str,
) -> std::result::Result<(), String> {
    const STATUS_DLL_NOT_FOUND: i32 = -1_073_741_515; // 0xC0000135
    const STATUS_ENTRYPOINT_NOT_FOUND: i32 = -1_073_741_511; // 0xC0000139
    const STATUS_INVALID_IMAGE_FORMAT: i32 = -1_073_741_701; // 0xC000007B

    let stderr_trim = stderr.trim();
    let stdout_trim = stdout.trim();
    let combined_lower = format!("{stdout_trim}\n{stderr_trim}").to_ascii_lowercase();
    let stderr_lower = stderr_trim.to_ascii_lowercase();

    let missing_dll_code = matches!(
        code,
        Some(STATUS_DLL_NOT_FOUND)
            | Some(STATUS_ENTRYPOINT_NOT_FOUND)
            | Some(STATUS_INVALID_IMAGE_FORMAT)
    );
    let missing_dll_text = stderr_lower.contains("0xc0000135")
        || combined_lower.contains("0xc0000135")
        || ((stderr_lower.contains("nvcuda.dll") || stderr_lower.contains("cublas"))
            && (stderr_lower.contains("not found")
                || stderr_lower.contains("missing")
                || stderr_lower.contains("cannot load")
                || stderr_lower.contains("could not")
                || stderr_lower.contains("failed")
                || !success));

    if !success || missing_dll_code || missing_dll_text {
        let code_desc = match code {
            Some(c) if c < 0 => format!("exit code {} (0x{:08X})", c, c as u32),
            Some(c) => format!("exit code {}", c),
            None => "terminated by signal".to_string(),
        };
        let detail = if !stderr_trim.is_empty() {
            stderr_trim
        } else if !stdout_trim.is_empty() {
            stdout_trim
        } else {
            "missing CUDA/DLL dependency or runtime load failure"
        };
        return Err(format!("{label} --help failed ({code_desc}): {detail}"));
    }

    Ok(())
}

/// Smoke-test extracted Windows CUDA binaries (`whisper-cli.exe --help` and `llama-server.exe --help`)
/// using the same binary PATH/working-directory environment used at runtime, detecting missing
/// DLL errors (`0xC0000135` / `nvcuda.dll` / `cublas`) before leaving CUDA binaries active.
pub fn probe_windows_cuda_runtime(paths: &StoragePaths) -> std::result::Result<(), String> {
    let whisper_exe = Path::new(&paths.bin_dir)
        .join("whisper")
        .join("Release")
        .join("whisper-cli.exe");
    let llama_exe = Path::new(&paths.bin_dir)
        .join("llama")
        .join("llama-server.exe");

    for (label, exe) in [
        ("whisper-cli.exe", whisper_exe),
        ("llama-server.exe", llama_exe),
    ] {
        if !exe.is_file() {
            return Err(format!("{label} not found at {}", exe.display()));
        }

        let mut cmd = Command::new(&exe);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let (dir, path_val, ld_val) = crate::pipeline::ffmpeg::build_binary_env_strings(&exe);
        if let Some(parent) = dir {
            cmd.current_dir(parent);
        }
        if let Some(path) = path_val {
            cmd.env("PATH", path);
        }
        if let Some(ld) = ld_val {
            cmd.env("LD_LIBRARY_PATH", ld);
        }

        cmd.arg("--help")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|err| format!("Failed to launch {label} at {}: {err}", exe.display()))?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stdout_pipe {
                let _ = pipe.take(256 * 1024).read_to_end(&mut buf);
            }
            buf
        });
        let stderr_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stderr_pipe {
                let _ = pipe.take(256 * 1024).read_to_end(&mut buf);
            }
            buf
        });

        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() >= CUDA_PROBE_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdout_handle.join();
                        let _ = stderr_handle.join();
                        return Err(format!(
                            "Timed out probing {label} --help after {}s",
                            CUDA_PROBE_TIMEOUT.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(format!("Failed polling {label} --help: {err}"));
                }
            }
        };

        let stdout_bytes = stdout_handle.join().unwrap_or_default();
        let stderr_bytes = stderr_handle.join().unwrap_or_default();
        let stdout = String::from_utf8_lossy(&stdout_bytes);
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        check_cuda_probe_output(label, status.code(), status.success(), &stdout, &stderr)?;
    }

    Ok(())
}

async fn install_windows_cpu_fallback<F>(
    catalog: &CatalogManifest,
    paths: &StoragePaths,
    config_path: &Path,
    resource_dir: Option<&Path>,
    probe_err: &str,
    on_progress: &mut F,
) -> Result<()>
where
    F: FnMut(DownloadProgressEvent),
{
    tracing::warn!(
        "Windows CUDA runtime probe failed ({probe_err}); automatically installing CPU fallback binaries"
    );
    on_progress(make_progress_event(
        "cuda-probe",
        0,
        0,
        "warning",
        format!(
            "CUDA runtime probe failed ({probe_err}). Automatically installing CPU fallback binaries..."
        ),
    ));

    // Remove stale CUDA runtime marker so status cleanly reflects CPU fallback
    let llama_dir = Path::new(&paths.bin_dir).join("llama");
    let _ = fs::remove_file(marker_file_path(&llama_dir, "cudart-win-cuda12.4"));

    for fallback_id in ["whisper-win-cpu", "llama-win-cpu"] {
        install_component_to_paths(
            catalog,
            paths,
            config_path,
            resource_dir,
            fallback_id,
            &mut *on_progress,
        )
        .await?;
    }

    Ok(())
}

/// Install the core AI runtime bundle (FFmpeg, CUDA/CPU, Whisper, llama.cpp, VAD, default Whisper
/// model) into explicit storage paths without installing large translation LLMs.
pub async fn install_core_runtime_to_paths<F>(
    catalog: &CatalogManifest,
    gpu: GpuInfo,
    paths: &StoragePaths,
    config_path: &Path,
    resource_dir: Option<&Path>,
    platform: &str,
    on_progress: F,
) -> Result<SetupStatus>
where
    F: FnMut(DownloadProgressEvent),
{
    install_core_runtime_to_paths_with_options(
        catalog,
        gpu,
        paths,
        config_path,
        resource_dir,
        platform,
        true,
        on_progress,
    )
    .await
}

/// Same as `install_core_runtime_to_paths`, allowing CLI bootstrap tests to disable automatic
/// Windows CUDA->CPU fallback when inspecting raw CUDA binary layout.
#[allow(clippy::too_many_arguments)]
pub async fn install_core_runtime_to_paths_with_options<F>(
    catalog: &CatalogManifest,
    mut gpu: GpuInfo,
    paths: &StoragePaths,
    config_path: &Path,
    resource_dir: Option<&Path>,
    platform: &str,
    allow_cuda_fallback: bool,
    mut on_progress: F,
) -> Result<SetupStatus>
where
    F: FnMut(DownloadProgressEvent),
{
    let use_gpu_binaries =
        gpu.has_nvidia && gpu.cuda_supported && gpu.recommended_tier != VramTier::CpuOnly;
    let bundle_ids = select_core_runtime_bundle_ids(&gpu, platform);
    let mut installed_whisper_id: Option<String> = None;

    for item_id in &bundle_ids {
        let Some(item) = catalog.items.iter().find(|i| i.id == *item_id) else {
            continue;
        };

        if item.kind == CatalogItemKind::WhisperModel {
            installed_whisper_id = Some(item.id.clone());
        }

        let current_status = check_component_installed(item, &catalog.items, paths);
        if !current_status.installed {
            install_component_to_paths(
                catalog,
                paths,
                config_path,
                resource_dir,
                item_id,
                &mut on_progress,
            )
            .await?;
        } else {
            // Repair bundled MSVC runtime DLLs beside already-extracted Windows binaries
            ensure_vcruntime_for_item_dir(item, paths, resource_dir);
        }

        if platform == "windows-x64"
            && use_gpu_binaries
            && allow_cuda_fallback
            && item_id == "llama-win-cuda"
        {
            if let Err(probe_err) = probe_windows_cuda_runtime(paths) {
                install_windows_cpu_fallback(
                    catalog,
                    paths,
                    config_path,
                    resource_dir,
                    &probe_err,
                    &mut on_progress,
                )
                .await?;
                gpu.cuda_supported = false;
                gpu.recommended_tier = VramTier::CpuOnly;
                gpu.cuda_status_note = Some(format!(
                    "CUDA runtime smoke-test failed ({probe_err}); switched automatically to CPU mode."
                ));
            }
        }
    }

    let updated_config = storage::update_config_at_path(config_path, |cfg| {
        if cfg.active_whisper_model_id.is_none() {
            if let Some(w_id) = installed_whisper_id {
                cfg.active_whisper_model_id = Some(w_id);
            }
        }
    })?;

    Ok(compute_setup_status(
        catalog,
        gpu,
        paths.clone(),
        &updated_config.custom_models,
        platform,
    ))
}

/// Install missing core runtime components inside the running Tauri application.
pub async fn install_core_runtime(app: &AppHandle) -> Result<SetupStatus> {
    let gpu = hardware::detect_gpu();
    let platform = current_platform_tag();
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    let resource_dir = app.path().resource_dir().ok();
    let app_handle = app.clone();

    install_core_runtime_to_paths(
        &catalog,
        gpu,
        &paths,
        Path::new(&paths.config_path),
        resource_dir.as_deref(),
        platform,
        |evt| {
            let _ = app_handle.emit(EVENT_DOWNLOAD_PROGRESS, evt);
        },
    )
    .await
}

/// Install the complete recommended offline bundle for the detected GPU tier and platform,
/// update active model selections in `AppConfig`, and return the resulting `SetupStatus`.
pub async fn install_recommended_bundle(app: &AppHandle) -> Result<SetupStatus> {
    let mut gpu = hardware::detect_gpu();
    let platform = current_platform_tag();
    let use_gpu_binaries =
        gpu.has_nvidia && gpu.cuda_supported && gpu.recommended_tier != VramTier::CpuOnly;
    let bundle_ids = select_recommended_bundle_ids(&gpu, platform);

    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    let resource_dir = app.path().resource_dir().ok();
    let app_handle = app.clone();

    let mut installed_whisper_id: Option<String> = None;
    let mut installed_llm_id: Option<String> = None;

    for item_id in &bundle_ids {
        let Some(item) = catalog.items.iter().find(|i| i.id == *item_id) else {
            continue;
        };

        if item.kind == CatalogItemKind::WhisperModel {
            installed_whisper_id = Some(item.id.clone());
        } else if item.kind == CatalogItemKind::LlmModel {
            installed_llm_id = Some(item.id.clone());
        }

        let current_status = check_component_installed(item, &catalog.items, &paths);
        if !current_status.installed {
            install_component_to_paths(
                &catalog,
                &paths,
                Path::new(&paths.config_path),
                resource_dir.as_deref(),
                item_id,
                |evt| {
                    let _ = app_handle.emit(EVENT_DOWNLOAD_PROGRESS, evt);
                },
            )
            .await?;
        } else {
            ensure_vcruntime_for_item_dir(item, &paths, resource_dir.as_deref());
        }

        if platform == "windows-x64" && use_gpu_binaries && item_id == "llama-win-cuda" {
            if let Err(probe_err) = probe_windows_cuda_runtime(&paths) {
                let mut emit_progress = |evt: DownloadProgressEvent| {
                    let _ = app_handle.emit(EVENT_DOWNLOAD_PROGRESS, evt);
                };
                install_windows_cpu_fallback(
                    &catalog,
                    &paths,
                    Path::new(&paths.config_path),
                    resource_dir.as_deref(),
                    &probe_err,
                    &mut emit_progress,
                )
                .await?;
                gpu.cuda_supported = false;
                gpu.recommended_tier = VramTier::CpuOnly;
                gpu.cuda_status_note = Some(format!(
                    "CUDA runtime smoke-test failed ({probe_err}); switched automatically to CPU mode."
                ));
            }
        }
    }

    let updated_config = storage::update_config(app, |cfg| {
        if let Some(w_id) = installed_whisper_id {
            cfg.active_whisper_model_id = Some(w_id);
        }
        if let Some(l_id) = installed_llm_id {
            cfg.active_llm_model_id = Some(l_id);
        }
        cfg.has_completed_setup = true;
    })?;

    Ok(compute_setup_status(
        &catalog,
        gpu,
        paths,
        &updated_config.custom_models,
        platform,
    ))
}

/// Run headless core runtime bootstrap (`--install-runtime`) invoked by NSIS `NSIS_HOOK_POSTINSTALL`.
/// Logs flushed ASCII progress to stdout and `<data_dir>/setup-bootstrap.log`, returning `0` on success.
pub fn run_cli_bootstrap(args: &[String]) -> i32 {
    let mut data_dir_override: Option<PathBuf> = None;
    let mut force_cpu = false;
    let mut force_cuda = false;
    let mut no_cuda_fallback = false;

    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "--data-dir" {
            if let Some(val) = args.get(idx + 1) {
                if !val.trim().is_empty() {
                    data_dir_override = Some(PathBuf::from(val));
                }
                idx += 1;
            }
        } else if let Some(val) = arg.strip_prefix("--data-dir=") {
            if !val.trim().is_empty() {
                data_dir_override = Some(PathBuf::from(val));
            }
        } else if arg == "--force-cpu" {
            force_cpu = true;
        } else if arg == "--force-cuda" {
            force_cuda = true;
        } else if arg == "--no-cuda-fallback" {
            no_cuda_fallback = true;
        }
        idx += 1;
    }

    let root_dir = data_dir_override.unwrap_or_else(storage::default_installed_root_dir);
    let paths = match storage::resolve_storage_paths_from_root(
        &root_dir,
        crate::models::StorageMode::Installed,
        None,
    ) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("[EasySRT Setup] Failed to initialize storage directories: {err:#}");
            return 1;
        }
    };

    let log_path = Path::new(&paths.data_dir).join("setup-bootstrap.log");
    let mut log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .ok();

    let mut write_log = |line: &str| {
        println!("{line}");
        let _ = std::io::stdout().flush();
        if let Some(ref mut f) = log_file {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    };

    write_log(&format!(
        "[EasySRT Setup] Starting core runtime bootstrap in {}",
        paths.root_dir
    ));

    let catalog = match load_catalog_from_dirs(None) {
        Ok(c) => c,
        Err(err) => {
            write_log(&format!("[EasySRT Setup] Failed to load catalog: {err:#}"));
            return 1;
        }
    };

    let mut gpu = if force_cpu {
        GpuInfo {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: 0,
            recommended_tier: VramTier::CpuOnly,
            driver_version: None,
            cuda_supported: false,
            cuda_status_note: None,
        }
    } else if force_cuda {
        GpuInfo {
            has_nvidia: true,
            gpu_name: Some("Simulated NVIDIA CUDA GPU".to_string()),
            vram_mb: 8192,
            recommended_tier: VramTier::Vram8Gb,
            driver_version: Some("555.00".to_string()),
            cuda_supported: true,
            cuda_status_note: None,
        }
    } else {
        hardware::detect_gpu()
    };

    if gpu.has_nvidia && (!gpu.cuda_supported || gpu.recommended_tier == VramTier::CpuOnly) {
        let reason = gpu
            .cuda_status_note
            .as_deref()
            .unwrap_or("NVIDIA GPU does not meet CUDA 12.4 requirements; using CPU runtime.");
        write_log(&format!("[EasySRT Setup] {reason}"));
        gpu.cuda_supported = false;
        gpu.recommended_tier = VramTier::CpuOnly;
    } else if gpu.has_nvidia {
        write_log(&format!(
            "[EasySRT Setup] Detected GPU: {} ({} MB VRAM, driver {}) -> CUDA 12.4 bundle",
            gpu.gpu_name.as_deref().unwrap_or("NVIDIA GPU"),
            gpu.vram_mb,
            gpu.driver_version.as_deref().unwrap_or("unknown"),
        ));
    } else {
        write_log("[EasySRT Setup] No NVIDIA GPU detected -> CPU runtime bundle");
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            write_log(&format!(
                "[EasySRT Setup] Failed to create async runtime: {err:#}"
            ));
            return 1;
        }
    };

    let platform = current_platform_tag();
    let mut last_item = String::new();
    let mut last_stage = String::new();
    let mut last_bucket = i32::MIN;

    let result = rt.block_on(install_core_runtime_to_paths_with_options(
        &catalog,
        gpu,
        &paths,
        Path::new(&paths.config_path),
        None,
        platform,
        !no_cuda_fallback,
        |evt| {
            if evt.stage == "warning" {
                write_log(&format!("[EasySRT Setup] WARNING: {}", evt.message));
                return;
            }
            let bucket = (evt.percent / 10.0).floor() as i32;
            let stage_changed = evt.item_id != last_item || evt.stage != last_stage;
            if stage_changed || (evt.stage == "downloading" && bucket > last_bucket) {
                last_item = evt.item_id.clone();
                last_stage = evt.stage.clone();
                last_bucket = bucket;
                write_log(&format!(
                    "[EasySRT Setup] [{}] {} ({:.0}%)",
                    evt.item_id, evt.stage, evt.percent
                ));
            }
        },
    ));

    match result {
        Ok(status) if status.core_runtime_ready => {
            write_log("[EasySRT Setup] Core AI runtime ready (FFmpeg, Whisper, llama.cpp, VAD).");
            0
        }
        Ok(status) => {
            write_log(&format!(
                "[EasySRT Setup] Core runtime incomplete. Missing: {:?}",
                status.missing_required
            ));
            1
        }
        Err(err) => {
            write_log(&format!("[EasySRT Setup] ERROR: {err:#}"));
            1
        }
    }
}

/// Import a user-supplied `.bin` or `.gguf` model file into `AppConfig.custom_models`.
pub fn import_custom_model_with_config(
    config: &mut AppConfig,
    source_path: &str,
    kind: CatalogItemKind,
    name: Option<String>,
) -> Result<CustomModelEntry> {
    if !matches!(
        kind,
        CatalogItemKind::WhisperModel | CatalogItemKind::LlmModel | CatalogItemKind::VadModel
    ) {
        return Err(anyhow!(
            "Custom model import only supports WhisperModel, LlmModel, or VadModel"
        ));
    }

    let path = Path::new(source_path.trim());
    if !path.is_file() {
        return Err(anyhow!(
            "Custom model file does not exist: {}",
            path.display()
        ));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "bin" && ext != "gguf" {
        return Err(anyhow!(
            "Unsupported model extension '.{}' (expected .bin or .gguf)",
            ext
        ));
    }

    let meta = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for {}", path.display()))?;
    if meta.len() == 0 {
        return Err(anyhow!("Custom model file is empty: {}", path.display()));
    }

    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let canonical_str = canonical.to_string_lossy().into_owned();

    let display_name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Custom Model")
                .to_string()
        });

    if let Some(existing) = config
        .custom_models
        .iter_mut()
        .find(|m| m.path == canonical_str)
    {
        existing.name = display_name;
        existing.kind = kind;
        existing.size_bytes = meta.len();
        let entry = existing.clone();
        if kind == CatalogItemKind::WhisperModel && config.active_whisper_model_id.is_none() {
            config.active_whisper_model_id = Some(entry.id.clone());
        } else if kind == CatalogItemKind::LlmModel && config.active_llm_model_id.is_none() {
            config.active_llm_model_id = Some(entry.id.clone());
        }
        return Ok(entry);
    }

    let entry = CustomModelEntry {
        id: format!("custom-{}", Uuid::new_v4()),
        name: display_name,
        kind,
        path: canonical_str,
        size_bytes: meta.len(),
    };

    if kind == CatalogItemKind::WhisperModel && config.active_whisper_model_id.is_none() {
        config.active_whisper_model_id = Some(entry.id.clone());
    } else if kind == CatalogItemKind::LlmModel && config.active_llm_model_id.is_none() {
        config.active_llm_model_id = Some(entry.id.clone());
    }

    config.custom_models.push(entry.clone());
    Ok(entry)
}

/// Import a custom model and persist the updated `AppConfig`.
pub fn import_custom_model(
    app: &AppHandle,
    source_path: &str,
    kind: CatalogItemKind,
    name: Option<String>,
) -> Result<CustomModelEntry> {
    storage::try_update_config(app, |cfg| {
        import_custom_model_with_config(cfg, source_path, kind, name)
    })
}

/// Delete a downloaded catalog model/component or remove an imported custom model entry.
pub fn delete_model(app: &AppHandle, model_id: &str) -> Result<()> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    let catalog_item = catalog.items.iter().find(|i| i.id == model_id).cloned();

    storage::try_update_config(app, |cfg| {
        let before_len = cfg.custom_models.len();
        cfg.custom_models.retain(|m| m.id != model_id);
        let removed_custom = cfg.custom_models.len() != before_len;

        if let Some(ref item) = catalog_item {
            let target_path = expected_item_path(item, &paths);
            if target_path.is_file() {
                fs::remove_file(&target_path).with_context(|| {
                    format!("Failed to delete model file: {}", target_path.display())
                })?;
            }
            if is_binary_kind(item.kind) {
                let subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
                let marker = marker_file_path(&subdir, &item.id);
                let _ = fs::remove_file(marker);
            }
        } else if !removed_custom {
            return Err(anyhow!(
                "Model '{}' not found in catalog or custom models",
                model_id
            ));
        }

        if cfg.active_whisper_model_id.as_deref() == Some(model_id) {
            cfg.active_whisper_model_id = None;
        }
        if cfg.active_llm_model_id.as_deref() == Some(model_id) {
            cfg.active_llm_model_id = None;
        }

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Pipeline Path Resolution Helpers (called by `crate::pipeline`)
// ---------------------------------------------------------------------------

/// Core testable binary path resolver for `Ffmpeg`, `WhisperBin`, or `LlamaBin`.
pub fn resolve_binary_path_with_state(
    catalog: &CatalogManifest,
    paths: &StoragePaths,
    kind: CatalogItemKind,
    platform: &str,
) -> Result<PathBuf> {
    // 1. Check installed catalog binaries for current platform (preferring required/GPU variants first)
    let mut candidates: Vec<&CatalogItem> = catalog
        .items
        .iter()
        .filter(|i| i.kind == kind && (i.platform == platform || i.platform == "all"))
        .collect();
    candidates.sort_by_key(|i| !i.required);

    for item in candidates {
        let status = check_component_installed(item, &catalog.items, paths);
        if status.installed {
            if let Some(p) = status.path {
                let pb = PathBuf::from(p);
                if pb.is_file() {
                    return Ok(pb);
                }
            }
        }
    }

    // 2. Fall back to system PATH
    let system_names: &[&str] = match kind {
        CatalogItemKind::Ffmpeg => &["ffmpeg"],
        CatalogItemKind::WhisperBin => &["whisper-cli", "whisper-cpp"],
        CatalogItemKind::LlamaBin => &["llama-server"],
        _ => &[],
    };

    for name in system_names {
        if let Some(found) = find_in_system_path(name) {
            return Ok(found);
        }
    }

    Err(anyhow!(
        "Required binary ({:?}) is not installed. Please run Setup Wizard to download required tools.",
        kind
    ))
}

/// Resolve the executable path for `Ffmpeg`, `WhisperBin`, or `LlamaBin`.
pub fn resolve_binary_path(app: &AppHandle, kind: CatalogItemKind) -> Result<PathBuf> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    resolve_binary_path_with_state(&catalog, &paths, kind, current_platform_tag())
}

/// Resolve the Silero VAD model path if installed.
pub fn resolve_vad_model_path(app: &AppHandle) -> Result<Option<PathBuf>> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;

    for cm in &config.custom_models {
        if cm.kind == CatalogItemKind::VadModel {
            let p = PathBuf::from(&cm.path);
            if p.is_file() {
                return Ok(Some(p));
            }
        }
    }

    for item in catalog
        .items
        .iter()
        .filter(|i| i.kind == CatalogItemKind::VadModel)
    {
        let status = check_component_installed(item, &catalog.items, &paths);
        if status.installed {
            if let Some(p) = status.path {
                return Ok(Some(PathBuf::from(p)));
            }
        }
    }

    Ok(None)
}

/// Core testable active model path resolver for `WhisperModel` or `LlmModel`.
pub fn resolve_active_model_path_with_state(
    catalog: &CatalogManifest,
    paths: &StoragePaths,
    custom_models: &[CustomModelEntry],
    active_id: Option<&str>,
    kind: CatalogItemKind,
) -> Result<PathBuf> {
    // 1. Check explicitly configured active_id first
    if let Some(id) = active_id {
        if let Some(cm) = custom_models
            .iter()
            .find(|m| m.id == id && m.kind == kind)
        {
            let p = PathBuf::from(&cm.path);
            if p.is_file() {
                return Ok(p);
            }
        }

        if let Some(item) = catalog
            .items
            .iter()
            .find(|i| i.id == id && i.kind == kind)
        {
            let status = check_component_installed(item, &catalog.items, paths);
            if status.installed {
                if let Some(p) = status.path {
                    return Ok(PathBuf::from(p));
                }
            }
        }
    }

    // 2. Fallback to any installed custom model of this kind
    for cm in custom_models.iter().filter(|m| m.kind == kind) {
        let p = PathBuf::from(&cm.path);
        if p.is_file() {
            return Ok(p);
        }
    }

    // 3. Fallback to any installed catalog model of this kind
    for item in catalog.items.iter().filter(|i| i.kind == kind) {
        let status = check_component_installed(item, &catalog.items, paths);
        if status.installed {
            if let Some(p) = status.path {
                return Ok(PathBuf::from(p));
            }
        }
    }

    Err(anyhow!(
        "No installed {:?} found. Please download or import a model in Setup / Settings.",
        kind
    ))
}

/// Resolve the active Whisper GGML model `.bin` path.
pub fn resolve_active_whisper_model_path(app: &AppHandle) -> Result<PathBuf> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    resolve_active_model_path_with_state(
        &catalog,
        &paths,
        &config.custom_models,
        config.active_whisper_model_id.as_deref(),
        CatalogItemKind::WhisperModel,
    )
}

/// Resolve the active local LLM `.gguf` model path.
pub fn resolve_active_llm_model_path(app: &AppHandle) -> Result<PathBuf> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;
    resolve_active_model_path_with_state(
        &catalog,
        &paths,
        &config.custom_models,
        config.active_llm_model_id.as_deref(),
        CatalogItemKind::LlmModel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StorageMode;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::collections::HashSet;
    use zip::write::SimpleFileOptions;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("easysrt_setup_test_{}_{}", prefix, Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn embedded_catalog_parses_all_20_verified_items() {
        let catalog = load_embedded_catalog().unwrap();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.items.len(), 20);

        let mut seen_ids = HashSet::new();
        for item in &catalog.items {
            assert!(
                seen_ids.insert(item.id.clone()),
                "Duplicate catalog item id: {}",
                item.id
            );
            assert!(!item.name.is_empty());
            assert!(item.url.starts_with("https://"));
            assert!(item.size_bytes > 0);
            let sha = item.sha256.as_ref().expect("Every item must have sha256");
            assert_eq!(sha.len(), 64, "Invalid sha256 length on {}", item.id);
            assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        }

        // Verify critical Windows CUDA 12.4 subdir isolation rule from #4:
        // whisper-win-cuda must use "whisper", while cudart-win-cuda12.4 and llama-win-cuda share "llama"
        let whisper_cuda = catalog
            .items
            .iter()
            .find(|i| i.id == "whisper-win-cuda")
            .unwrap();
        let cudart_win = catalog
            .items
            .iter()
            .find(|i| i.id == "cudart-win-cuda12.4")
            .unwrap();
        let llama_cuda = catalog
            .items
            .iter()
            .find(|i| i.id == "llama-win-cuda")
            .unwrap();

        assert_eq!(whisper_cuda.target_subdir, "whisper");
        assert_eq!(cudart_win.target_subdir, "llama");
        assert_eq!(llama_cuda.target_subdir, "llama");
    }

    #[test]
    fn sha256_verification_helper_accepts_valid_and_rejects_invalid() {
        let dir = make_temp_dir("sha256");
        let file_path = dir.join("payload.bin");
        fs::write(&file_path, b"EasySRT Studio verification payload").unwrap();

        // Precompute expected SHA-256
        let digest = compute_file_sha256(&file_path).unwrap();
        assert_eq!(digest.len(), 64);

        // Case-insensitive match succeeds
        verify_file_sha256(&file_path, &digest.to_uppercase()).unwrap();

        // Mismatched hash fails
        let bad_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_file_sha256(&file_path, bad_hash).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_zip_and_tar_gz_archives_correctly() {
        let dir = make_temp_dir("archives");

        // 1. Build a test .zip archive with nested Release/whisper-cli.exe
        let zip_path = dir.join("test_whisper.zip");
        {
            let file = File::create(&zip_path).unwrap();
            let mut zip_writer = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            zip_writer
                .start_file("Release/whisper-cli.exe", opts)
                .unwrap();
            zip_writer.write_all(b"fake-whisper-binary-data").unwrap();
            zip_writer.finish().unwrap();
        }

        let zip_out = dir.join("zip_extracted");
        extract_zip_archive(&zip_path, &zip_out).unwrap();
        let extracted_exe = zip_out.join("Release").join("whisper-cli.exe");
        assert!(extracted_exe.is_file());
        assert_eq!(
            fs::read(&extracted_exe).unwrap(),
            b"fake-whisper-binary-data"
        );

        // 2. Build a test .tar.gz archive with nested llama-b12633/llama-server
        let targz_path = dir.join("test_llama.tar.gz");
        {
            let file = File::create(&targz_path).unwrap();
            let enc = GzEncoder::new(file, Compression::default());
            let mut tar_builder = tar::Builder::new(enc);

            let payload = b"fake-llama-server-elf";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar_builder
                .append_data(&mut header, "llama-b12633/llama-server", &payload[..])
                .unwrap();
            let enc = tar_builder.into_inner().unwrap();
            enc.finish().unwrap();
        }

        let targz_out = dir.join("targz_extracted");
        extract_tar_gz_archive(&targz_path, &targz_out).unwrap();
        let extracted_llama = targz_out.join("llama-b12633").join("llama-server");
        assert!(extracted_llama.is_file());
        assert_eq!(fs::read(&extracted_llama).unwrap(), b"fake-llama-server-elf");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recommended_bundle_selection_across_gpu_tiers_and_platforms() {
        // 1. Windows 4GB NVIDIA (e.g. RTX 3050 Laptop)
        let gpu_4gb = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 3050 Laptop GPU".to_string()),
            vram_mb: 4096,
            recommended_tier: VramTier::Vram4Gb,
            driver_version: Some("551.86".to_string()),
            cuda_supported: true,
            cuda_status_note: None,
        };
        let win_4gb = select_recommended_bundle_ids(&gpu_4gb, "windows-x64");
        assert_eq!(
            win_4gb,
            vec![
                "ffmpeg-win-x64",
                "whisper-win-cuda",
                "cudart-win-cuda12.4",
                "llama-win-cuda",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo-q5_0",
                "llm-gemma3-4b-q4km",
            ]
        );

        // 2. Windows 8GB NVIDIA (e.g. RTX 4060)
        let gpu_8gb = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 4060".to_string()),
            vram_mb: 8188,
            recommended_tier: VramTier::Vram8Gb,
            driver_version: Some("555.85".to_string()),
            cuda_supported: true,
            cuda_status_note: None,
        };
        let win_8gb = select_recommended_bundle_ids(&gpu_8gb, "windows-x64");
        assert!(win_8gb.contains(&"whisper-large-v3-turbo-q5_0".to_string()));
        assert!(win_8gb.contains(&"llm-gemma4-e4b-qat".to_string()));

        // 3. Linux 12GB+ NVIDIA (e.g. RTX 4070 Ti / 3090) — still defaults to Large V3 Turbo Q5_0
        let gpu_12gb = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 3090".to_string()),
            vram_mb: 24576,
            recommended_tier: VramTier::Vram12GbPlus,
            driver_version: Some("560.35.03".to_string()),
            cuda_supported: true,
            cuda_status_note: None,
        };
        let linux_12gb = select_recommended_bundle_ids(&gpu_12gb, "linux-x64");
        assert_eq!(
            linux_12gb,
            vec![
                "ffmpeg-linux-x64",
                "whisper-linux-x64",
                "llama-linux-vulkan",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo-q5_0",
                "llm-gemma4-12b-qat",
            ]
        );

        // 4. Windows CPU-only fallback — also defaults to Large V3 Turbo Q5_0
        let cpu_only = GpuInfo {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: 0,
            recommended_tier: VramTier::CpuOnly,
            driver_version: None,
            cuda_supported: false,
            cuda_status_note: None,
        };
        let win_cpu = select_recommended_bundle_ids(&cpu_only, "windows-x64");
        assert_eq!(
            win_cpu,
            vec![
                "ffmpeg-win-x64",
                "whisper-win-cpu",
                "llama-win-cpu",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo-q5_0",
                "llm-gemma3-4b-q4km",
            ]
        );

        // 5. NVIDIA GPU with unsupported driver (cuda_supported = false) uses CPU binaries
        let old_driver_gpu = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 3060".to_string()),
            vram_mb: 12288,
            recommended_tier: VramTier::Vram12GbPlus,
            driver_version: Some("535.98".to_string()),
            cuda_supported: false,
            cuda_status_note: Some("Old driver".to_string()),
        };
        let win_old_drv = select_core_runtime_bundle_ids(&old_driver_gpu, "windows-x64");
        assert_eq!(
            win_old_drv,
            vec![
                "ffmpeg-win-x64",
                "whisper-win-cpu",
                "llama-win-cpu",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo-q5_0",
            ]
        );
    }

    #[test]
    fn custom_model_import_and_active_model_resolution() {
        let dir = make_temp_dir("custom_model");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Installed, None).unwrap();
        let catalog = load_embedded_catalog().unwrap();
        let mut config = AppConfig::default();

        let fake_gguf = dir.join("my-serbian-llm.gguf");
        fs::write(&fake_gguf, b"GGUF-model-weights").unwrap();

        let entry = import_custom_model_with_config(
            &mut config,
            fake_gguf.to_str().unwrap(),
            CatalogItemKind::LlmModel,
            Some("My Serbian LLM".to_string()),
        )
        .unwrap();

        assert_eq!(entry.name, "My Serbian LLM");
        assert_eq!(config.active_llm_model_id.as_deref(), Some(entry.id.as_str()));

        let resolved = resolve_active_model_path_with_state(
            &catalog,
            &paths,
            &config.custom_models,
            config.active_llm_model_id.as_deref(),
            CatalogItemKind::LlmModel,
        )
        .unwrap();
        assert_eq!(resolved, fs::canonicalize(&fake_gguf).unwrap());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn core_runtime_bundle_excludes_llm_and_includes_cuda_on_nvidia() {
        let catalog = load_embedded_catalog().unwrap();
        let gpu_4gb = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 3050 Laptop GPU".to_string()),
            vram_mb: 4096,
            recommended_tier: VramTier::Vram4Gb,
            driver_version: Some("551.86".to_string()),
            cuda_supported: true,
            cuda_status_note: None,
        };
        let core_ids = select_core_runtime_bundle_ids(&gpu_4gb, "windows-x64");
        assert_eq!(
            core_ids,
            vec![
                "ffmpeg-win-x64",
                "whisper-win-cuda",
                "cudart-win-cuda12.4",
                "llama-win-cuda",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo-q5_0",
            ]
        );
        assert!(!core_ids.iter().any(|id| id.starts_with("llm-")));

        let cpu_only = GpuInfo {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: 0,
            recommended_tier: VramTier::CpuOnly,
            driver_version: None,
            cuda_supported: false,
            cuda_status_note: None,
        };
        let win_cpu_core = select_core_runtime_bundle_ids(&cpu_only, "windows-x64");
        assert_eq!(
            win_cpu_core,
            vec![
                "ffmpeg-win-x64",
                "whisper-win-cpu",
                "llama-win-cpu",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo-q5_0",
            ]
        );

        let dir = make_temp_dir("core_status");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Installed, None).unwrap();
        for core_id in &win_cpu_core {
            let item = catalog.items.iter().find(|i| i.id == *core_id).unwrap();
            let target = expected_item_path(item, &paths);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let payload = if item.size_bytes > 0 && !is_binary_kind(item.kind) {
                // Sparse/sized dummy model check isn't needed if we set len on file
                let f = File::create(&target).unwrap();
                f.set_len(item.size_bytes).unwrap();
                continue;
            } else {
                b"bin".as_slice()
            };
            fs::write(&target, payload).unwrap();
            if is_binary_kind(item.kind) {
                let subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
                fs::write(marker_file_path(&subdir, &item.id), b"1").unwrap();
            }
        }

        let status = compute_setup_status(&catalog, cpu_only, paths, &[], "windows-x64");
        assert_eq!(status.platform, "windows-x64");
        assert!(status.core_runtime_ready);
        assert!(!status.has_llm_model);
        assert!(!status.ready_for_offline);
        assert_eq!(status.missing_required, vec!["llm-gemma3-4b-q4km"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reuses_cuda_runtime_dlls_from_whisper_dir_when_present() {
        let dir = make_temp_dir("reuse_cuda");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Installed, None).unwrap();
        assert!(!try_reuse_cuda_dlls_from_whisper(&paths).unwrap());

        let whisper_release = Path::new(&paths.bin_dir).join("whisper").join("Release");
        fs::create_dir_all(&whisper_release).unwrap();
        for dll in CUDA_RUNTIME_DLLS {
            fs::write(whisper_release.join(dll), b"cuda-dll-bytes").unwrap();
        }

        assert!(try_reuse_cuda_dlls_from_whisper(&paths).unwrap());
        let llama_dir = Path::new(&paths.bin_dir).join("llama");
        for dll in CUDA_RUNTIME_DLLS {
            assert_eq!(fs::read(llama_dir.join(dll)).unwrap(), b"cuda-dll-bytes");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_vcruntime_dlls_into_target_directory() {
        let dir = make_temp_dir("vcruntime_copy");
        let src = dir.join("vcruntime");
        let dst = dir.join("target_bin");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("vcruntime140.dll"), b"vcrt140").unwrap();
        fs::write(src.join("msvcp140.dll"), b"msvcp140").unwrap();
        fs::write(src.join("README.txt"), b"ignore-non-dll").unwrap();

        let copied = copy_vcruntime_dlls_from_dir(&src, &dst).unwrap();
        assert_eq!(copied, 2);
        assert!(dst.join("vcruntime140.dll").is_file());
        assert!(dst.join("msvcp140.dll").is_file());
        assert!(!dst.join("README.txt").exists());

        // Second pass skips unchanged DLLs cleanly
        assert_eq!(copy_vcruntime_dlls_from_dir(&src, &dst).unwrap(), 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cuda_probe_detects_missing_dll_exit_codes_and_error_text() {
        assert!(check_cuda_probe_output("whisper-cli.exe", Some(0), true, "usage: whisper-cli", "").is_ok());

        let ntstatus_err =
            check_cuda_probe_output("whisper-cli.exe", Some(-1_073_741_515), false, "", "")
                .unwrap_err();
        assert!(ntstatus_err.contains("0xC0000135"));

        let nvcuda_err = check_cuda_probe_output(
            "llama-server.exe",
            Some(0),
            true,
            "",
            "Failed to load nvcuda.dll",
        )
        .unwrap_err();
        assert!(nvcuda_err.contains("nvcuda.dll"));

        let cublas_err = check_cuda_probe_output(
            "llama-server.exe",
            Some(1),
            false,
            "",
            "cublas64_12.dll was not found",
        )
        .unwrap_err();
        assert!(cublas_err.contains("cublas64_12.dll"));

        // Missing binaries in a fresh directory should fail probe cleanly
        let dir = make_temp_dir("cuda_probe_missing");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Installed, None).unwrap();
        assert!(probe_windows_cuda_runtime(&paths).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validates_mandatory_sha256_and_https_allowlisted_hosts() {
        // Every embedded catalog item must satisfy both URL allowlist and 64-hex SHA-256
        let catalog = load_embedded_catalog().unwrap();
        for item in &catalog.items {
            validate_catalog_item_for_download(item)
                .unwrap_or_else(|e| panic!("Embedded item '{}' failed validation: {e}", item.id));
        }

        // SHA-256 rejection cases
        assert!(validate_sha256_hex(None).is_err());
        assert!(validate_sha256_hex(Some("")).is_err());
        assert!(validate_sha256_hex(Some("   ")).is_err());
        assert!(validate_sha256_hex(Some("deadbeef")).is_err());
        assert!(validate_sha256_hex(Some(&"z".repeat(64))).is_err());
        assert!(validate_sha256_hex(Some(&"a".repeat(63))).is_err());
        assert!(validate_sha256_hex(Some(&"a".repeat(65))).is_err());
        assert_eq!(
            validate_sha256_hex(Some(&"A".repeat(64))).unwrap(),
            "a".repeat(64)
        );

        // Allowlisted HTTPS URLs pass
        for ok_url in [
            "https://github.com/ggml-org/whisper.cpp/releases/download/b1/x.zip",
            "https://objects.githubusercontent.com/some/asset",
            "https://huggingface.co/ggml-org/whisper-vad/resolve/main/model.bin",
            "https://cdn-lfs.huggingface.co/repos/abc/def",
            "https://cas-bridge.xethub.hf.co/xet-bridge-us/123",
        ] {
            assert!(
                validate_catalog_download_url(ok_url).is_ok(),
                "Expected allowlisted URL to pass: {ok_url}"
            );
        }

        // Non-HTTPS, credentialed, or non-allowlisted hosts are rejected
        for bad_url in [
            "http://github.com/ggml-org/whisper.cpp/releases/download/b1/x.zip",
            "http://huggingface.co/model.bin",
            "https://evil.example.com/model.bin",
            "https://github.com.evil.example/asset.zip",
            "https://user:pass@github.com/asset.zip",
            "file:///etc/passwd",
        ] {
            assert!(
                validate_catalog_download_url(bad_url).is_err(),
                "Expected URL to be rejected: {bad_url}"
            );
        }
    }

    #[tokio::test]
    async fn install_component_rejects_missing_sha256_or_disallowed_host_before_network() {
        let dir = make_temp_dir("install_sec_reject");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Installed, None).unwrap();

        let base_item = CatalogItem {
            id: "bad-item".to_string(),
            name: "Bad Item".to_string(),
            kind: CatalogItemKind::VadModel,
            platform: "all".to_string(),
            version: "1.0".to_string(),
            url: "https://github.com/ggml-org/whisper.cpp/releases/download/v1/test.bin".to_string(),
            sha256: None,
            size_bytes: 128,
            vram_tier: None,
            target_subdir: "vad".to_string(),
            executable_name: None,
            archive_type: ArchiveType::Raw,
            description: "test".to_string(),
            required: false,
        };

        // 1. Missing sha256 rejected
        let manifest_no_sha = CatalogManifest {
            schema_version: 1,
            updated_at: "2026-11-25T00:00:00Z".to_string(),
            items: vec![base_item.clone()],
        };
        let err_sha = install_component_to_paths(
            &manifest_no_sha,
            &paths,
            Path::new(&paths.config_path),
            None,
            "bad-item",
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(
            format!("{err_sha:#}").to_ascii_lowercase().contains("sha-256"),
            "unexpected error: {err_sha:#}"
        );

        // 2. Non-allowlisted host rejected even with valid sha256
        let mut bad_host_item = base_item;
        bad_host_item.sha256 = Some("a".repeat(64));
        bad_host_item.url = "https://untrusted.example.org/payload.bin".to_string();
        let manifest_bad_host = CatalogManifest {
            schema_version: 1,
            updated_at: "2026-11-25T00:00:00Z".to_string(),
            items: vec![bad_host_item],
        };
        let err_host = install_component_to_paths(
            &manifest_bad_host,
            &paths,
            Path::new(&paths.config_path),
            None,
            "bad-item",
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(
            format!("{err_host:#}").to_ascii_lowercase().contains("allowed hosts"),
            "unexpected error: {err_host:#}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archives_reject_symlinks_hardlinks_traversal_and_size_or_count_bombs() {
        let dir = make_temp_dir("archive_sec");

        // 1. ZIP containing a Unix symlink entry must be rejected
        let zip_symlink = dir.join("symlink.zip");
        {
            let file = File::create(&zip_symlink).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .add_symlink("evil_link", "/etc/passwd", SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();
        }
        let zip_out = dir.join("zip_symlink_out");
        let err = extract_zip_archive(&zip_symlink, &zip_out).unwrap_err();
        assert!(
            err.to_string().to_ascii_lowercase().contains("symlink"),
            "Expected symlink error, got: {err}"
        );

        // 1b. ZIP exceeding max file count must be rejected
        let zip_many = dir.join("many.zip");
        {
            let file = File::create(&zip_many).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            for i in 0..5 {
                writer
                    .start_file(format!("f{i}.txt"), SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"hi").unwrap();
            }
            writer.finish().unwrap();
        }
        let zip_many_out = dir.join("zip_many_out");
        let err_many =
            extract_zip_archive_with_limits(&zip_many, &zip_many_out, 3, 1024 * 1024).unwrap_err();
        assert!(
            err_many
                .to_string()
                .to_ascii_lowercase()
                .contains("entry count"),
            "Expected entry count limit error, got: {err_many}"
        );

        // 2. ZIP exceeding max decompressed byte cap must be rejected
        let zip_bomb = dir.join("bomb.zip");
        {
            let file = File::create(&zip_bomb).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("big.bin", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&vec![b'A'; 2048]).unwrap();
            writer.finish().unwrap();
        }
        let zip_bomb_out = dir.join("zip_bomb_out");
        let err_bomb =
            extract_zip_archive_with_limits(&zip_bomb, &zip_bomb_out, 10, 1024).unwrap_err();
        assert!(
            err_bomb
                .to_string()
                .to_ascii_lowercase()
                .contains("decompressed size"),
            "Expected size limit error, got: {err_bomb}"
        );

        // 3. TAR.GZ malicious symlinks (/etc/passwd, ../escape, ../libfoo.so.1, non-.so) must be rejected
        for (idx, (link_name, target)) in [
            ("evil_symlink", "/etc/passwd"),
            ("libfoo.so", "/etc/passwd"),
            ("libfoo.so", "../escape"),
            ("libfoo.so", "../libfoo.so.1"),
            ("evil.txt", "target.txt"),
            ("libfoo.so", "libfoo.so"), // self-loop
        ]
        .into_iter()
        .enumerate()
        {
            let tar_bad = dir.join(format!("bad_symlink_{idx}.tar.gz"));
            {
                let file = File::create(&tar_bad).unwrap();
                let enc = GzEncoder::new(file, Compression::default());
                let mut builder = tar::Builder::new(enc);
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                header.set_cksum();
                builder.append_link(&mut header, link_name, target).unwrap();
                builder.into_inner().unwrap().finish().unwrap();
            }
            let tar_out = dir.join(format!("tar_bad_sym_out_{idx}"));
            let err_tar_sym = extract_tar_gz_archive(&tar_bad, &tar_out).unwrap_err();
            assert!(
                err_tar_sym
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("symlink"),
                "Expected tar symlink error for {link_name} -> {target}, got: {err_tar_sym}"
            );
        }

        // 3b. TAR.GZ benign same-dir ELF SONAME symlinks (deferred after regular files) extract safely
        let tar_soname = dir.join("soname.tar.gz");
        {
            let file = File::create(&tar_soname).unwrap();
            let enc = GzEncoder::new(file, Compression::default());
            let mut builder = tar::Builder::new(enc);

            // Put symlinks BEFORE the regular file in the archive to verify deferred creation
            let mut l1 = tar::Header::new_gnu();
            l1.set_entry_type(tar::EntryType::Symlink);
            l1.set_size(0);
            l1.set_mode(0o777);
            l1.set_cksum();
            builder
                .append_link(&mut l1, "pkg/libfoo.so", "libfoo.so.1")
                .unwrap();

            let mut l2 = tar::Header::new_gnu();
            l2.set_entry_type(tar::EntryType::Symlink);
            l2.set_size(0);
            l2.set_mode(0o777);
            l2.set_cksum();
            builder
                .append_link(&mut l2, "pkg/libfoo.so.1", "libfoo.so.1.2.3")
                .unwrap();

            let payload = b"ELF-shared-library-bytes";
            let mut fh = tar::Header::new_gnu();
            fh.set_size(payload.len() as u64);
            fh.set_mode(0o755);
            fh.set_cksum();
            builder
                .append_data(&mut fh, "pkg/libfoo.so.1.2.3", &payload[..])
                .unwrap();

            builder.into_inner().unwrap().finish().unwrap();
        }
        let tar_soname_out = dir.join("tar_soname_out");
        extract_tar_gz_archive(&tar_soname, &tar_soname_out).unwrap();
        let real_so = tar_soname_out.join("pkg").join("libfoo.so.1.2.3");
        assert_eq!(fs::read(&real_so).unwrap(), b"ELF-shared-library-bytes");
        #[cfg(unix)]
        {
            let link_so = tar_soname_out.join("pkg").join("libfoo.so");
            assert!(fs::symlink_metadata(&link_so).unwrap().file_type().is_symlink());
            assert_eq!(fs::read(&link_so).unwrap(), b"ELF-shared-library-bytes");

            // Symlinks also count toward max_files (3 entries total > max_files = 2 -> rejected)
            let tar_soname_cap = dir.join("tar_soname_cap");
            assert!(
                extract_tar_gz_archive_with_limits(&tar_soname, &tar_soname_cap, 2, 1024 * 1024)
                    .is_err()
            );

            // (a) Re-extracting over an existing install (reinstall or Vulkan <-> CPU switch)
            // cleanly replaces the previous SONAME symlinks and preserves the resolved target.
            extract_tar_gz_archive(&tar_soname, &tar_soname_out).unwrap();
            assert!(fs::symlink_metadata(&link_so).unwrap().file_type().is_symlink());
            assert_eq!(fs::read(&link_so).unwrap(), b"ELF-shared-library-bytes");

            // (b) Extracting a SONAME symlink over an existing regular file or directory on disk
            // must be rejected (never clobber non-symlink paths).
            let tar_file_collision_out = dir.join("tar_file_collision_out");
            fs::create_dir_all(tar_file_collision_out.join("pkg")).unwrap();
            fs::write(
                tar_file_collision_out.join("pkg").join("libfoo.so"),
                b"existing-regular-file",
            )
            .unwrap();
            let err_file_collision =
                extract_tar_gz_archive(&tar_soname, &tar_file_collision_out).unwrap_err();
            assert!(
                err_file_collision
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("non-symlink"),
                "Expected non-symlink collision error for regular file, got: {err_file_collision}"
            );

            let tar_dir_collision_out = dir.join("tar_dir_collision_out");
            fs::create_dir_all(tar_dir_collision_out.join("pkg").join("libfoo.so")).unwrap();
            let err_dir_collision =
                extract_tar_gz_archive(&tar_soname, &tar_dir_collision_out).unwrap_err();
            assert!(
                err_dir_collision
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("non-symlink"),
                "Expected non-symlink collision error for directory, got: {err_dir_collision}"
            );
        }

        // (c) Same-archive collisions (regular file + symlink at same path in either order, or
        // duplicate symlink entries) must be rejected on all platforms.
        for (case_idx, symlink_first) in [false, true].into_iter().enumerate() {
            let tar_same_archive_collide =
                dir.join(format!("soname_same_archive_collide_{case_idx}.tar.gz"));
            {
                let file = File::create(&tar_same_archive_collide).unwrap();
                let enc = GzEncoder::new(file, Compression::default());
                let mut builder = tar::Builder::new(enc);

                let append_sym = |b: &mut tar::Builder<GzEncoder<File>>| {
                    let mut l = tar::Header::new_gnu();
                    l.set_entry_type(tar::EntryType::Symlink);
                    l.set_size(0);
                    l.set_mode(0o777);
                    l.set_cksum();
                    b.append_link(&mut l, "pkg/libfoo.so", "libfoo.so.1")
                        .unwrap();
                };
                let append_file = |b: &mut tar::Builder<GzEncoder<File>>| {
                    let payload = b"colliding-regular-file";
                    let mut fh = tar::Header::new_gnu();
                    fh.set_size(payload.len() as u64);
                    fh.set_mode(0o644);
                    fh.set_cksum();
                    b.append_data(&mut fh, "pkg/libfoo.so", &payload[..])
                        .unwrap();
                };

                if symlink_first {
                    append_sym(&mut builder);
                    append_file(&mut builder);
                } else {
                    append_file(&mut builder);
                    append_sym(&mut builder);
                }
                builder.into_inner().unwrap().finish().unwrap();
            }
            let out_collide = dir.join(format!("tar_same_archive_collide_out_{case_idx}"));
            let err_collide =
                extract_tar_gz_archive(&tar_same_archive_collide, &out_collide).unwrap_err();
            assert!(
                err_collide
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("colliding"),
                "Expected same-archive collision error (symlink_first={symlink_first}), got: {err_collide}"
            );
        }

        let tar_dup_symlink = dir.join("soname_dup_symlink.tar.gz");
        {
            let file = File::create(&tar_dup_symlink).unwrap();
            let enc = GzEncoder::new(file, Compression::default());
            let mut builder = tar::Builder::new(enc);
            for target in ["libfoo.so.1", "libfoo.so.2"] {
                let mut l = tar::Header::new_gnu();
                l.set_entry_type(tar::EntryType::Symlink);
                l.set_size(0);
                l.set_mode(0o777);
                l.set_cksum();
                builder
                    .append_link(&mut l, "pkg/libfoo.so", target)
                    .unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }
        let err_dup = extract_tar_gz_archive(&tar_dup_symlink, &dir.join("tar_dup_sym_out"))
            .unwrap_err();
        assert!(
            err_dup
                .to_string()
                .to_ascii_lowercase()
                .contains("duplicate"),
            "Expected duplicate symlink error, got: {err_dup}"
        );

        // 4. TAR.GZ containing a hardlink must be rejected
        let tar_hardlink = dir.join("hardlink.tar.gz");
        {
            let file = File::create(&tar_hardlink).unwrap();
            let enc = GzEncoder::new(file, Compression::default());
            let mut builder = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Link);
            header.set_size(0);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_link(&mut header, "evil_hardlink", "/etc/passwd")
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let tar_hl_out = dir.join("tar_hardlink_out");
        let err_tar_hl = extract_tar_gz_archive(&tar_hardlink, &tar_hl_out).unwrap_err();
        assert!(
            err_tar_hl
                .to_string()
                .to_ascii_lowercase()
                .contains("hardlink"),
            "Expected tar hardlink error, got: {err_tar_hl}"
        );

        // 5. TAR.GZ exceeding max bytes / file count must be rejected
        let tar_bomb = dir.join("bomb.tar.gz");
        {
            let file = File::create(&tar_bomb).unwrap();
            let enc = GzEncoder::new(file, Compression::default());
            let mut builder = tar::Builder::new(enc);
            let data = vec![b'B'; 2048];
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, "f1.bin", &data[..]).unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let tar_bomb_out = dir.join("tar_bomb_out");
        let err_tar_bomb =
            extract_tar_gz_archive_with_limits(&tar_bomb, &tar_bomb_out, 10, 1024).unwrap_err();
        assert!(
            err_tar_bomb
                .to_string()
                .to_ascii_lowercase()
                .contains("decompressed size"),
            "Expected tar size limit error, got: {err_tar_bomb}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
