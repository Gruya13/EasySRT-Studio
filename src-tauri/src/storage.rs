//! Storage path resolution (Installed vs Portable), config persistence,
//! checkpoint storage, and API key management.

use crate::models::{
    ApiKeyStatus, AppConfig, StorageMode, StoragePaths, SubtitleProject, TranslationProviderKind,
};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub const KEYRING_SERVICE: &str = "com.easysrt.studio";
pub const KEYRING_USER_DEEPSEEK: &str = "deepseek";
pub const KEYRING_USER_CUSTOM_OPENAI: &str = "custom_openai";
pub const PORTABLE_FLAG_FILE: &str = "portable.flag";
pub const CREDENTIALS_FILE_NAME: &str = ".credentials.json";

const OBFUSCATION_KEY: &[u8] = b"EasySRT-Studio-Portable-Credential-Mask-2026";
const OBFUSCATION_PREFIX: &str = "obf1:";

/// Determine the fallback installed root directory when Tauri path state is unavailable.
pub fn default_installed_root_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("EasySRT Studio")
}

/// Core testable path resolution logic given an optional current executable path and the
/// installed app data root directory.
pub fn resolve_storage_paths_with_exe(
    current_exe: Option<&Path>,
    installed_root: &Path,
    models_dir_override: Option<&str>,
) -> Result<StoragePaths> {
    let (mode, root_dir) = match current_exe.and_then(|p| p.parent()) {
        Some(exe_dir) if exe_dir.join(PORTABLE_FLAG_FILE).is_file() => {
            (StorageMode::Portable, exe_dir.to_path_buf())
        }
        _ => (StorageMode::Installed, installed_root.to_path_buf()),
    };

    resolve_storage_paths_from_root(&root_dir, mode, models_dir_override)
}

/// Construct and ensure storage directories from a resolved root directory and storage mode.
pub fn resolve_storage_paths_from_root(
    root_dir: &Path,
    mode: StorageMode,
    models_dir_override: Option<&str>,
) -> Result<StoragePaths> {
    let bin_dir = root_dir.join("bin");
    let data_dir = root_dir.join("data");
    let checkpoints_dir = data_dir.join("checkpoints");
    let config_path = root_dir.join("config.json");

    // Determine effective models_dir_override: use explicit parameter first, or peek config.json if None
    let effective_override: Option<String> = match models_dir_override {
        Some(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(_) => None,
        None => {
            if config_path.is_file() {
                fs::read_to_string(&config_path)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<AppConfig>(&raw).ok())
                    .and_then(|cfg| cfg.models_dir_override)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        }
    };

    let models_dir = match effective_override {
        Some(custom) => PathBuf::from(custom),
        None => root_dir.join("models"),
    };

    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("Failed to create bin directory: {}", bin_dir.display()))?;
    fs::create_dir_all(&models_dir)
        .with_context(|| format!("Failed to create models directory: {}", models_dir.display()))?;
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("Failed to create data directory: {}", data_dir.display()))?;
    fs::create_dir_all(&checkpoints_dir).with_context(|| {
        format!(
            "Failed to create checkpoints directory: {}",
            checkpoints_dir.display()
        )
    })?;

    Ok(StoragePaths {
        mode,
        root_dir: root_dir.to_string_lossy().into_owned(),
        bin_dir: bin_dir.to_string_lossy().into_owned(),
        models_dir: models_dir.to_string_lossy().into_owned(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        config_path: config_path.to_string_lossy().into_owned(),
    })
}

/// Resolve application storage paths (Portable if `portable.flag` exists next to executable,
/// otherwise Installed under `app_local_data_dir`).
pub fn resolve_storage_paths(
    app: &AppHandle,
    models_dir_override: Option<&str>,
) -> Result<StoragePaths> {
    let exe_path = std::env::current_exe().ok();
    let installed_root = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| default_installed_root_dir());

    resolve_storage_paths_with_exe(exe_path.as_deref(), &installed_root, models_dir_override)
}

