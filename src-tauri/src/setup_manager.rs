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
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const EMBEDDED_CATALOG_JSON: &str = include_str!("../resources/catalog.json");

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

/// Load the component catalog, checking `<resource_dir>/resources/catalog.json` first if overridden
/// on disk, and falling back to the compile-time embedded `catalog.json`.
pub fn load_catalog(app: &AppHandle) -> Result<CatalogManifest> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        for candidate in [
            resource_dir.join("resources").join("catalog.json"),
            resource_dir.join("catalog.json"),
        ] {
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
    }

    load_embedded_catalog()
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

/// Select the recommended bundle item IDs for a given GPU profile and platform (`"windows-x64"` or `"linux-x64"`).
pub fn select_recommended_bundle_ids(gpu: &GpuInfo, platform: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let use_gpu_binaries = gpu.has_nvidia && gpu.recommended_tier != VramTier::CpuOnly;

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

    let whisper_model = match gpu.recommended_tier {
        VramTier::Vram12GbPlus => "whisper-large-v3-turbo",
        VramTier::Vram8Gb | VramTier::Vram4Gb => "whisper-large-v3-turbo-q5_0",
        VramTier::CpuOnly => "whisper-medium-q5_0",
    };
    ids.push(whisper_model.to_string());

    let llm_model = match gpu.recommended_tier {
        VramTier::Vram12GbPlus => "llm-gemma4-12b-qat",
        VramTier::Vram8Gb => "llm-gemma4-e4b-qat",
        VramTier::Vram4Gb | VramTier::CpuOnly => "llm-gemma3-4b-q4km",
    };
    ids.push(llm_model.to_string());

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

    let recommended = select_recommended_bundle_ids(&gpu, platform);
    let mut missing_required = Vec::new();

    for rec_id in &recommended {
        let Some(item) = catalog.items.iter().find(|i| i.id == *rec_id) else {
            continue;
        };
        let satisfied = match item.kind {
            CatalogItemKind::Ffmpeg => {
                has_kind_for_platform(CatalogItemKind::Ffmpeg)
                    || find_in_system_path("ffmpeg").is_some()
            }
            CatalogItemKind::WhisperBin => {
                has_kind_for_platform(CatalogItemKind::WhisperBin)
                    || find_in_system_path("whisper-cli").is_some()
            }
            CatalogItemKind::CudaRuntime => {
                // Only required if llama-win-cuda is used or no CPU llama is installed
                is_installed("cudart-win-cuda12.4") || is_installed("llama-win-cpu")
            }
            CatalogItemKind::LlamaBin => {
                has_kind_for_platform(CatalogItemKind::LlamaBin)
                    || find_in_system_path("llama-server").is_some()
            }
            CatalogItemKind::VadModel => has_kind_for_platform(CatalogItemKind::VadModel),
            CatalogItemKind::WhisperModel => has_kind_for_platform(CatalogItemKind::WhisperModel),
            CatalogItemKind::LlmModel => has_kind_for_platform(CatalogItemKind::LlmModel),
        };

        if !satisfied {
            missing_required.push(rec_id.clone());
        }
    }

    let ready_for_offline = missing_required.is_empty();

    SetupStatus {
        gpu,
        storage,
        components,
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

/// Verify a file's SHA-256 matches `expected_hex` (case-insensitive).
pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<()> {
    let expected = expected_hex.trim().to_ascii_lowercase();
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

/// Extract a `.zip` archive safely into `target_dir` (guarding against zip-slip).
pub fn extract_zip_archive(archive_path: &Path, target_dir: &Path) -> Result<()> {
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

    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .with_context(|| format!("Failed to read zip entry #{}", idx))?;
        let Some(rel_path) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            return Err(anyhow!(
                "Zip archive contains unsafe path traversal entry: {}",
                entry.name()
            ));
        };

        let out_path = target_dir.join(&rel_path);
        if entry.is_dir() || entry.name().ends_with('/') {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path).with_context(|| {
                format!("Failed to create extracted file: {}", out_path.display())
            })?;
            std::io::copy(&mut entry, &mut out_file).with_context(|| {
                format!("Failed to extract zip entry to {}", out_path.display())
            })?;
            out_file.flush()?;

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

/// Extract a `.tar.gz` archive safely into `target_dir` (preserving internal symlinks and permissions).
pub fn extract_tar_gz_archive(archive_path: &Path, target_dir: &Path) -> Result<()> {
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

    for entry_res in archive
        .entries()
        .context("Failed to read entries from tar.gz archive")?
    {
        let mut entry = entry_res.context("Corrupt entry in tar.gz archive")?;
        let unpacked = entry.unpack_in(target_dir).with_context(|| {
            format!(
                "Failed unpacking tar.gz entry into {}",
                target_dir.display()
            )
        })?;
        if !unpacked {
            return Err(anyhow!(
                "Tar.gz archive contains unsafe path traversal entry"
            ));
        }
    }

    Ok(())
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

fn emit_download_progress(
    app: &AppHandle,
    item_id: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
    stage: &str,
    message: String,
) {
    let percent = if total_bytes > 0 {
        ((downloaded_bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0)
    } else if stage == "completed" {
        100.0
    } else {
        0.0
    };

    let _ = app.emit(
        EVENT_DOWNLOAD_PROGRESS,
        DownloadProgressEvent {
            item_id: item_id.to_string(),
            downloaded_bytes,
            total_bytes,
            percent,
            stage: stage.to_string(),
            message,
        },
    );
}

/// Download, checksum-verify, and extract/install a single catalog component by `item_id`.
pub async fn install_component(
    app: &AppHandle,
    item_id: &str,
) -> Result<InstalledComponentStatus> {
    let mut config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;

    let item = catalog
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| anyhow!("Catalog item '{}' not found", item_id))?
        .clone();

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

    emit_download_progress(
        app,
        &item.id,
        existing_len,
        item.size_bytes,
        "downloading",
        format!("Preuzimanje {}...", item.name),
    );

    if item.size_bytes == 0 || existing_len < item.size_bytes {
        let client = reqwest::Client::builder()
            .user_agent("EasySRT-Studio/0.1.0")
            .connect_timeout(Duration::from_secs(30))
            .build()
            .context("Failed to initialize HTTP download client")?;

        let mut req = client.get(&item.url);
        if existing_len > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", existing_len));
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}", item.url))?;

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
                    item.url,
                    status
                ));
            };

        let total_bytes = if item.size_bytes > 0 {
            item.size_bytes
        } else {
            resp.content_length()
                .map(|cl| cl + downloaded_bytes)
                .unwrap_or(0)
        };

        let mut stream = resp.bytes_stream();
        let mut last_emit = Instant::now();

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res
                .with_context(|| format!("Network stream interrupted for {}", item.id))?;
            part_file
                .write_all(&chunk)
                .with_context(|| format!("Disk write error for {}", part_path.display()))?;
            downloaded_bytes += chunk.len() as u64;

            if last_emit.elapsed() >= Duration::from_millis(100) {
                emit_download_progress(
                    app,
                    &item.id,
                    downloaded_bytes,
                    total_bytes,
                    "downloading",
                    format!("Preuzimanje {}...", item.name),
                );
                last_emit = Instant::now();
            }
        }
        part_file.flush()?;
    }

    // Verify SHA-256 checksum
    if let Some(ref expected_sha) = item.sha256 {
        if !expected_sha.trim().is_empty() {
            emit_download_progress(
                app,
                &item.id,
                item.size_bytes,
                item.size_bytes,
                "verifying",
                format!("Provera SHA-256 potpisa za {}...", item.name),
            );
            if let Err(err) = verify_file_sha256(&part_path, expected_sha) {
                let _ = fs::remove_file(&part_path);
                return Err(err);
            }
        }
    }

    // Extract or move into target location
    emit_download_progress(
        app,
        &item.id,
        item.size_bytes,
        item.size_bytes,
        "extracting",
        format!("Instalacija {}...", item.name),
    );

    if is_binary_kind(item.kind) {
        let target_subdir = Path::new(&paths.bin_dir).join(&item.target_subdir);
        fs::create_dir_all(&target_subdir)?;

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
                let dest_path = expected_item_path(&item, &paths);
                move_file_across_fs(&part_path, &dest_path)?;
                set_executable_permission(&dest_path)?;
            }
        }

        if let Some(ref rel_exe) = item.executable_name {
            let exe_path = target_subdir.join(rel_exe);
            let _ = set_executable_permission(&exe_path);
        }

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
        let dest_path = expected_item_path(&item, &paths);
        move_file_across_fs(&part_path, &dest_path)?;

        // Auto-select newly installed model if none is currently active
        let mut config_changed = false;
        if item.kind == CatalogItemKind::WhisperModel && config.active_whisper_model_id.is_none() {
            config.active_whisper_model_id = Some(item.id.clone());
            config_changed = true;
        } else if item.kind == CatalogItemKind::LlmModel && config.active_llm_model_id.is_none() {
            config.active_llm_model_id = Some(item.id.clone());
            config_changed = true;
        }
        if config_changed {
            let _ = storage::save_config(app, &config);
        }
    }

    let status = check_component_installed(&item, &catalog.items, &paths);
    if !status.installed {
        return Err(anyhow!(
            "Component '{}' failed post-install verification at expected path {}",
            item.id,
            expected_item_path(&item, &paths).display()
        ));
    }

    emit_download_progress(
        app,
        &item.id,
        item.size_bytes,
        item.size_bytes,
        "completed",
        format!("{} je uspešno instaliran.", item.name),
    );

    Ok(status)
}

