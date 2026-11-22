pub mod commands;
pub mod hardware;
pub mod models;
pub mod pipeline;
pub mod setup_manager;
pub mod storage;

use pipeline::PipelineState;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    tauri::Builder::default()
        .manage(PipelineState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_setup_status,
            commands::get_catalog,
            commands::install_component,
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
            commands::delete_checkpoint,
            commands::import_subtitle_file,
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
            commands::read_audio_preview_base64,
        ])
        .run(tauri::generate_context!())
        .expect("error while running EasySRT Studio");
}
