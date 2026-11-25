//! Storage path resolution (`%LOCALAPPDATA%\com.easysrt.studio`), config persistence,
//! checkpoint storage, and API key management.

use crate::models::{
    ApiKeyStatus, AppConfig, StorageMode, StoragePaths, SubtitleProject, TranslationProviderKind,
};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(any(not(windows), test))]
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use tauri::{AppHandle, Manager};

pub const KEYRING_SERVICE: &str = "com.easysrt.studio";
pub const KEYRING_USER_DEEPSEEK: &str = "deepseek";
pub const KEYRING_USER_CUSTOM_OPENAI: &str = "custom_openai";
pub const CREDENTIALS_FILE_NAME: &str = ".credentials.json";
pub const VAULT_KEY_FILE_NAME: &str = ".vault.key";

#[cfg(not(windows))]
const VAULT_ENVELOPE_PREFIX: &str = "v2:";
#[cfg(windows)]
const DPAPI_ENVELOPE_PREFIX: &str = "dpapi1:";

static TEMP_FILE_SEQ: AtomicU64 = AtomicU64::new(0);
static CONFIG_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CREDENTIALS_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static SESSION_VAULT: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn lock_config_io() -> MutexGuard<'static, ()> {
    CONFIG_IO_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_credentials_io() -> MutexGuard<'static, ()> {
    CREDENTIALS_IO_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_session_vault() -> MutexGuard<'static, HashMap<String, String>> {
    SESSION_VAULT
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn session_vault_key(paths: &StoragePaths, user: &str) -> String {
    format!("{}\0{}", paths.data_dir, user)
}

/// Determine the canonical installed root directory matching Tauri's `app_local_data_dir()`
/// (`%LOCALAPPDATA%\com.easysrt.studio` on Windows, `~/.local/share/com.easysrt.studio` on Linux).
pub fn default_installed_root_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.easysrt.studio")
}

/// Core testable path resolution logic using `installed_root` and an optional models override.
pub fn resolve_storage_paths_with_exe(
    _current_exe: Option<&Path>,
    installed_root: &Path,
    models_dir_override: Option<&str>,
) -> Result<StoragePaths> {
    resolve_storage_paths_from_root(installed_root, StorageMode::Installed, models_dir_override)
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

/// Resolve application storage paths under `app_local_data_dir` (canonical `Installed` mode).
pub fn resolve_storage_paths(
    app: &AppHandle,
    models_dir_override: Option<&str>,
) -> Result<StoragePaths> {
    let installed_root = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| default_installed_root_dir());

    resolve_storage_paths_from_root(&installed_root, StorageMode::Installed, models_dir_override)
}

/// Default `AppConfig` with English (`"en"`) as the default UI language.
pub fn default_app_config() -> AppConfig {
    AppConfig::default()
}

