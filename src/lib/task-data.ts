import { defaultSettings } from "@/config";
import type { JobEvent, SettingsState, TaskRecord, TaskSettingsSnapshot } from "@/types";

export function upsertTask(tasks: TaskRecord[], incoming: TaskRecord) {
  const exists = tasks.some((task) => task.id === incoming.id);
  const next = exists ? tasks.map((task) => (task.id === incoming.id ? incoming : task)) : [incoming, ...tasks];
  return next.sort((a, b) => b.updated_at - a.updated_at || b.created_at - a.created_at);
}

export function applyJobEventToTask(task: TaskRecord, event: JobEvent): TaskRecord {
  if (task.id !== event.job_id) return task;
  const outputs = event.outputs ?? null;
  return {
    ...task,
    status: event.status,
    stage: event.stage,
    message: event.message,
    progress: event.progress,
    source_file_name: outputs?.source_file_name ?? task.source_file_name,
    translated_file_name: outputs?.translated_file_name ?? task.translated_file_name,
    output_dir: outputs?.output_dir ?? task.output_dir,
    segment_count: outputs?.segment_count ?? task.segment_count,
    error: event.error ?? null,
    updated_at: Math.floor(Date.now() / 1000),
  };
}

export function applyJobEventToTasks(tasks: TaskRecord[], event: JobEvent) {
  let changed = false;
  const next = tasks.map((task) => {
    if (task.id !== event.job_id) return task;
    changed = true;
    return applyJobEventToTask(task, event);
  });
  return changed ? next.sort((a, b) => b.updated_at - a.updated_at || b.created_at - a.created_at) : tasks;
}

export function appendRealtimeLog(logs: string[], event: JobEvent) {
  const lines = [`${event.stage} · ${event.message}`];
  if (event.error) lines.push(`error · ${event.error}`);
  const next = [...logs];
  for (const line of lines) {
    if (!next.includes(line)) next.push(line);
  }
  return next;
}

export function normalizeTaskSettings(settings: TaskSettingsSnapshot): TaskSettingsSnapshot {
  return {
    ...settings,
    translation_shard_size: settings.translation_shard_size ?? defaultSettings.translation_shard_size,
  };
}

export function taskSettingsEqual(left: TaskSettingsSnapshot, right: TaskSettingsSnapshot) {
  const normalizedLeft = normalizeTaskSettings(left);
  const normalizedRight = normalizeTaskSettings(right);

  return (
    normalizedLeft.output_dir === normalizedRight.output_dir &&
    normalizedLeft.target_language === normalizedRight.target_language &&
    normalizedLeft.whisper_model_path === normalizedRight.whisper_model_path &&
    normalizedLeft.whisper_language === normalizedRight.whisper_language &&
    normalizedLeft.base_url === normalizedRight.base_url &&
    normalizedLeft.model === normalizedRight.model &&
    normalizedLeft.temperature === normalizedRight.temperature &&
    normalizedLeft.translation_shard_size === normalizedRight.translation_shard_size
  );
}

export function taskSettingsUpdatePayload(settings: TaskSettingsSnapshot) {
  return {
    target_language: settings.target_language,
    whisper_model_path: settings.whisper_model_path,
    whisper_language: settings.whisper_language,
    base_url: settings.base_url,
    model: settings.model,
    temperature: settings.temperature,
    translation_shard_size: settings.translation_shard_size ?? defaultSettings.translation_shard_size,
  };
}

export function taskCreatePayload(
  settings: SettingsState & { output_dir?: string | null },
  path: string,
  sourceType: "video" | "srt" = "video",
) {
  return {
    [sourceType === "video" ? "video_path" : "srt_path"]: path,
    output_dir: settings.output_dir || null,
    target_language: settings.target_language,
    whisper_model_path: settings.whisper_model_path,
    whisper_language: settings.whisper_language,
    base_url: settings.base_url,
    model: settings.model,
    temperature: settings.temperature,
    translation_shard_size: settings.translation_shard_size,
  };
}