/// Default `AppConfig` with Serbian Latin (`"sr"`) as the default UI language.
pub fn default_app_config() -> AppConfig {
    AppConfig {
        ui_language: "sr".to_string(),
        ..AppConfig::default()
    }
}

/// Atomic file write helper: writes to a temporary file in the same directory and renames it.
fn atomic_write_bytes(target_path: &Path, bytes: &[u8], unix_mode: Option<u32>) -> Result<()> {
    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow!("Target path has no parent directory: {}", target_path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;

    let file_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp_path = parent.join(format!(".{}.tmp.{}", file_name, std::process::id()));

    fs::write(&tmp_path, bytes)
        .with_context(|| format!("Failed to write temporary file: {}", tmp_path.display()))?;

    #[cfg(unix)]
    if let Some(mode) = unix_mode {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(&tmp_path, perms).with_context(|| {
            format!(
                "Failed to set permissions {:o} on temporary file: {}",
                mode,
                tmp_path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = unix_mode;

    fs::rename(&tmp_path, target_path).with_context(|| {
        format!(
            "Failed to atomically rename {} to {}",
            tmp_path.display(),
            target_path.display()
        )
    })?;

    Ok(())
}

/// Load `AppConfig` from an explicit `config.json` path.
pub fn load_config_from_path(config_path: &Path) -> Result<AppConfig> {
    if !config_path.is_file() {
        return Ok(default_app_config());
    }

    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let mut cfg: AppConfig = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse config JSON: {}", config_path.display()))?;

    if cfg.ui_language.trim().is_empty() {
        cfg.ui_language = "sr".to_string();
    }

    // Normalize and verify models_dir_override if set
    if let Some(ref raw_override) = cfg.models_dir_override {
        let trimmed = raw_override.trim();
        if trimmed.is_empty() {
            cfg.models_dir_override = None;
        } else {
            let override_path = PathBuf::from(trimmed);
            fs::create_dir_all(&override_path).with_context(|| {
                format!(
                    "Failed to ensure models_dir_override directory exists: {}",
                    override_path.display()
                )
            })?;
            cfg.models_dir_override = Some(trimmed.to_string());
        }
    }

    Ok(cfg)
}

/// Save `AppConfig` atomically to an explicit `config.json` path.
pub fn save_config_to_path(config_path: &Path, config: &AppConfig) -> Result<AppConfig> {
    let mut normalized = config.clone();
    if normalized.ui_language.trim().is_empty() {
        normalized.ui_language = "sr".to_string();
    }
    if let Some(ref raw_override) = normalized.models_dir_override {
        let trimmed = raw_override.trim();
        if trimmed.is_empty() {
            normalized.models_dir_override = None;
        } else {
            let override_path = PathBuf::from(trimmed);
            fs::create_dir_all(&override_path).with_context(|| {
                format!(
                    "Failed to create models_dir_override directory: {}",
                    override_path.display()
                )
            })?;
            normalized.models_dir_override = Some(trimmed.to_string());
        }
    }

    let serialized =
        serde_json::to_vec_pretty(&normalized).context("Failed to serialize AppConfig to JSON")?;
    atomic_write_bytes(config_path, &serialized, None)?;
    Ok(normalized)
}

/// Load `AppConfig` from `<root>/config.json`, or return the default config with `ui_language = "sr"`.
pub fn load_config(app: &AppHandle) -> Result<AppConfig> {
    let paths = resolve_storage_paths(app, None)?;
    load_config_from_path(Path::new(&paths.config_path))
}

/// Atomically persist `AppConfig` to `<root>/config.json`.
pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<AppConfig> {
    let paths = resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    save_config_to_path(Path::new(&paths.config_path), config)
}

// ---------------------------------------------------------------------------
// API Key Storage (OS Keyring with Portable / Headless File Fallback)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CredentialsFile {
    #[serde(default)]
    entries: BTreeMap<String, String>,
}

fn provider_user_key(provider: TranslationProviderKind) -> Option<&'static str> {
    match provider {
        TranslationProviderKind::DeepSeek => Some(KEYRING_USER_DEEPSEEK),
        TranslationProviderKind::CustomOpenAi => Some(KEYRING_USER_CUSTOM_OPENAI),
        TranslationProviderKind::LocalLlama => None,
    }
}

fn obfuscate_secret(plain: &str) -> String {
    let masked: Vec<u8> = plain
        .as_bytes()
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ OBFUSCATION_KEY[i % OBFUSCATION_KEY.len()])
        .collect();
    format!("{}{}", OBFUSCATION_PREFIX, hex::encode(masked))
}

fn deobfuscate_secret(stored: &str) -> String {
    if let Some(hex_part) = stored.strip_prefix(OBFUSCATION_PREFIX) {
        if let Ok(bytes) = hex::decode(hex_part) {
            let unmasked: Vec<u8> = bytes
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ OBFUSCATION_KEY[i % OBFUSCATION_KEY.len()])
                .collect();
            if let Ok(s) = String::from_utf8(unmasked) {
                return s;
            }
        }
    }
    stored.to_string()
}

fn credentials_file_path(paths: &StoragePaths) -> PathBuf {
    Path::new(&paths.data_dir).join(CREDENTIALS_FILE_NAME)
}

fn read_credentials_file(path: &Path) -> CredentialsFile {
    if !path.is_file() {
        return CredentialsFile::default();
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<CredentialsFile>(&s).ok())
        .unwrap_or_default()
}

fn write_credentials_file(path: &Path, creds: &CredentialsFile) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(creds).context("Failed to serialize credentials file")?;
    atomic_write_bytes(path, &bytes, Some(0o600))
}