/// Race-free atomic file write helper with optional `0600` creation mode on Unix (zero TOCTOU).
///
/// Writes to a uniquely numbered temporary file `.{file_name}.tmp.{pid}.{seq}` in the target
/// directory, flushes/syncs it to disk, and atomically renames it into place. On any error, the
/// temporary file is cleaned up before returning.
pub fn atomic_write_bytes_with_mode(
    target_path: &Path,
    bytes: &[u8],
    sensitive: bool,
) -> Result<()> {
    let parent = target_path.parent().ok_or_else(|| {
        anyhow!(
            "Target path has no parent directory: {}",
            target_path.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;

    let file_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let seq = TEMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(
        ".{}.tmp.{}.{}",
        file_name,
        std::process::id(),
        seq
    ));

    let mut open_opts = OpenOptions::new();
    open_opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open_opts.mode(if sensitive { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = sensitive;

    let mut file = open_opts
        .open(&tmp_path)
        .with_context(|| format!("Failed to create temporary file: {}", tmp_path.display()))?;

    if let Err(err) = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&tmp_path);
        return Err(err)
            .with_context(|| format!("Failed to write temporary file: {}", tmp_path.display()));
    }
    drop(file);

    if let Err(err) = fs::rename(&tmp_path, target_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err).with_context(|| {
            format!(
                "Failed to atomically rename {} to {}",
                tmp_path.display(),
                target_path.display()
            )
        });
    }

    Ok(())
}

/// Race-free atomic file write helper with standard non-sensitive permissions.
pub fn atomic_write_bytes(target_path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_bytes_with_mode(target_path, bytes, false)
}

fn load_config_from_path_unlocked(config_path: &Path) -> Result<AppConfig> {
    if !config_path.is_file() {
        return Ok(default_app_config());
    }

    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let mut cfg: AppConfig = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse config JSON: {}", config_path.display()))?;

    if cfg.ui_language.trim().is_empty() {
        cfg.ui_language = "en".to_string();
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

fn save_config_to_path_unlocked(config_path: &Path, config: &AppConfig) -> Result<AppConfig> {
    let mut normalized = config.clone();
    if normalized.ui_language.trim().is_empty() {
        normalized.ui_language = "en".to_string();
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
    atomic_write_bytes(config_path, &serialized)?;
    Ok(normalized)
}

/// Load `AppConfig` from an explicit `config.json` path.
pub fn load_config_from_path(config_path: &Path) -> Result<AppConfig> {
    let _guard = lock_config_io();
    load_config_from_path_unlocked(config_path)
}

/// Save `AppConfig` atomically to an explicit `config.json` path.
pub fn save_config_to_path(config_path: &Path, config: &AppConfig) -> Result<AppConfig> {
    let _guard = lock_config_io();
    save_config_to_path_unlocked(config_path, config)
}

/// Perform a serialized Read-Modify-Write update on an explicit `config.json` path.
pub fn update_config_at_path<F>(config_path: &Path, f: F) -> Result<AppConfig>
where
    F: FnOnce(&mut AppConfig),
{
    let _guard = lock_config_io();
    let mut cfg = load_config_from_path_unlocked(config_path)?;
    f(&mut cfg);
    save_config_to_path_unlocked(config_path, &cfg)
}

/// Perform a serialized Read-Modify-Write update on `config.json` for resolved `StoragePaths`.
pub fn update_config_for_paths<F>(paths: &StoragePaths, f: F) -> Result<AppConfig>
where
    F: FnOnce(&mut AppConfig),
{
    update_config_at_path(Path::new(&paths.config_path), f)
}

/// Perform a serialized Read-Modify-Write update on `<root>/config.json`.
pub fn update_config<F>(app: &AppHandle, f: F) -> Result<AppConfig>
where
    F: FnOnce(&mut AppConfig),
{
    let paths = resolve_storage_paths(app, None)?;
    update_config_for_paths(&paths, f)
}

/// Perform a fallible serialized Read-Modify-Write update on `<root>/config.json`.
pub fn try_update_config<F, T>(app: &AppHandle, f: F) -> Result<T>
where
    F: FnOnce(&mut AppConfig) -> Result<T>,
{
    let paths = resolve_storage_paths(app, None)?;
    let _guard = lock_config_io();
    let config_path = Path::new(&paths.config_path);
    let mut cfg = load_config_from_path_unlocked(config_path)?;
    let outcome = f(&mut cfg)?;
    save_config_to_path_unlocked(config_path, &cfg)?;
    Ok(outcome)
}

/// Load `AppConfig` from `<root>/config.json`, or return the default config with `ui_language = "en"`.
pub fn load_config(app: &AppHandle) -> Result<AppConfig> {
    let paths = resolve_storage_paths(app, None)?;
    load_config_from_path(Path::new(&paths.config_path))
}

/// Atomically persist `AppConfig` to `<root>/config.json` for resolved `StoragePaths`,
/// migrating RAM session API keys to the local encrypted vault or removing disk vault files
/// whenever `allow_local_encrypted_vault` is toggled.
pub fn save_config_for_paths(paths: &StoragePaths, config: &AppConfig) -> Result<AppConfig> {
    let config_path = Path::new(&paths.config_path);
    let prev_allow = load_config_from_path(config_path)
        .map(|c| c.allow_local_encrypted_vault)
        .unwrap_or(false);
    let saved = save_config_to_path(config_path, config)?;
    if prev_allow != saved.allow_local_encrypted_vault {
        sync_local_vault_opt_in_for_paths(paths, saved.allow_local_encrypted_vault)?;
    }
    Ok(saved)
}

/// Atomically persist `AppConfig` to `<root>/config.json` and sync local vault opt-in transitions.
pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<AppConfig> {
    let paths = resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    save_config_for_paths(&paths, config)
}

// ---------------------------------------------------------------------------
// API Key Storage Hierarchy (OS Keyring -> RAM SessionVault -> Opt-in Local Vault)
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

fn credentials_file_path(paths: &StoragePaths) -> PathBuf {
    Path::new(&paths.data_dir).join(CREDENTIALS_FILE_NAME)
}

#[cfg(not(windows))]
fn vault_key_file_path(paths: &StoragePaths) -> PathBuf {
    Path::new(&paths.data_dir).join(VAULT_KEY_FILE_NAME)
}

/// RFC 2104 HMAC-SHA256 used for Encrypt-then-MAC authentication of opt-in local vault entries.
#[cfg(any(not(windows), test))]
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = Sha256::digest(key);
        key_block[..32].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    let out = outer.finalize();

    let mut result = [0u8; 32];
    result.copy_from_slice(&out);
    result
}

#[cfg(not(windows))]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(not(windows))]
fn fill_csprng_bytes(buf: &mut [u8]) -> Result<()> {
    use std::io::Read;
    let mut urandom = fs::File::open("/dev/urandom")
        .context("Failed to open /dev/urandom for CSPRNG key generation")?;
    urandom
        .read_exact(buf)
        .context("Failed to read random bytes from /dev/urandom")?;
    Ok(())
}

#[cfg(not(windows))]
fn load_or_create_vault_key(paths: &StoragePaths) -> Result<[u8; 32]> {
    let key_path = vault_key_file_path(paths);
    if key_path.is_file() {
        let raw = fs::read_to_string(&key_path)
            .with_context(|| format!("Failed to read vault key file: {}", key_path.display()))?;
        let decoded = hex::decode(raw.trim())
            .with_context(|| format!("Corrupted vault key file: {}", key_path.display()))?;
        if decoded.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            return Ok(key);
        }
        return Err(anyhow!(
            "Invalid vault key length ({} bytes) in {}",
            decoded.len(),
            key_path.display()
        ));
    }

    let mut key = [0u8; 32];
    fill_csprng_bytes(&mut key)?;
    let hex_key = hex::encode(key);
    atomic_write_bytes_with_mode(&key_path, hex_key.as_bytes(), true)?;
    Ok(key)
}

#[cfg(not(windows))]
fn load_existing_vault_key(paths: &StoragePaths) -> Option<[u8; 32]> {
    let key_path = vault_key_file_path(paths);
    if !key_path.is_file() {
        return None;
    }
    let raw = fs::read_to_string(&key_path).ok()?;
    let decoded = hex::decode(raw.trim()).ok()?;
    if decoded.len() != 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded);
    Some(key)
}

#[cfg(not(windows))]
fn derive_subkeys(master_key: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut enc_hasher = Sha256::new();
    enc_hasher.update(b"easysrt-vault-v2-enc:");
    enc_hasher.update(master_key);
    let enc_digest = enc_hasher.finalize();

    let mut mac_hasher = Sha256::new();
    mac_hasher.update(b"easysrt-vault-v2-mac:");
    mac_hasher.update(master_key);
    let mac_digest = mac_hasher.finalize();

    let mut enc_key = [0u8; 32];
    let mut mac_key = [0u8; 32];
    enc_key.copy_from_slice(&enc_digest);
    mac_key.copy_from_slice(&mac_digest);
    (enc_key, mac_key)
}

#[cfg(not(windows))]
fn sha256_ctr_xor(enc_key: &[u8; 32], nonce: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (block_idx, chunk) in data.chunks(32).enumerate() {
        let mut h = Sha256::new();
        h.update(enc_key);
        h.update(nonce);
        h.update((block_idx as u32).to_be_bytes());
        let stream = h.finalize();
        for (b, ks) in chunk.iter().zip(stream.iter()) {
            out.push(b ^ ks);
        }
    }
    out
}

#[cfg(not(windows))]
fn compute_vault_mac(
    mac_key: &[u8; 32],
    provider_user: &str,
    nonce: &[u8; 16],
    ciphertext: &[u8],
) -> [u8; 32] {
    let mut mac_input = Vec::with_capacity(3 + provider_user.len() + 1 + nonce.len() + ciphertext.len());
    mac_input.extend_from_slice(b"v2:");
    mac_input.extend_from_slice(provider_user.as_bytes());
    mac_input.push(b':');
    mac_input.extend_from_slice(nonce);
    mac_input.extend_from_slice(ciphertext);
    hmac_sha256(mac_key, &mac_input)
}

#[cfg(windows)]
mod dpapi {
    use anyhow::{anyhow, Result};
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    }

    pub fn protect(plaintext: &[u8], provider_user: &str) -> Result<Vec<u8>> {
        let mut input = plaintext.to_vec();
        let in_blob = DataBlob {
            cb_data: u32::try_from(input.len()).map_err(|_| anyhow!("Secret exceeds DPAPI limit"))?,
            pb_data: input.as_mut_ptr(),
        };
        let mut entropy = format!("com.easysrt.studio.vault.v2:{provider_user}").into_bytes();
        let entropy_blob = DataBlob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_mut_ptr(),
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };

        // SAFETY: Pointers reference live buffers for the duration of the synchronous call.
        let ok = unsafe {
            CryptProtectData(
                &in_blob,
                ptr::null(),
                &entropy_blob,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 || out_blob.pb_data.is_null() {
            return Err(anyhow!("Windows DPAPI CryptProtectData failed"));
        }

        // SAFETY: `out_blob` was allocated by `CryptProtectData` with `cb_data` valid bytes.
        let protected = unsafe {
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let copy = slice.to_vec();
            LocalFree(out_blob.pb_data.cast::<c_void>());
            copy
        };
        Ok(protected)
    }

    pub fn unprotect(ciphertext: &[u8], provider_user: &str) -> Result<Vec<u8>> {
        let mut input = ciphertext.to_vec();
        let in_blob = DataBlob {
            cb_data: u32::try_from(input.len())
                .map_err(|_| anyhow!("Ciphertext exceeds DPAPI limit"))?,
            pb_data: input.as_mut_ptr(),
        };
        let mut entropy = format!("com.easysrt.studio.vault.v2:{provider_user}").into_bytes();
        let entropy_blob = DataBlob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_mut_ptr(),
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };

        // SAFETY: Pointers reference live buffers for the duration of the synchronous call.
        let ok = unsafe {
            CryptUnprotectData(
                &in_blob,
                ptr::null_mut(),
                &entropy_blob,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 || out_blob.pb_data.is_null() {
            return Err(anyhow!("Windows DPAPI CryptUnprotectData failed"));
        }

        // SAFETY: `out_blob` was allocated by `CryptUnprotectData` with `cb_data` valid bytes.
        let plain = unsafe {
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let copy = slice.to_vec();
            LocalFree(out_blob.pb_data.cast::<c_void>());
            copy
        };
        Ok(plain)
    }
}

fn encrypt_vault_secret(paths: &StoragePaths, provider_user: &str, plain: &str) -> Result<String> {
    #[cfg(windows)]
    {
        let _ = paths;
        let protected = dpapi::protect(plain.as_bytes(), provider_user)?;
        Ok(format!("{}{}", DPAPI_ENVELOPE_PREFIX, hex::encode(protected)))
    }

    #[cfg(not(windows))]
    {
        let master_key = load_or_create_vault_key(paths)?;
        let (enc_key, mac_key) = derive_subkeys(&master_key);
        let mut nonce = [0u8; 16];
        fill_csprng_bytes(&mut nonce)?;
        let ciphertext = sha256_ctr_xor(&enc_key, &nonce, plain.as_bytes());
        let tag = compute_vault_mac(&mac_key, provider_user, &nonce, &ciphertext);
        Ok(format!(
            "{}{}:{}:{}",
            VAULT_ENVELOPE_PREFIX,
            hex::encode(nonce),
            hex::encode(ciphertext),
            hex::encode(tag)
        ))
    }
}

fn decrypt_vault_secret(paths: &StoragePaths, provider_user: &str, stored: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let _ = paths;
        let hex_part = stored.strip_prefix(DPAPI_ENVELOPE_PREFIX)?;
        let blob = hex::decode(hex_part).ok()?;
        let plain_bytes = dpapi::unprotect(&blob, provider_user).ok()?;
        let plain = String::from_utf8(plain_bytes).ok()?;
        if plain.trim().is_empty() {
            None
        } else {
            Some(plain)
        }
    }

    #[cfg(not(windows))]
    {
        let body = stored.strip_prefix(VAULT_ENVELOPE_PREFIX)?;
        let mut parts = body.split(':');
        let nonce_hex = parts.next()?;
        let ct_hex = parts.next()?;
        let tag_hex = parts.next()?;
        if parts.next().is_some() {
            return None;
        }

        let nonce_vec = hex::decode(nonce_hex).ok()?;
        if nonce_vec.len() != 16 {
            return None;
        }
        let mut nonce = [0u8; 16];
        nonce.copy_from_slice(&nonce_vec);

        let ciphertext = hex::decode(ct_hex).ok()?;
        let tag = hex::decode(tag_hex).ok()?;
        if tag.len() != 32 {
            return None;
        }

        let master_key = load_existing_vault_key(paths)?;
        let (enc_key, mac_key) = derive_subkeys(&master_key);
        let expected_tag = compute_vault_mac(&mac_key, provider_user, &nonce, &ciphertext);
        if !constant_time_eq(&tag, &expected_tag) {
            tracing::warn!(
                "Rejected tampered or corrupted local vault entry for provider '{}'",
                provider_user
            );
            return None;
        }

        let plain_bytes = sha256_ctr_xor(&enc_key, &nonce, &ciphertext);
        let plain = String::from_utf8(plain_bytes).ok()?;
        if plain.trim().is_empty() {
            None
        } else {
            Some(plain)
        }
    }
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
    atomic_write_bytes_with_mode(path, &bytes, true)
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

fn session_vault_get(paths: &StoragePaths, user: &str) -> Option<String> {
    let key = session_vault_key(paths, user);
    lock_session_vault()
        .get(&key)
        .cloned()
        .filter(|s| !s.trim().is_empty())
}

fn session_vault_set(paths: &StoragePaths, user: &str, secret: &str) {
    let key = session_vault_key(paths, user);
    lock_session_vault().insert(key, secret.to_string());
}

fn session_vault_remove(paths: &StoragePaths, user: &str) {
    let key = session_vault_key(paths, user);
    lock_session_vault().remove(&key);
}

/// Core testable API key lookup with explicit `use_keyring` and `allow_local_vault` controls.
///
/// Lookup hierarchy:
/// 1. OS Keyring (when `use_keyring == true`)
/// 2. In-memory `SessionVault` (process-lifetime RAM only)
/// 3. Encrypted local disk vault (ONLY when `allow_local_vault == true`)
pub fn get_api_key_for_paths_with_opt_in(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    use_keyring: bool,
    allow_local_vault: bool,
) -> Result<Option<String>> {
    let Some(user) = provider_user_key(provider) else {
        return Ok(None);
    };

    if use_keyring {
        if let Ok(Some(secret)) = try_keyring_get(user) {
            return Ok(Some(secret));
        }
    }

    if let Some(mem_secret) = session_vault_get(paths, user) {
        return Ok(Some(mem_secret));
    }

    if !allow_local_vault {
        return Ok(None);
    }

    let _cred_guard = lock_credentials_io();
    let cred_path = credentials_file_path(paths);
    let creds = read_credentials_file(&cred_path);
    Ok(creds
        .entries
        .get(user)
        .and_then(|stored| decrypt_vault_secret(paths, user, stored)))
}

/// Core testable API key status lookup with explicit `use_keyring` and `allow_local_vault` controls.
pub fn get_api_key_status_for_paths_with_opt_in(
    paths: &StoragePaths,
    use_keyring: bool,
    allow_local_vault: bool,
) -> Result<ApiKeyStatus> {
    let mut keyring_available = use_keyring;
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

    let mem_deepseek = session_vault_get(paths, KEYRING_USER_DEEPSEEK);
    let mem_custom = session_vault_get(paths, KEYRING_USER_CUSTOM_OPENAI);

    let (vault_deepseek, vault_custom) = if allow_local_vault {
        let _cred_guard = lock_credentials_io();
        let cred_path = credentials_file_path(paths);
        let file_creds = read_credentials_file(&cred_path);
        let ds = file_creds
            .entries
            .get(KEYRING_USER_DEEPSEEK)
            .and_then(|s| decrypt_vault_secret(paths, KEYRING_USER_DEEPSEEK, s));
        let cu = file_creds
            .entries
            .get(KEYRING_USER_CUSTOM_OPENAI)
            .and_then(|s| decrypt_vault_secret(paths, KEYRING_USER_CUSTOM_OPENAI, s));
        (ds, cu)
    } else {
        (None, None)
    };

    let has_deep_seek_key =
        kr_deepseek.is_some() || mem_deepseek.is_some() || vault_deepseek.is_some();
    let has_custom_api_key = kr_custom.is_some() || mem_custom.is_some() || vault_custom.is_some();

    let using_memory = (kr_deepseek.is_none() && mem_deepseek.is_some())
        || (kr_custom.is_none() && mem_custom.is_some());
    let using_local_vault = (kr_deepseek.is_none() && mem_deepseek.is_none() && vault_deepseek.is_some())
        || (kr_custom.is_none() && mem_custom.is_none() && vault_custom.is_some());

    let storage_backend = if keyring_available && !using_memory && !using_local_vault {
        "keyring".to_string()
    } else if allow_local_vault && (!keyring_available || using_local_vault) && !using_memory {
        "local-vault".to_string()
    } else {
        "memory-session".to_string()
    };

    Ok(ApiKeyStatus {
        has_deep_seek_key,
        has_custom_api_key,
        storage_backend,
    })
}

/// Core testable API key setter with explicit `use_keyring` and `allow_local_vault` controls.
///
/// - When `use_keyring == true` and OS Keyring succeeds, stores in OS Keyring and cleans up any
///   shadow entries in memory/disk.
/// - When OS Keyring is unavailable or disabled:
///   - If `allow_local_vault == false` (default): stores ONLY in RAM (`SessionVault`), returning
///     `storage_backend = "memory-session"` and writing zero files to disk.
///   - If `allow_local_vault == true`: encrypts using DPAPI (Windows) or a CSPRNG `0600` `.vault.key`
///     with SHA-256-CTR + HMAC-SHA256 (Unix) and writes `.credentials.json` atomically (`0600`).
pub fn set_api_key_for_paths_with_opt_in(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    api_key: &str,
    use_keyring: bool,
    allow_local_vault: bool,
) -> Result<ApiKeyStatus> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return clear_api_key_for_paths_with_opt_in(paths, provider, use_keyring, allow_local_vault);
    }

    let Some(user) = provider_user_key(provider) else {
        return get_api_key_status_for_paths_with_opt_in(paths, use_keyring, allow_local_vault);
    };

    let cred_path = credentials_file_path(paths);

    if use_keyring && try_keyring_set(user, trimmed).is_ok() {
        session_vault_remove(paths, user);
        let _cred_guard = lock_credentials_io();
        if cred_path.is_file() {
            let mut creds = read_credentials_file(&cred_path);
            if creds.entries.remove(user).is_some() {
                let _ = write_credentials_file(&cred_path, &creds);
            }
        }
        drop(_cred_guard);
        return get_api_key_status_for_paths_with_opt_in(paths, use_keyring, allow_local_vault);
    }

    if !allow_local_vault {
        session_vault_set(paths, user, trimmed);
        return get_api_key_status_for_paths_with_opt_in(paths, use_keyring, false);
    }

    {
        let _cred_guard = lock_credentials_io();
        let envelope = encrypt_vault_secret(paths, user, trimmed)?;
        let mut creds = read_credentials_file(&cred_path);
        creds.entries.insert(user.to_string(), envelope);
        write_credentials_file(&cred_path, &creds)?;
    }
    session_vault_remove(paths, user);

    get_api_key_status_for_paths_with_opt_in(paths, use_keyring, true)
}