/// Install the complete recommended offline bundle for the detected GPU tier and platform,
/// update active model selections in `AppConfig`, and return the resulting `SetupStatus`.
pub async fn install_recommended_bundle(app: &AppHandle) -> Result<SetupStatus> {
    let gpu = hardware::detect_gpu();
    let platform = current_platform_tag();
    let bundle_ids = select_recommended_bundle_ids(&gpu, platform);

    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;

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
            install_component(app, item_id).await?;
        }
    }

    let mut updated_config = storage::load_config(app)?;
    if let Some(w_id) = installed_whisper_id {
        updated_config.active_whisper_model_id = Some(w_id);
    }
    if let Some(l_id) = installed_llm_id {
        updated_config.active_llm_model_id = Some(l_id);
    }
    updated_config.has_completed_setup = true;
    storage::save_config(app, &updated_config)?;

    get_setup_status(app)
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
    let mut config = storage::load_config(app)?;
    let entry = import_custom_model_with_config(&mut config, source_path, kind, name)?;
    storage::save_config(app, &config)?;
    Ok(entry)
}

/// Delete a downloaded catalog model/component or remove an imported custom model entry.
pub fn delete_model(app: &AppHandle, model_id: &str) -> Result<()> {
    let mut config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    let catalog = load_catalog(app)?;

    // Remove from custom_models if present
    let before_len = config.custom_models.len();
    config.custom_models.retain(|m| m.id != model_id);
    let removed_custom = config.custom_models.len() != before_len;

    // Also remove downloaded catalog file if it matches a catalog item
    if let Some(item) = catalog.items.iter().find(|i| i.id == model_id) {
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
        return Err(anyhow!("Model '{}' not found in catalog or custom models", model_id));
    }

    if config.active_whisper_model_id.as_deref() == Some(model_id) {
        config.active_whisper_model_id = None;
    }
    if config.active_llm_model_id.as_deref() == Some(model_id) {
        config.active_llm_model_id = None;
    }

    storage::save_config(app, &config)?;
    Ok(())
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
        };
        let win_8gb = select_recommended_bundle_ids(&gpu_8gb, "windows-x64");
        assert!(win_8gb.contains(&"whisper-large-v3-turbo-q5_0".to_string()));
        assert!(win_8gb.contains(&"llm-gemma4-e4b-qat".to_string()));

        // 3. Linux 12GB+ NVIDIA (e.g. RTX 4070 Ti / 3090)
        let gpu_12gb = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 3090".to_string()),
            vram_mb: 24576,
            recommended_tier: VramTier::Vram12GbPlus,
        };
        let linux_12gb = select_recommended_bundle_ids(&gpu_12gb, "linux-x64");
        assert_eq!(
            linux_12gb,
            vec![
                "ffmpeg-linux-x64",
                "whisper-linux-x64",
                "llama-linux-vulkan",
                "vad-silero-v6.2.0",
                "whisper-large-v3-turbo",
                "llm-gemma4-12b-qat",
            ]
        );

        // 4. Windows CPU-only fallback
        let cpu_only = GpuInfo {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: 0,
            recommended_tier: VramTier::CpuOnly,
        };
        let win_cpu = select_recommended_bundle_ids(&cpu_only, "windows-x64");
        assert_eq!(
            win_cpu,
            vec![
                "ffmpeg-win-x64",
                "whisper-win-cpu",
                "llama-win-cpu",
                "vad-silero-v6.2.0",
                "whisper-medium-q5_0",
                "llm-gemma3-4b-q4km",
            ]
        );
    }

    #[test]
    fn custom_model_import_and_active_model_resolution() {
        let dir = make_temp_dir("custom_model");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Portable, None).unwrap();
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
}