fn try_keyring_get(user: &str) -> std::result::Result<Option<String>, keyring::Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, user)?;
    match entry.get_password() {
        Ok(val) if !val.trim().is_empty() => Ok(Some(val)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err),
    }
}

fn try_keyring_set(user: &str, secret: &str) -> std::result::Result<(), keyring::Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, user)?;
    entry.set_password(secret)?;
    // Verify read-after-write so headless/stub keyrings don't silently drop secrets
    match entry.get_password() {
        Ok(read_back) if read_back == secret => Ok(()),
        Ok(_) => Err(keyring::Error::NoStorageAccess(
            "Keyring read-after-write verification mismatch".into(),
        )),
        Err(err) => Err(err),
    }
}

fn try_keyring_delete(user: &str) -> std::result::Result<(), keyring::Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, user)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err),
    }
}

/// Core testable API key lookup using resolved `StoragePaths`.
pub fn get_api_key_for_paths(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    force_file_fallback: bool,
) -> Result<Option<String>> {
    let Some(user) = provider_user_key(provider) else {
        return Ok(None);
    };

    let cred_path = credentials_file_path(paths);

    if paths.mode != StorageMode::Portable && !force_file_fallback {
        match try_keyring_get(user) {
            Ok(Some(secret)) => return Ok(Some(secret)),
            Ok(None) => {
                // Also check fallback file in case it was previously written there
            }
            Err(_) => {
                // Fall back to file
            }
        }
    }

    let creds = read_credentials_file(&cred_path);
    Ok(creds
        .entries
        .get(user)
        .map(|s| deobfuscate_secret(s))
        .filter(|s| !s.trim().is_empty()))
}