/// Core testable API key clearing helper with explicit `use_keyring` and `allow_local_vault` controls.
pub fn clear_api_key_for_paths_with_opt_in(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    use_keyring: bool,
    allow_local_vault: bool,
) -> Result<ApiKeyStatus> {
    let Some(user) = provider_user_key(provider) else {
        return get_api_key_status_for_paths_with_opt_in(paths, use_keyring, allow_local_vault);
    };

    if use_keyring {
        let _ = try_keyring_delete(user);
    }

    session_vault_remove(paths, user);

    {
        let _cred_guard = lock_credentials_io();
        let cred_path = credentials_file_path(paths);
        if cred_path.is_file() {
            let mut creds = read_credentials_file(&cred_path);
            if creds.entries.remove(user).is_some() {
                write_credentials_file(&cred_path, &creds)?;
            }
        }
    }

    get_api_key_status_for_paths_with_opt_in(paths, use_keyring, allow_local_vault)
}

/// Core testable API key lookup using resolved `StoragePaths` (defaults to `allow_local_vault = false`).
pub fn get_api_key_for_paths(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    force_fallback: bool,
) -> Result<Option<String>> {
    get_api_key_for_paths_with_opt_in(paths, provider, !force_fallback, false)
}

/// Core testable API key status lookup using resolved `StoragePaths` (defaults to `allow_local_vault = false`).
pub fn get_api_key_status_for_paths(
    paths: &StoragePaths,
    force_fallback: bool,
) -> Result<ApiKeyStatus> {
    get_api_key_status_for_paths_with_opt_in(paths, !force_fallback, false)
}

