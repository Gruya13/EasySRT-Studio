pub mod commands;
pub mod diagnostics;
pub mod hardware;
pub mod models;
pub mod pipeline;
pub mod setup_manager;
pub mod storage;

use pipeline::PipelineState;
use std::path::PathBuf;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let default_data_dir = storage::default_installed_root_dir().join("data");
    diagnostics::init_logging(&default_data_dir);
    diagnostics::install_panic_hook();

    tracing::info!(
        "EasySRT Studio v{} starting on {}/{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    tauri::Builder::default()
        .setup(|app| {
            if let Ok(paths) = storage::resolve_storage_paths(app.handle(), None) {
                diagnostics::set_app_data_dir(PathBuf::from(paths.data_dir));
            }
            Ok(())
        })
        .manage(PipelineState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_setup_status,
            commands::get_catalog,
            commands::install_component,
            commands::install_core_runtime,
            commands::install_recommended_bundle,
            commands::import_custom_model,
            commands::delete_model,
            commands::get_config,
            commands::save_config,
            commands::get_api_key_status,
            commands::set_api_key,
            commands::clear_api_key,
            commands::list_checkpoints,
            commands::load_checkpoint,
            commands::save_project_checkpoint,
            commands::delete_checkpoint,
            commands::import_subtitle_file,
            commands::try_load_companion_subtitles,
            commands::transcribe_audio,
            commands::resegment_project,
            commands::translate_project,
            commands::polish_project,
            commands::condense_project_cps,
            commands::run_full_pipeline,
            commands::run_local_llm_batch,
            commands::cancel_job,
            commands::convert_cues_script,
            commands::export_subtitles,
            commands::prepare_audio_waveform,
            commands::probe_media_file,
            commands::cancel_media_probe,
            commands::set_window_maximized,
            commands::collect_diagnostics_preview,
            commands::submit_bug_report,
            commands::submit_crash_report,
            commands::flush_pending_crash_reports,
            commands::save_diagnostics_local,
        ])
        .run(tauri::generate_context!())
        .expect("error while running EasySRT Studio");
}