/// Core testable API key status lookup using resolved `StoragePaths`.
pub fn get_api_key_status_for_paths(
    paths: &StoragePaths,
    force_file_fallback: bool,
) -> Result<ApiKeyStatus> {
    let cred_path = credentials_file_path(paths);
    let file_creds = read_credentials_file(&cred_path);

    let mut keyring_available = paths.mode != StorageMode::Portable && !force_file_fallback;
    let mut kr_deepseek = None;
    let mut kr_custom = None;

    if keyring_available {
        match (
            try_keyring_get(KEYRING_USER_DEEPSEEK),
            try_keyring_get(KEYRING_USER_CUSTOM_OPENAI),
        ) {
            (Ok(ds), Ok(cu)) => {
                kr_deepseek = ds;
                kr_custom = cu;
            }
            _ => {
                keyring_available = false;
            }
        }
    }

    let file_deepseek = file_creds
        .entries
        .get(KEYRING_USER_DEEPSEEK)
        .map(|s| deobfuscate_secret(s))
        .filter(|s| !s.trim().is_empty());
    let file_custom = file_creds
        .entries
        .get(KEYRING_USER_CUSTOM_OPENAI)
        .map(|s| deobfuscate_secret(s))
        .filter(|s| !s.trim().is_empty());

    let has_deep_seek_key = kr_deepseek.is_some() || file_deepseek.is_some();
    let has_custom_api_key = kr_custom.is_some() || file_custom.is_some();

    let using_file = !keyring_available
        || (kr_deepseek.is_none() && file_deepseek.is_some())
        || (kr_custom.is_none() && file_custom.is_some());

    let storage_backend = if using_file {
        "portable-file".to_string()
    } else {
        "keyring".to_string()
    };

    Ok(ApiKeyStatus {
        has_deep_seek_key,
        has_custom_api_key,
        storage_backend,
    })
}

/// Core testable API key setter using resolved `StoragePaths`.
pub fn set_api_key_for_paths(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    api_key: &str,
    force_file_fallback: bool,
) -> Result<ApiKeyStatus> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return clear_api_key_for_paths(paths, provider, force_file_fallback);
    }

    let Some(user) = provider_user_key(provider) else {
        return get_api_key_status_for_paths(paths, force_file_fallback);
    };

    let cred_path = credentials_file_path(paths);

    if paths.mode != StorageMode::Portable
        && !force_file_fallback
        && try_keyring_set(user, trimmed).is_ok()
    {
        // If fallback file also had an entry, clean it up so keyring is authoritative
        if cred_path.is_file() {
            let mut creds = read_credentials_file(&cred_path);
            if creds.entries.remove(user).is_some() {
                let _ = write_credentials_file(&cred_path, &creds);
            }
        }
        return get_api_key_status_for_paths(paths, force_file_fallback);
    }

    let mut creds = read_credentials_file(&cred_path);
    creds
        .entries
        .insert(user.to_string(), obfuscate_secret(trimmed));
    write_credentials_file(&cred_path, &creds)?;

    get_api_key_status_for_paths(paths, force_file_fallback)
}

/// Core testable API key clearing helper using resolved `StoragePaths`.
pub fn clear_api_key_for_paths(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    force_file_fallback: bool,
) -> Result<ApiKeyStatus> {
    let Some(user) = provider_user_key(provider) else {
        return get_api_key_status_for_paths(paths, force_file_fallback);
    };

    if paths.mode != StorageMode::Portable && !force_file_fallback {
        let _ = try_keyring_delete(user);
    }

    let cred_path = credentials_file_path(paths);
    if cred_path.is_file() {
        let mut creds = read_credentials_file(&cred_path);
        if creds.entries.remove(user).is_some() {
            write_credentials_file(&cred_path, &creds)?;
        }
    }

    get_api_key_status_for_paths(paths, force_file_fallback)
}

/// Retrieve an API key for `DeepSeek` or `CustomOpenAi` (used by `pipeline::translator`).
pub fn get_api_key(app: &AppHandle, provider: TranslationProviderKind) -> Result<Option<String>> {
    let paths = resolve_storage_paths(app, None)?;
    get_api_key_for_paths(&paths, provider, false)
}

/// Return current API key presence and active backend (`"keyring"` or `"portable-file"`).
pub fn get_api_key_status(app: &AppHandle) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    get_api_key_status_for_paths(&paths, false)
}

/// Save an API key for `DeepSeek` or `CustomOpenAi`.
pub fn set_api_key(
    app: &AppHandle,
    provider: TranslationProviderKind,
    api_key: &str,
) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    set_api_key_for_paths(&paths, provider, api_key, false)
}