/// Core testable API key setter using resolved `StoragePaths` (defaults to `allow_local_vault = false`).
pub fn set_api_key_for_paths(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    api_key: &str,
    force_fallback: bool,
) -> Result<ApiKeyStatus> {
    set_api_key_for_paths_with_opt_in(paths, provider, api_key, !force_fallback, false)
}

/// Core testable API key clearing helper using resolved `StoragePaths` (defaults to `allow_local_vault = false`).
pub fn clear_api_key_for_paths(
    paths: &StoragePaths,
    provider: TranslationProviderKind,
    force_fallback: bool,
) -> Result<ApiKeyStatus> {
    clear_api_key_for_paths_with_opt_in(paths, provider, !force_fallback, false)
}

/// Retrieve an API key with explicit opt-in flag for encrypted local persistence.
pub fn get_api_key_with_opt_in(
    app: &AppHandle,
    provider: TranslationProviderKind,
    allow_local_vault: bool,
) -> Result<Option<String>> {
    let paths = resolve_storage_paths(app, None)?;
    get_api_key_for_paths_with_opt_in(&paths, provider, true, allow_local_vault)
}

/// Return current API key status with explicit opt-in flag for encrypted local persistence.
pub fn get_api_key_status_with_opt_in(
    app: &AppHandle,
    allow_local_vault: bool,
) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    get_api_key_status_for_paths_with_opt_in(&paths, true, allow_local_vault)
}

