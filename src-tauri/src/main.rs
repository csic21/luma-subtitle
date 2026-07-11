#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dependencies;
mod environment;
mod job_events;
mod jobs;
mod paths;
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
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            task_db::init(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::select_video,
            commands::select_audio,
            commands::select_output_dir,
            commands::select_whisper_model,
            commands::select_srt,
            settings::load_settings,
            settings::save_settings,
            environment::check_environment,
            dependencies::download_status,
            dependencies::install_dependencies,
            dependencies::download_whisper_model,
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
            commands::open_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Luma Subtitle");
}
