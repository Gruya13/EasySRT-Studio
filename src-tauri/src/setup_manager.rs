//! Catalog loading, component installation, checksum verification, extraction,
//! and custom model import/deletion.
//! Stub module to be implemented in Wave 2.

use crate::models::{
    CatalogItemKind, CatalogManifest, CustomModelEntry, InstalledComponentStatus, SetupStatus,
};
use anyhow::{anyhow, Result};
use tauri::AppHandle;

pub fn load_catalog(_app: &AppHandle) -> Result<CatalogManifest> {
    Err(anyhow!("setup_manager::load_catalog not implemented"))
}

pub fn get_setup_status(_app: &AppHandle) -> Result<SetupStatus> {
    Err(anyhow!("setup_manager::get_setup_status not implemented"))
}

pub async fn install_component(
    _app: &AppHandle,
    _item_id: &str,
) -> Result<InstalledComponentStatus> {
    Err(anyhow!("setup_manager::install_component not implemented"))
}

pub async fn install_recommended_bundle(_app: &AppHandle) -> Result<SetupStatus> {
    Err(anyhow!(
        "setup_manager::install_recommended_bundle not implemented"
    ))
}

pub fn import_custom_model(
    _app: &AppHandle,
    _source_path: &str,
    _kind: CatalogItemKind,
    _name: Option<String>,
) -> Result<CustomModelEntry> {
    Err(anyhow!("setup_manager::import_custom_model not implemented"))
}

pub fn delete_model(_app: &AppHandle, _model_id: &str) -> Result<()> {
    Err(anyhow!("setup_manager::delete_model not implemented"))
}