/// Save an API key with explicit opt-in flag for encrypted local persistence.
pub fn set_api_key_with_opt_in(
    app: &AppHandle,
    provider: TranslationProviderKind,
    api_key: &str,
    allow_local_vault: bool,
) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    set_api_key_for_paths_with_opt_in(&paths, provider, api_key, true, allow_local_vault)
}

/// Clear an API key with explicit opt-in flag for encrypted local persistence.
pub fn clear_api_key_with_opt_in(
    app: &AppHandle,
    provider: TranslationProviderKind,
    allow_local_vault: bool,
) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    clear_api_key_for_paths_with_opt_in(&paths, provider, true, allow_local_vault)
}

/// Migrates RAM session keys into the opt-in encrypted local vault when `allow_local_vault`
/// is enabled, or moves decrypted vault keys back into the RAM session vault and removes
/// local disk vault files (`.credentials.json` and Unix `.vault.key`) when disabled.
pub fn sync_local_vault_opt_in_for_paths(
    paths: &StoragePaths,
    allow_local_vault: bool,
) -> Result<()> {
    let cred_path = credentials_file_path(paths);
    let users = [KEYRING_USER_DEEPSEEK, KEYRING_USER_CUSTOM_OPENAI];

    if allow_local_vault {
        let mut to_migrate: Vec<(&str, String)> = Vec::new();
        for user in users {
            if let Some(secret) = session_vault_get(paths, user) {
                to_migrate.push((user, secret));
            }
        }
        if !to_migrate.is_empty() {
            let _cred_guard = lock_credentials_io();
            let mut creds = read_credentials_file(&cred_path);
            for (user, secret) in &to_migrate {
                let envelope = encrypt_vault_secret(paths, user, secret)?;
                creds.entries.insert((*user).to_string(), envelope);
            }
            write_credentials_file(&cred_path, &creds)?;
            drop(_cred_guard);
            for (user, _) in to_migrate {
                session_vault_remove(paths, user);
            }
        }
    } else {
        let _cred_guard = lock_credentials_io();
        if cred_path.is_file() {
            let creds = read_credentials_file(&cred_path);
            for user in users {
                if session_vault_get(paths, user).is_none() {
                    if let Some(plain) = creds
                        .entries
                        .get(user)
                        .and_then(|stored| decrypt_vault_secret(paths, user, stored))
                    {
                        session_vault_set(paths, user, &plain);
                    }
                }
            }
            if let Err(err) = fs::remove_file(&cred_path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    return Err(err).with_context(|| {
                        format!(
                            "Failed to remove local credentials file: {}",
                            cred_path.display()
                        )
                    });
                }
            }
        }
        #[cfg(not(windows))]
        {
            let key_path = vault_key_file_path(paths);
            if key_path.exists() {
                if let Err(err) = fs::remove_file(&key_path) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        return Err(err).with_context(|| {
                            format!(
                                "Failed to remove local vault key file: {}",
                                key_path.display()
                            )
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

fn load_allow_local_vault_flag(paths: &StoragePaths) -> bool {
    load_config_from_path(Path::new(&paths.config_path))
        .map(|cfg| cfg.allow_local_encrypted_vault)
        .unwrap_or(false)
}

/// Retrieve an API key for `DeepSeek` or `CustomOpenAi` (honoring `AppConfig.allow_local_encrypted_vault`).
pub fn get_api_key(app: &AppHandle, provider: TranslationProviderKind) -> Result<Option<String>> {
    let paths = resolve_storage_paths(app, None)?;
    let allow = load_allow_local_vault_flag(&paths);
    get_api_key_for_paths_with_opt_in(&paths, provider, true, allow)
}

/// Return current API key presence and active backend (`"keyring"`, `"memory-session"`, or `"local-vault"`),
/// honoring `AppConfig.allow_local_encrypted_vault`.
pub fn get_api_key_status(app: &AppHandle) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    let allow = load_allow_local_vault_flag(&paths);
    get_api_key_status_for_paths_with_opt_in(&paths, true, allow)
}

/// Save an API key for `DeepSeek` or `CustomOpenAi` (honoring `AppConfig.allow_local_encrypted_vault`).
pub fn set_api_key(
    app: &AppHandle,
    provider: TranslationProviderKind,
    api_key: &str,
) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    let allow = load_allow_local_vault_flag(&paths);
    set_api_key_for_paths_with_opt_in(&paths, provider, api_key, true, allow)
}

/// Clear an API key for `DeepSeek` or `CustomOpenAi` (honoring `AppConfig.allow_local_encrypted_vault`).
pub fn clear_api_key(app: &AppHandle, provider: TranslationProviderKind) -> Result<ApiKeyStatus> {
    let paths = resolve_storage_paths(app, None)?;
    let allow = load_allow_local_vault_flag(&paths);
    clear_api_key_for_paths_with_opt_in(&paths, provider, true, allow)
}

// ---------------------------------------------------------------------------
// Checkpoint Persistence (<data_dir>/checkpoints/<job_id>.easysrt.json)
// ---------------------------------------------------------------------------

/// Strictly validates a checkpoint `job_id` against `^[A-Za-z0-9_-]{1,128}$`.
pub fn validate_job_id(job_id: &str) -> Result<()> {
    if job_id.is_empty() || job_id.len() > 128 {
        return Err(anyhow!(
            "Invalid checkpoint job_id length ({}): expected 1..=128 characters matching ^[A-Za-z0-9_-]+$",
            job_id.len()
        ));
    }
    if !job_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(anyhow!(
            "Invalid checkpoint job_id '{}': must match ^[A-Za-z0-9_-]{{1,128}}$",
            job_id
        ));
    }
    Ok(())
}

fn checkpoint_file_path(data_dir: &Path, job_id: &str) -> Result<PathBuf> {
    validate_job_id(job_id)?;
    let checkpoints_dir = data_dir.join("checkpoints");
    let candidate = checkpoints_dir.join(format!("{}.easysrt.json", job_id));
    if candidate.parent() != Some(checkpoints_dir.as_path()) {
        return Err(anyhow!(
            "Invalid checkpoint path outside checkpoints directory for job '{}'",
            job_id
        ));
    }
    Ok(candidate)
}

/// Save a `SubtitleProject` checkpoint atomically inside `<data_dir>/checkpoints/<job_id>.easysrt.json`.
pub fn save_checkpoint_to_data_dir(data_dir: &Path, project: &SubtitleProject) -> Result<()> {
    let path = checkpoint_file_path(data_dir, &project.job_id)?;
    let bytes = serde_json::to_vec_pretty(project)
        .with_context(|| format!("Failed to serialize checkpoint for job {}", project.job_id))?;
    atomic_write_bytes(&path, &bytes)
}

/// Save a `SubtitleProject` checkpoint atomically using resolved `StoragePaths`.
pub fn save_checkpoint_for_paths(paths: &StoragePaths, project: &SubtitleProject) -> Result<()> {
    validate_job_id(&project.job_id)?;
    save_checkpoint_to_data_dir(Path::new(&paths.data_dir), project)
}

/// Seeds `cue.translated_variants` from legacy v0.3.x checkpoints where `translated_text`
/// is populated but `translated_variants` is empty.
pub fn seed_legacy_translated_variants(project: &mut SubtitleProject) {
    let style = project
        .completed_translation_style
        .unwrap_or(project.options.translation_style);
    let mut seeded_any = false;
    for cue in &mut project.cues {
        if cue.translated_variants.is_empty() && !cue.translated_text.trim().is_empty() {
            cue.translated_variants
                .insert(style, cue.translated_text.clone());
            seeded_any = true;
        }
    }
    if project.completed_translation_style.is_none()
        && (seeded_any || project.completed_translation_batches > 0)
    {
        project.completed_translation_style = Some(style);
    }
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
            Some(mut project) => {
                seed_legacy_translated_variants(&mut project);
                projects.push(project);
            }
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
    let mut project: SubtitleProject = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse checkpoint JSON: {}", path.display()))?;
    seed_legacy_translated_variants(&mut project);
    Ok(project)
}

/// Load a specific `SubtitleProject` checkpoint by `job_id` using resolved `StoragePaths`.
pub fn load_checkpoint_for_paths(paths: &StoragePaths, job_id: &str) -> Result<SubtitleProject> {
    validate_job_id(job_id)?;
    load_checkpoint_from_data_dir(Path::new(&paths.data_dir), job_id)
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

/// Delete a specific `SubtitleProject` checkpoint by `job_id` using resolved `StoragePaths`.
pub fn delete_checkpoint_for_paths(paths: &StoragePaths, job_id: &str) -> Result<()> {
    validate_job_id(job_id)?;
    delete_checkpoint_from_data_dir(Path::new(&paths.data_dir), job_id)
}

/// Save a `SubtitleProject` checkpoint (called by `pipeline` after transcription and after each translation batch).
pub fn save_checkpoint(app: &AppHandle, project: &SubtitleProject) -> Result<()> {
    let paths = resolve_storage_paths(app, None)?;
    save_checkpoint_for_paths(&paths, project)
}

/// List all saved checkpoints sorted by `updated_at` descending.
pub fn list_checkpoints(app: &AppHandle) -> Result<Vec<SubtitleProject>> {
    let paths = resolve_storage_paths(app, None)?;
    list_checkpoints_in_data_dir(Path::new(&paths.data_dir))
}

/// Load a checkpoint by `job_id`.
pub fn load_checkpoint(app: &AppHandle, job_id: &str) -> Result<SubtitleProject> {
    let paths = resolve_storage_paths(app, None)?;
    load_checkpoint_for_paths(&paths, job_id)
}

/// Delete a checkpoint by `job_id`.
pub fn delete_checkpoint(app: &AppHandle, job_id: &str) -> Result<()> {
    let paths = resolve_storage_paths(app, None)?;
    delete_checkpoint_for_paths(&paths, job_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{JobOptions, JobPhase, SerbianScript, SubtitleCue, TimedWord};
    use std::sync::Arc;
    use uuid::Uuid;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("easysrt_test_{}_{}", prefix, Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_project(job_id: &str) -> SubtitleProject {
        let mut variants = BTreeMap::new();
        variants.insert(
            crate::models::TranslationStyle::Natural,
            "Zdravo svete".to_string(),
        );
        SubtitleProject {
            job_id: job_id.to_string(),
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
                speaker_change: false,
            }],
            cues: vec![SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1500,
                source_text: "Hello world".to_string(),
                translated_text: "Zdravo svete".to_string(),
                translated_variants: variants,
                cps: 8.0,
                warnings: vec![],
            }],
            phase: JobPhase::Completed,
            completed_translation_batches: 1,
            topic_context: None,
            completed_translation_style: Some(crate::models::TranslationStyle::Natural),
            error_message: None,
            updated_at: "2026-11-22T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn installed_path_resolution_and_override() {
        let base = make_temp_dir("paths");
        let exe_dir = base.join("exe_folder");
        let installed_dir = base.join("installed_data");
        fs::create_dir_all(&exe_dir).unwrap();
        let fake_exe = exe_dir.join("easysrt-studio.exe");
        fs::write(&fake_exe, b"binary").unwrap();

        assert!(default_installed_root_dir().ends_with("com.easysrt.studio"));

        // 1. Canonical Installed mode
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

        // 2. Legacy portable.flag beside executable is ignored -> always Installed mode
        fs::write(exe_dir.join("portable.flag"), b"").unwrap();
        let paths_still_installed =
            resolve_storage_paths_with_exe(Some(&fake_exe), &installed_dir, None).unwrap();
        assert_eq!(paths_still_installed.mode, StorageMode::Installed);
        assert_eq!(
            Path::new(&paths_still_installed.root_dir),
            installed_dir.as_path()
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
    fn config_default_is_english_and_roundtrips_cleanly() {
        let base = make_temp_dir("config");
        let config_path = base.join("config.json");

        // Missing file -> returns default with "en" UI language
        let initial = load_config_from_path(&config_path).unwrap();
        assert_eq!(initial.ui_language, "en");
        assert_eq!(initial.default_script, SerbianScript::Latin);
        assert!(!initial.has_completed_setup);

        // Modify and save atomically
        let custom_models_dir = base.join("my_models");
        let mut updated = initial.clone();
        updated.ui_language = "sr".to_string();
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
    fn atomic_write_bytes_survives_32_thread_parallel_stress() {
        let base = make_temp_dir("atomic_stress");
        let target = Arc::new(base.join("shared.json"));

        let mut handles = Vec::new();
        for tid in 0..32usize {
            let target_ref = Arc::clone(&target);
            handles.push(std::thread::spawn(move || {
                for iter in 0..15usize {
                    let payload = serde_json::json!({
                        "thread": tid,
                        "iter": iter,
                        "data": "EasySRT-Studio-atomic-write-stress-check"
                    });
                    let bytes = serde_json::to_vec(&payload).unwrap();
                    atomic_write_bytes(&target_ref, &bytes).unwrap();
                }
            }));
        }

        for h in handles {
            h.join().expect("worker thread panicked during atomic write");
        }

        // Final file must parse as intact JSON
        let raw = fs::read_to_string(&*target).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed.get("data").and_then(|v| v.as_str()),
            Some("EasySRT-Studio-atomic-write-stress-check")
        );

        // Zero leftover .tmp files in parent directory
        let leftovers: Vec<String> = fs::read_dir(&base)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "Expected 0 leftover .tmp files, found: {leftovers:?}"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn update_config_for_paths_serializes_concurrent_rmw() {
        let base = make_temp_dir("config_rmw");
        let paths = Arc::new(
            resolve_storage_paths_from_root(&base, StorageMode::Installed, None).unwrap(),
        );

        let mut handles = Vec::new();
        for tid in 0..16usize {
            let paths_ref = Arc::clone(&paths);
            handles.push(std::thread::spawn(move || {
                update_config_for_paths(&paths_ref, |cfg| {
                    cfg.custom_models.push(crate::models::CustomModelEntry {
                        id: format!("custom-{tid}"),
                        name: format!("Model {tid}"),
                        kind: crate::models::CatalogItemKind::LlmModel,
                        path: format!("/tmp/model_{tid}.gguf"),
                        size_bytes: 1024,
                    });
                })
                .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let final_cfg = load_config_from_path(Path::new(&paths.config_path)).unwrap();
        assert_eq!(final_cfg.custom_models.len(), 16);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn checkpoint_crud_and_descending_sort() {
        let base = make_temp_dir("checkpoints");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Installed, None).unwrap();
        let data_dir = Path::new(&paths.data_dir);

        let proj_older = sample_project("job-100");
        let mut proj_newer = proj_older.clone();
        proj_newer.job_id = "job-200".to_string();
        proj_newer.updated_at = "2026-11-22T12:00:00Z".to_string();

        save_checkpoint_for_paths(&paths, &proj_older).unwrap();
        save_checkpoint_for_paths(&paths, &proj_newer).unwrap();

        let listed = list_checkpoints_in_data_dir(data_dir).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].job_id, "job-200");
        assert_eq!(listed[1].job_id, "job-100");

        let loaded = load_checkpoint_for_paths(&paths, "job-100").unwrap();
        assert_eq!(loaded, proj_older);

        delete_checkpoint_for_paths(&paths, "job-100").unwrap();
        let listed_after = list_checkpoints_in_data_dir(data_dir).unwrap();
        assert_eq!(listed_after.len(), 1);
        assert_eq!(listed_after[0].job_id, "job-200");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn validate_job_id_rejects_traversal_and_invalid_inputs() {
        let base = make_temp_dir("job_id_validation");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Installed, None).unwrap();

        // Valid IDs
        assert!(validate_job_id("job-123_ABC").is_ok());
        assert!(validate_job_id(&"a".repeat(128)).is_ok());

        // Invalid / traversal IDs
        let too_long = "a".repeat(129);
        let invalid_ids = [
            "",
            "../escape",
            "..\\escape",
            "foo/bar",
            "foo\\bar",
            "job.1",
            "job id",
            " job-1",
            "job\0id",
            too_long.as_str(),
        ];

        for bad in invalid_ids {
            assert!(
                validate_job_id(bad).is_err(),
                "Expected validate_job_id({bad:?}) to fail"
            );
            let bad_proj = sample_project(bad);
            assert!(save_checkpoint_for_paths(&paths, &bad_proj).is_err());
            assert!(load_checkpoint_for_paths(&paths, bad).is_err());
            assert!(delete_checkpoint_for_paths(&paths, bad).is_err());
        }

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn credential_file_fallback_works() {
        let base = make_temp_dir("creds");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Installed, None).unwrap();
        let cred_file = credentials_file_path(&paths);

        // -------------------------------------------------------------------
        // Part 1: Default fallback (allow_local_vault = false) -> RAM-only SessionVault
        // -------------------------------------------------------------------
        let initial_ram = get_api_key_status_for_paths(&paths, true).unwrap();
        assert!(!initial_ram.has_deep_seek_key);
        assert!(!initial_ram.has_custom_api_key);
        assert_eq!(initial_ram.storage_backend, "memory-session");

        let after_ram_set = set_api_key_for_paths(
            &paths,
            TranslationProviderKind::DeepSeek,
            "sk-ram-session-secret-123",
            true,
        )
        .unwrap();
        assert!(after_ram_set.has_deep_seek_key);
        assert!(!after_ram_set.has_custom_api_key);
        assert_eq!(after_ram_set.storage_backend, "memory-session");

        // Zero files written to disk in RAM-session mode!
        assert!(
            !cred_file.exists(),
            "RAM-only fallback must NOT write .credentials.json to disk"
        );
        #[cfg(not(windows))]
        assert!(
            !vault_key_file_path(&paths).exists(),
            "RAM-only fallback must NOT write .vault.key to disk"
        );

        assert_eq!(
            get_api_key_for_paths(&paths, TranslationProviderKind::DeepSeek, true)
                .unwrap()
                .as_deref(),
            Some("sk-ram-session-secret-123")
        );

        let after_ram_clear =
            clear_api_key_for_paths(&paths, TranslationProviderKind::DeepSeek, true).unwrap();
        assert!(!after_ram_clear.has_deep_seek_key);
        assert_eq!(
            get_api_key_for_paths(&paths, TranslationProviderKind::DeepSeek, true).unwrap(),
            None
        );

        // -------------------------------------------------------------------
        // Part 2: Explicit opt-in encrypted local vault (allow_local_vault = true)
        // -------------------------------------------------------------------
        let after_ds = set_api_key_for_paths_with_opt_in(
            &paths,
            TranslationProviderKind::DeepSeek,
            "sk-deepseek-test-12345",
            false,
            true,
        )
        .unwrap();
        assert!(after_ds.has_deep_seek_key);
        assert!(!after_ds.has_custom_api_key);
        assert_eq!(after_ds.storage_backend, "local-vault");

        // Raw secret must never appear in plaintext on disk
        let raw_on_disk = fs::read_to_string(&cred_file).unwrap();
        assert!(!raw_on_disk.contains("sk-deepseek-test-12345"));

        // Verify strict Unix 0600 file creation permissions (zero TOCTOU)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let cred_mode = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                cred_mode, 0o600,
                "Expected 0600 mode on .credentials.json, got {:o}",
                cred_mode
            );
            let key_mode = fs::metadata(vault_key_file_path(&paths))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                key_mode, 0o600,
                "Expected 0600 mode on .vault.key, got {:o}",
                key_mode
            );
        }

        // Verify authenticated decryption round-trip
        let fetched = get_api_key_for_paths_with_opt_in(
            &paths,
            TranslationProviderKind::DeepSeek,
            false,
            true,
        )
        .unwrap();
        assert_eq!(fetched.as_deref(), Some("sk-deepseek-test-12345"));

        // Verify without opt-in, disk vault is not read
        assert_eq!(
            get_api_key_for_paths_with_opt_in(
                &paths,
                TranslationProviderKind::DeepSeek,
                false,
                false,
            )
            .unwrap(),
            None
        );

        // Save CustomOpenAi key
        let after_custom = set_api_key_for_paths_with_opt_in(
            &paths,
            TranslationProviderKind::CustomOpenAi,
            "sk-custom-999",
            false,
            true,
        )
        .unwrap();
        assert!(after_custom.has_deep_seek_key);
        assert!(after_custom.has_custom_api_key);
        assert_eq!(
            get_api_key_for_paths_with_opt_in(
                &paths,
                TranslationProviderKind::CustomOpenAi,
                false,
                true,
            )
            .unwrap()
            .as_deref(),
            Some("sk-custom-999")
        );

        // Tampering with ciphertext/envelope must cleanly reject (return None)
        let mut tampered_creds = read_credentials_file(&cred_file);
        if let Some(val) = tampered_creds.entries.get_mut(KEYRING_USER_DEEPSEEK) {
            let flipped = if val.ends_with('0') { "1" } else { "0" };
            val.pop();
            val.push_str(flipped);
        }
        write_credentials_file(&cred_file, &tampered_creds).unwrap();
        assert_eq!(
            get_api_key_for_paths_with_opt_in(
                &paths,
                TranslationProviderKind::DeepSeek,
                false,
                true,
            )
            .unwrap(),
            None
        );

        // Legacy/corrupted obf1 entry must cleanly reject without panicking
        let mut legacy_creds = read_credentials_file(&cred_file);
        legacy_creds
            .entries
            .insert(KEYRING_USER_DEEPSEEK.to_string(), "obf1:deadbeef".to_string());
        write_credentials_file(&cred_file, &legacy_creds).unwrap();
        assert_eq!(
            get_api_key_for_paths_with_opt_in(
                &paths,
                TranslationProviderKind::DeepSeek,
                false,
                true,
            )
            .unwrap(),
            None
        );

        // Clear CustomOpenAi key
        let after_clear = clear_api_key_for_paths_with_opt_in(
            &paths,
            TranslationProviderKind::CustomOpenAi,
            false,
            true,
        )
        .unwrap();
        assert!(!after_clear.has_custom_api_key);
        assert_eq!(
            get_api_key_for_paths_with_opt_in(
                &paths,
                TranslationProviderKind::CustomOpenAi,
                false,
                true,
            )
            .unwrap(),
            None
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_test_vectors() {
        // RFC 4231 Test Case 1
        let key1 = [0x0bu8; 20];
        let mac1 = hmac_sha256(&key1, b"Hi There");
        assert_eq!(
            hex::encode(mac1),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );

        // RFC 4231 Test Case 2
        let mac2 = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac2),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn save_config_toggle_migrates_ram_keys_to_local_vault_and_cleans_up_on_disable() {
        let base = make_temp_dir("vault_toggle");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Installed, None).unwrap();
        let cred_file = credentials_file_path(&paths);

        // 1. Store key with allow_local_vault = false -> stays strictly in RAM
        let status_ram = set_api_key_for_paths_with_opt_in(
            &paths,
            TranslationProviderKind::DeepSeek,
            "sk-migrate-secret-999",
            false,
            false,
        )
        .unwrap();
        assert_eq!(status_ram.storage_backend, "memory-session");
        assert!(!cred_file.exists());

        // 2. Toggle allow_local_encrypted_vault = true via save_config_for_paths -> migrates RAM to disk vault
        let mut cfg = default_app_config();
        cfg.allow_local_encrypted_vault = true;
        save_config_for_paths(&paths, &cfg).unwrap();

        assert!(cred_file.is_file(), "Expected .credentials.json after opt-in migration");
        let status_disk = get_api_key_status_for_paths_with_opt_in(&paths, false, true).unwrap();
        assert!(status_disk.has_deep_seek_key);
        assert_eq!(status_disk.storage_backend, "local-vault");
        assert_eq!(
            get_api_key_for_paths_with_opt_in(&paths, TranslationProviderKind::DeepSeek, false, true)
                .unwrap()
                .as_deref(),
            Some("sk-migrate-secret-999")
        );

        // 3. Toggle allow_local_encrypted_vault = false -> removes disk vault files & preserves RAM session
        cfg.allow_local_encrypted_vault = false;
        save_config_for_paths(&paths, &cfg).unwrap();

        assert!(
            !cred_file.exists(),
            "Disabling local encrypted vault must delete .credentials.json"
        );
        #[cfg(not(windows))]
        assert!(
            !vault_key_file_path(&paths).exists(),
            "Disabling local encrypted vault must delete .vault.key"
        );
        let status_back_ram =
            get_api_key_status_for_paths_with_opt_in(&paths, false, false).unwrap();
        assert!(status_back_ram.has_deep_seek_key);
        assert_eq!(status_back_ram.storage_backend, "memory-session");
        assert_eq!(
            get_api_key_for_paths_with_opt_in(
                &paths,
                TranslationProviderKind::DeepSeek,
                false,
                false,
            )
            .unwrap()
            .as_deref(),
            Some("sk-migrate-secret-999")
        );

        let _ = clear_api_key_for_paths_with_opt_in(
            &paths,
            TranslationProviderKind::DeepSeek,
            false,
            false,
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn legacy_checkpoint_seeds_translated_variants_on_load() {
        let base = make_temp_dir("legacy_cp_seed");
        let paths = resolve_storage_paths_from_root(&base, StorageMode::Installed, None).unwrap();
        let cp_path = Path::new(&paths.data_dir)
            .join("checkpoints")
            .join("legacy-seed-job.easysrt.json");

        let legacy_json = serde_json::json!({
            "jobId": "legacy-seed-job",
            "audioPath": "/tmp/a.mp3",
            "audioFileName": "a.mp3",
            "durationMs": 2000,
            "detectedLanguage": "en",
            "options": {
                "sourceLanguage": "en",
                "targetLanguage": "sr",
                "script": "latin",
                "segmentRules": {
                    "preset": "standard",
                    "maxCharsPerLine": 42,
                    "maxLines": 2,
                    "minDurationMs": 1000,
                    "maxDurationMs": 7000,
                    "maxCps": 17.0
                },
                "glossary": {},
                "polishSerbian": true,
                "translationStyle": "formal",
                "providerOverride": null
            },
            "words": [],
            "cues": [{
                "id": 1,
                "startMs": 0,
                "endMs": 2000,
                "sourceText": "Good morning.",
                "translatedText": "Dobro jutro.",
                "cps": 6.0,
                "warnings": []
            }],
            "phase": "completed",
            "completedTranslationBatches": 1,
            "errorMessage": null,
            "updatedAt": "2026-11-22T12:00:00Z"
        });
        fs::write(&cp_path, serde_json::to_vec_pretty(&legacy_json).unwrap()).unwrap();

        let loaded = load_checkpoint_for_paths(&paths, "legacy-seed-job").unwrap();
        assert_eq!(
            loaded.completed_translation_style,
            Some(crate::models::TranslationStyle::Formal)
        );
        assert_eq!(
            loaded.cues[0]
                .translated_variants
                .get(&crate::models::TranslationStyle::Formal)
                .map(String::as_str),
            Some("Dobro jutro.")
        );

        let listed = list_checkpoints_in_data_dir(Path::new(&paths.data_dir)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].cues[0]
                .translated_variants
                .get(&crate::models::TranslationStyle::Formal)
                .map(String::as_str),
            Some("Dobro jutro.")
        );

        let _ = fs::remove_dir_all(&base);
    }
}
