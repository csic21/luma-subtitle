import { invoke } from "@tauri-apps/api/core";

import type {
  DownloadStatus,
  EnvironmentState,
  QueueSettings,
  SettingsState,
  SubtitlePreview,
  TaskOperation,
  TaskRecord,
} from "@/types";

type TaskCreatePayload = {
  video_path?: string;
  srt_path?: string;
  output_dir: string | null;
  target_language: string;
  whisper_model_path: string;
  whisper_language: string;
  base_url: string;
  base_url_is_complete: boolean;
  model: string;
  temperature: number;
  translation_shard_size: number;
};

type TaskSettingsUpdatePayload = {
  target_language: string;
  whisper_model_path: string;
  whisper_language: string;
  base_url: string;
  base_url_is_complete: boolean;
  model: string;
  temperature: number;
  translation_shard_size: number;
};

export function listTasks() {
  return invoke<TaskRecord[]>("list_tasks");
}

export function getTask(taskId: string) {
  return invoke<TaskRecord>("get_task", { taskId });
}

export function getTaskLogs(taskId: string) {
  return invoke<string[]>("get_task_logs", { taskId });
}

export function loadSettings() {
  return invoke<SettingsState>("load_settings");
}

export function saveSettings(payload: SettingsState & { api_key: string }) {
  return invoke<SettingsState>("save_settings", { payload });
}

export function loadQueueSettings() {
  return invoke<QueueSettings>("load_queue_settings");
}

export function saveQueueSettings(maxConcurrency: number, autoStartNext: boolean) {
  return invoke<QueueSettings>("save_queue_settings", {
    maxConcurrency,
    autoStartNext,
  });
}

export function selectOutputDir() {
  return invoke<string | null>("select_output_dir");
}

export function selectVideo() {
  return invoke<string | null>("select_video");
}

export function selectSrt() {
  return invoke<string | null>("select_srt");
}

export function selectWhisperModel() {
  return invoke<string | null>("select_whisper_model");
}

export function createVideoTask(request: TaskCreatePayload) {
  return invoke<TaskRecord>("create_video_task", { request });
}

export function createSrtTask(request: TaskCreatePayload) {
  return invoke<TaskRecord>("create_srt_task", { request });
}

export function runTaskOperation(taskId: string, operation: TaskOperation) {
  return invoke("run_task_operation", { taskId, operation });
}

export function runTaskOperations(taskIds: string[], operation: TaskOperation) {
  return invoke("run_task_operations", { taskIds, operation });
}

export function cancelTask(taskId: string) {
  return invoke("cancel_task", { taskId });
}

export function deleteTask(taskId: string) {
  return invoke("delete_task", { taskId });
}

export function applyCurrentSettingsToTask(taskId: string) {
  return invoke<TaskRecord>("apply_current_settings_to_task", { taskId });
}

export function updateTaskSettings(taskId: string, settings: TaskSettingsUpdatePayload) {
  return invoke<TaskRecord>("update_task_settings", { taskId, settings });
}

export function subtitlePreview(taskId: string) {
  return invoke<SubtitlePreview>("subtitle_preview", { jobId: taskId });
}

export function openPath(path: string) {
  return invoke("open_path", { path });
}

export function checkEnvironment() {
  return invoke<EnvironmentState>("check_environment");
}

export function downloadStatus() {
  return invoke<DownloadStatus>("download_status");
}

export function downloadWhisperModel(presetId: string) {
  return invoke<string>("download_whisper_model", {
    request: { preset_id: presetId },
  });
}

export function installDependencies() {
  return invoke<string[]>("install_dependencies");
}
