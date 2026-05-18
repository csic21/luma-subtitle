export type TFunction = (key: string, values?: Record<string, string | number>) => string;

export type TaskOperation = "transcribe" | "translate" | "export";

export type SettingsState = {
  base_url: string;
  model: string;
  temperature: number;
  translation_shard_size: number;
  whisper_model_path: string;
  whisper_language: string;
  target_language: string;
  has_api_key: boolean;
};

export type EnvironmentState = {
  ffmpeg_path: string | null;
  whisper_path: string | null;
  gpu_name: string | null;
  cuda_driver: string | null;
  resource_dir: string;
  config_dir: string;
  sidecar_dir: string;
  model_dir: string;
};

export type JobOutputs = {
  source_file_name: string;
  translated_file_name?: string | null;
  output_dir: string;
  segment_count: number;
};

export type JobEvent = {
  job_id: string;
  stage: string;
  status: "running" | "completed" | "failed" | "cancelled";
  message: string;
  progress: number;
  outputs?: JobOutputs | null;
  error?: string | null;
};

export type ModelDownloadEvent = {
  preset_id: string;
  file_name: string;
  status: "running" | "completed" | "failed";
  message: string;
  progress: number;
  path?: string | null;
  error?: string | null;
  bytes_per_second?: number | null;
  eta_seconds?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
};

export type DependencyInstallEvent = {
  item: string;
  status: "running" | "completed" | "failed";
  message: string;
  progress: number;
  path?: string | null;
  error?: string | null;
  bytes_per_second?: number | null;
  eta_seconds?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
};

export type DownloadStatus = {
  model?: ModelDownloadEvent | null;
  dependency?: DependencyInstallEvent | null;
};

export type TaskSettingsSnapshot = {
  output_dir?: string | null;
  target_language: string;
  whisper_model_path: string;
  whisper_language: string;
  base_url: string;
  model: string;
  temperature: number;
  translation_shard_size: number;
};

export type TaskRecord = {
  id: string;
  source_type: "video" | "srt";
  video_path?: string | null;
  srt_path?: string | null;
  file_name: string;
  status: string;
  stage: string;
  message: string;
  progress: number;
  settings: TaskSettingsSnapshot;
  source_srt_path?: string | null;
  translated_srt_path?: string | null;
  source_file_name?: string | null;
  translated_file_name?: string | null;
  output_dir?: string | null;
  segment_count?: number | null;
  exported_source_srt?: string | null;
  exported_translated_srt?: string | null;
  exported_output_dir?: string | null;
  error?: string | null;
  created_at: number;
  updated_at: number;
};

export type SubtitlePreview = {
  source_srt: string;
  translated_srt?: string | null;
  source_file_name: string;
  translated_file_name?: string | null;
};

export type QueueSettings = {
  max_concurrency: number;
  auto_start_next: boolean;
};

export type WhisperModelPreset = {
  id: string;
  fileName: string;
};

export type WhisperLanguageOption = {
  value: string;
  labelKey: string;
};

export type TargetLanguageOption = {
  value: string;
  labelKey: string;
};

export type ModelPresetView = WhisperModelPreset & {
  labelKey: string;
};
