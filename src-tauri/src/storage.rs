//! Storage path resolution (Installed vs Portable), config persistence,
//! checkpoint storage, and API key management.
//! Stub module to be implemented in Wave 2.

use crate::models::{
    ApiKeyStatus, AppConfig, StoragePaths, SubtitleProject, TranslationProviderKind,
};
use anyhow::{anyhow, Result};
use tauri::AppHandle;

pub fn resolve_storage_paths(_app: &AppHandle, _models_dir_override: Option<&str>) -> Result<StoragePaths> {
    Err(anyhow!("storage::resolve_storage_paths not implemented"))
}

pub fn load_config(_app: &AppHandle) -> Result<AppConfig> {
    Err(anyhow!("storage::load_config not implemented"))
}

pub fn save_config(_app: &AppHandle, _config: &AppConfig) -> Result<AppConfig> {
    Err(anyhow!("storage::save_config not implemented"))
}

pub fn get_api_key_status(_app: &AppHandle) -> Result<ApiKeyStatus> {
    Err(anyhow!("storage::get_api_key_status not implemented"))
}

pub fn set_api_key(
    _app: &AppHandle,
    _provider: TranslationProviderKind,
    _api_key: &str,
) -> Result<ApiKeyStatus> {
    Err(anyhow!("storage::set_api_key not implemented"))
}

pub fn clear_api_key(
    _app: &AppHandle,
    _provider: TranslationProviderKind,
) -> Result<ApiKeyStatus> {
    Err(anyhow!("storage::clear_api_key not implemented"))
}

pub fn list_checkpoints(_app: &AppHandle) -> Result<Vec<SubtitleProject>> {
    Err(anyhow!("storage::list_checkpoints not implemented"))
}

pub fn load_checkpoint(_app: &AppHandle, _job_id: &str) -> Result<SubtitleProject> {
    Err(anyhow!("storage::load_checkpoint not implemented"))
}

pub fn delete_checkpoint(_app: &AppHandle, _job_id: &str) -> Result<()> {
    Err(anyhow!("storage::delete_checkpoint not implemented"))
}