/// Clear an API key for `DeepSeek` or `CustomOpenAi`.
pub fn clear_api_key(app: &AppHandle, provider: TranslationProviderKind) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    clear_api_key_for_paths(&paths, provider, false)
}

// ---------------------------------------------------------------------------
// Checkpoint Persistence (<data_dir>/checkpoints/<job_id>.easysrt.json)
// ---------------------------------------------------------------------------

fn sanitize_job_id(job_id: &str) -> Result<String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Checkpoint job_id cannot be empty"));
    }
    let sanitized: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(sanitized)
}

fn checkpoint_file_path(data_dir: &Path, job_id: &str) -> Result<PathBuf> {
    let safe_id = sanitize_job_id(job_id)?;
    Ok(data_dir
        .join("checkpoints")
        .join(format!("{}.easysrt.json", safe_id)))
}

/// Save a `SubtitleProject` checkpoint atomically inside `<data_dir>/checkpoints/<job_id>.easysrt.json`.
pub fn save_checkpoint_to_data_dir(data_dir: &Path, project: &SubtitleProject) -> Result<()> {
    let path = checkpoint_file_path(data_dir, &project.job_id)?;
    let bytes = serde_json::to_vec_pretty(project)
        .with_context(|| format!("Failed to serialize checkpoint for job {}", project.job_id))?;
    atomic_write_bytes(&path, &bytes, None)
}

/// List all saved `SubtitleProject` checkpoints in `<data_dir>/checkpoints/`, sorted by `updated_at` descending.
pub fn list_checkpoints_in_data_dir(data_dir: &Path) -> Result<Vec<SubtitleProject>> {
    let dir = data_dir.join("checkpoints");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut projects = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("Failed to read checkpoints directory: {}", dir.display()))?
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || !name.ends_with(".easysrt.json") {
            continue;
        }

        match fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<SubtitleProject>(&raw).ok())
        {
            Some(project) => projects.push(project),
            None => {
                tracing::warn!("Skipping unreadable checkpoint file: {}", path.display());
            }
        }
    }

    projects.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(projects)
}

