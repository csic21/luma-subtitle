#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod asr;
mod asr_components;
mod commands;
mod dependencies;
mod environment;
mod job_events;
mod jobs;
mod paths;
mod owned_process;
mod process_utils;
mod settings;
mod state;
mod subtitles;
mod task_db;
mod translation;

#[cfg(test)]
mod tests;

use state::AppState;

fn main() {
    #[cfg(windows)]
    if let Some(code) = asr_components::signature_helper_from_args() { std::process::exit(code); }
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .manage(asr::AsrRuntime::default())
        .manage(asr_components::ComponentManager::default())
        .setup(|app| {
            asr_components::initialize(app.handle())?;
            task_db::init(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::select_video,
            commands::select_audio,
            commands::select_output_dir,
            commands::select_whisper_model,
            commands::select_translation_model,
            commands::select_srt,
            settings::load_settings,
            settings::save_settings,
            asr_components::asr_component_catalog,
            asr_components::asr_component_status,
            asr_components::install_asr_component,
            asr_components::repair_asr_component,
            asr_components::remove_asr_component,
            asr_components::cancel_asr_component,
            asr::check_asr_backend,
            asr::release_asr_backend,
            translation::cli::check_translation_cli,
            translation::cli::list_translation_cli_models,
            environment::check_environment,
            dependencies::download_status,
            dependencies::install_dependencies,
            dependencies::install_llama_cpp,
            dependencies::download_whisper_model,
            dependencies::download_translation_model,
            jobs::list_tasks,
            jobs::get_task,
            jobs::get_task_logs,
            jobs::apply_current_settings_to_task,
            jobs::update_task_settings,
            jobs::create_video_task,
            jobs::create_audio_task,
            jobs::create_srt_task,
            jobs::delete_task,
            jobs::run_task_operation,
            jobs::run_task_operations,
            jobs::cancel_task,
            jobs::load_queue_settings,
            jobs::save_queue_settings,
            jobs::subtitle_preview,
            jobs::save_source_subtitles,
            jobs::save_translated_subtitles,
            commands::open_path
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Luma Subtitle")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                tauri::async_runtime::block_on(async {
                    app.state::<asr_components::ComponentManager>().shutdown().await;
                    app.state::<asr::AsrRuntime>().shutdown().await;
                });
            }
        });
}