/// Load a specific `SubtitleProject` checkpoint by `job_id` from `<data_dir>/checkpoints/`.
pub fn load_checkpoint_from_data_dir(data_dir: &Path, job_id: &str) -> Result<SubtitleProject> {
    let path = checkpoint_file_path(data_dir, job_id)?;
    if !path.is_file() {
        return Err(anyhow!(
            "Checkpoint not found for job '{}': {}",
            job_id,
            path.display()
        ));
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read checkpoint file: {}", path.display()))?;
    let project: SubtitleProject = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse checkpoint JSON: {}", path.display()))?;
    Ok(project)
}

/// Delete a specific `SubtitleProject` checkpoint by `job_id` from `<data_dir>/checkpoints/`.
pub fn delete_checkpoint_from_data_dir(data_dir: &Path, job_id: &str) -> Result<()> {
    let path = checkpoint_file_path(data_dir, job_id)?;
    if path.is_file() {
        fs::remove_file(&path)
            .with_context(|| format!("Failed to delete checkpoint file: {}", path.display()))?;
    }
    Ok(())
}

/// Save a `SubtitleProject` checkpoint (called by `pipeline` after transcription and after each translation batch).
pub fn save_checkpoint(app: &AppHandle, project: &SubtitleProject) -> Result<()> {
    let paths = resolve_storage_paths(app, None)?;
    save_checkpoint_to_data_dir(Path::new(&paths.data_dir), project)
}

/// List all saved checkpoints sorted by `updated_at` descending.
pub fn list_checkpoints(app: &AppHandle) -> Result<Vec<SubtitleProject>> {
    let paths = resolve_storage_paths(app, None)?;
    list_checkpoints_in_data_dir(Path::new(&paths.data_dir))
}

/// Load a checkpoint by `job_id`.
pub fn load_checkpoint(app: &AppHandle, job_id: &str) -> Result<SubtitleProject> {
    let paths = resolve_storage_paths(app, None)?;
    load_checkpoint_from_data_dir(Path::new(&paths.data_dir), job_id)
}

/// Delete a checkpoint by `job_id`.
pub fn delete_checkpoint(app: &AppHandle, job_id: &str) -> Result<()> {
    let paths = resolve_storage_paths(app, None)?;
    delete_checkpoint_from_data_dir(Path::new(&paths.data_dir), job_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{JobOptions, JobPhase, SerbianScript, SubtitleCue, TimedWord};
    use uuid::Uuid;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("easysrt_test_{}_{}", prefix, Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn portable_vs_installed_path_resolution() {
        let base = make_temp_dir("paths");
        let exe_dir = base.join("exe_folder");
        let installed_dir = base.join("installed_data");
        fs::create_dir_all(&exe_dir).unwrap();
        let fake_exe = exe_dir.join("easysrt-studio.exe");
        fs::write(&fake_exe, b"binary").unwrap();

        // 1. Without portable.flag -> Installed mode
        let paths_installed =
            resolve_storage_paths_with_exe(Some(&fake_exe), &installed_dir, None).unwrap();
        assert_eq!(paths_installed.mode, StorageMode::Installed);
        assert_eq!(
            Path::new(&paths_installed.root_dir),
            installed_dir.as_path()
        );
        assert!(Path::new(&paths_installed.bin_dir).is_dir());
        assert!(Path::new(&paths_installed.models_dir).is_dir());
        assert!(Path::new(&paths_installed.data_dir).is_dir());
        assert!(Path::new(&paths_installed.data_dir)
            .join("checkpoints")
            .is_dir());

        // 2. With portable.flag -> Portable mode
        fs::write(exe_dir.join(PORTABLE_FLAG_FILE), b"").unwrap();
        let paths_portable =
            resolve_storage_paths_with_exe(Some(&fake_exe), &installed_dir, None).unwrap();
        assert_eq!(paths_portable.mode, StorageMode::Portable);
        assert_eq!(Path::new(&paths_portable.root_dir), exe_dir.as_path());
        assert_eq!(
            Path::new(&paths_portable.models_dir),
            exe_dir.join("models").as_path()
        );

        // 3. With models_dir_override -> custom models dir created and used
        let custom_models = base.join("custom_models_location");
        let paths_override = resolve_storage_paths_with_exe(
            Some(&fake_exe),
            &installed_dir,
            Some(custom_models.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            Path::new(&paths_override.models_dir),
            custom_models.as_path()
        );
        assert!(custom_models.is_dir());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn config_default_is_serbian_and_roundtrips_cleanly() {
        let base = make_temp_dir("config");
        let config_path = base.join("config.json");

        // Missing file -> returns default with "sr" UI language
        let initial = load_config_from_path(&config_path).unwrap();
        assert_eq!(initial.ui_language, "sr");
        assert_eq!(initial.default_script, SerbianScript::Latin);
        assert!(!initial.has_completed_setup);

        // Modify and save atomically
        let custom_models_dir = base.join("my_models");
        let mut updated = initial.clone();
        updated.ui_language = "en".to_string();
        updated.active_whisper_model_id = Some("whisper-large-v3-turbo-q5_0".to_string());
        updated.active_llm_model_id = Some("llm-gemma3-4b-q4km".to_string());
        updated.models_dir_override = Some(custom_models_dir.to_string_lossy().into_owned());
        updated.has_completed_setup = true;

        let saved = save_config_to_path(&config_path, &updated).unwrap();
        assert!(config_path.is_file());
        assert!(custom_models_dir.is_dir());
        assert_eq!(saved, updated);

        let reloaded = load_config_from_path(&config_path).unwrap();
        assert_eq!(reloaded, updated);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn checkpoint_crud_and_descending_sort() {
        let base = make_temp_dir("checkpoints");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Portable, None).unwrap();
        let data_dir = Path::new(&paths.data_dir);

        let proj_older = SubtitleProject {
            job_id: "job-100".to_string(),
            audio_path: "/tmp/a.mp3".to_string(),
            audio_file_name: "a.mp3".to_string(),
            duration_ms: 15000,
            detected_language: Some("en".to_string()),
            options: JobOptions::default(),
            words: vec![TimedWord {
                start_ms: 0,
                end_ms: 500,
                text: "Hello".to_string(),
                probability: 0.99,
            }],
            cues: vec![SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1500,
                source_text: "Hello world".to_string(),
                translated_text: "Zdravo svete".to_string(),
                cps: 8.0,
                warnings: vec![],
            }],
            phase: JobPhase::Completed,
            completed_translation_batches: 1,
            error_message: None,
            updated_at: "2026-11-22T10:00:00Z".to_string(),
        };

        let mut proj_newer = proj_older.clone();
        proj_newer.job_id = "job-200".to_string();
        proj_newer.updated_at = "2026-11-22T12:00:00Z".to_string();

        save_checkpoint_to_data_dir(data_dir, &proj_older).unwrap();
        save_checkpoint_to_data_dir(data_dir, &proj_newer).unwrap();

        let listed = list_checkpoints_in_data_dir(data_dir).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].job_id, "job-200");
        assert_eq!(listed[1].job_id, "job-100");

        let loaded = load_checkpoint_from_data_dir(data_dir, "job-100").unwrap();
        assert_eq!(loaded, proj_older);

        delete_checkpoint_from_data_dir(data_dir, "job-100").unwrap();
        let listed_after = list_checkpoints_in_data_dir(data_dir).unwrap();
        assert_eq!(listed_after.len(), 1);
        assert_eq!(listed_after[0].job_id, "job-200");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn credential_file_fallback_and_portable_mode() {
        let base = make_temp_dir("creds");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Portable, None).unwrap();

        let initial_status = get_api_key_status_for_paths(&paths, false).unwrap();
        assert!(!initial_status.has_deep_seek_key);
        assert!(!initial_status.has_custom_api_key);
        assert_eq!(initial_status.storage_backend, "portable-file");

        // Save DeepSeek key
        let after_ds = set_api_key_for_paths(
            &paths,
            TranslationProviderKind::DeepSeek,
            "sk-deepseek-test-12345",
            false,
        )
        .unwrap();
        assert!(after_ds.has_deep_seek_key);
        assert!(!after_ds.has_custom_api_key);
        assert_eq!(after_ds.storage_backend, "portable-file");

        // Verify obfuscated storage on disk (raw secret must not appear in plaintext)
        let cred_file = credentials_file_path(&paths);
        let raw_on_disk = fs::read_to_string(&cred_file).unwrap();
        assert!(!raw_on_disk.contains("sk-deepseek-test-12345"));

        // Verify retrieval de-obfuscates accurately
        let fetched =
            get_api_key_for_paths(&paths, TranslationProviderKind::DeepSeek, false).unwrap();
        assert_eq!(fetched.as_deref(), Some("sk-deepseek-test-12345"));

        // Save CustomOpenAi key
        let after_custom = set_api_key_for_paths(
            &paths,
            TranslationProviderKind::CustomOpenAi,
            "sk-custom-999",
            false,
        )
        .unwrap();
        assert!(after_custom.has_deep_seek_key);
        assert!(after_custom.has_custom_api_key);

        // Clear DeepSeek key
        let after_clear =
            clear_api_key_for_paths(&paths, TranslationProviderKind::DeepSeek, false).unwrap();
        assert!(!after_clear.has_deep_seek_key);
        assert!(after_clear.has_custom_api_key);
        assert_eq!(
            get_api_key_for_paths(&paths, TranslationProviderKind::DeepSeek, false).unwrap(),
            None
        );

        let _ = fs::remove_dir_all(&base);
    }
}
