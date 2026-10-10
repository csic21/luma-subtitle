import { asrConfigurationIssues, normalizeAsrConfig, type AsrConfigurationIssue } from "@/lib/asr-config";
import type { Locale } from "@/i18n";
import type { DependencyInstallEvent, ModelDownloadEvent, TaskOperation, TaskRecord, TFunction } from "@/types";

export type OperationRequirementIssue =
  | AsrConfigurationIssue
  | "missingFfmpeg"
  | "taskBusy"
  | "unsupportedSource"
  | "missingSourceSubtitles"
  | "missingTranslationProgress"
  | "missingWhisperModel"
  | "missingEnvironment"
  | "missingBaseUrl"
  | "missingTranslationModel"
  | "missingApiKey"
  | "missingCliCommand"
  | "missingCliModel"
  | "missingLocalTranslationModel"
  | "missingLocalTranslationEngine";

export type OperationReadinessContext = {
  environmentReady: boolean;
  ffmpegReady?: boolean;
  hasApiCredential: boolean;
  llamaReady: boolean;
};

export function isLocalTranslationProvider(provider?: string | null) {
  const value = (provider ?? "api").trim().toLowerCase();
  return value === "local" || value === "llama" || value === "llamacpp" || value === "llama.cpp" || value === "gguf";
}

export function isCliTranslationProvider(provider?: string | null) {
  const value = (provider ?? "api").trim().toLowerCase();
  return value === "cli" || value === "opencode" || value === "custom";
}

export function fileName(path?: string | null) {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

export function taskSourcePath(task: Pick<TaskRecord, "audio_path" | "file_name" | "srt_path" | "video_path">) {
  return task.video_path || task.audio_path || task.srt_path || task.file_name;
}

export function progressValue(progress: number) {
  return Math.round(Math.max(0, Math.min(1, progress)) * 100);
}

export function progressLabel(progress: number) {
  return `${progressValue(progress)}%`;
}

export function bytesLabel(bytes?: number | null) {
  if (!bytes || bytes <= 0) return "";
  const mib = bytes / (1024 * 1024);
  if (mib >= 1) return `${mib.toFixed(1)} MiB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
}

export function speedLabel(bytesPerSecond?: number | null) {
  const bytes = bytesLabel(bytesPerSecond);
  return bytes ? `${bytes}/s` : "";
}

export function etaLabel(seconds: number | null | undefined, t: TFunction) {
  if (seconds === null || seconds === undefined) return "";
  if (seconds <= 0) return t("time.doneSoon");
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes <= 0) return t("time.remainingSeconds", { seconds: rest });
  return t("time.remainingMinutes", { minutes, seconds: rest });
}

export function downloadMeta(event: ModelDownloadEvent | DependencyInstallEvent | null | undefined, t: TFunction) {
  if (!event) return "";
  const parts: string[] = [];
  const downloaded = bytesLabel(event.downloaded_bytes);
  const total = bytesLabel(event.total_bytes);
  const speed = speedLabel(event.bytes_per_second);
  const eta = etaLabel(event.eta_seconds, t);

  if (downloaded && total) parts.push(`${downloaded} / ${total}`);
  else if (downloaded) parts.push(downloaded);
  if (speed) parts.push(speed);
  if (eta && event.status === "running") parts.push(eta);
  return parts.join(" · ");
}

export function statusText(status: string | undefined, t: TFunction) {
  const labels: Record<string, string> = {
    created: t("status.created"),
    queued: t("status.queued"),
    running: t("status.running"),
    completed: t("status.completed"),
    exported: t("status.exported"),
    failed: t("status.failed"),
    cancelled: t("status.cancelled"),
    interrupted: t("status.interrupted"),
  };
  return status ? labels[status] ?? status : t("status.pending");
}

export function stageText(stage: string | undefined, t: TFunction) {
  const labels: Record<string, string> = {
    created: t("stage.created"),
    transcribe: t("stage.transcribe"),
    extracting: t("stage.extracting"),
    transcribing: t("stage.transcribing"),
    "source-srt": t("stage.sourceSrt"),
    "source-ready": t("stage.sourceReady"),
    "preparing-audio": t("stage.preparingAudio"),
    "preparing-translation": t("stage.preparingTranslation"),
    "translate-shards": t("stage.translateShards"),
    "translate-shard": t("stage.translateShard"),
    "render-translated-srt": t("stage.renderTranslatedSrt"),
    exporting: t("stage.exporting"),
    exported: t("stage.exported"),
    completed: t("stage.completed"),
    failed: t("stage.failed"),
    cancelled: t("stage.cancelled"),
    interrupted: t("stage.interrupted"),
  };
  return stage ? labels[stage] ?? stage : t("status.pending");
}

export function isTranslateStage(stage?: string) {
  return Boolean(
    stage &&
      (stage.includes("translate") ||
        stage.includes("translation") ||
        stage === "render-translated-srt"),
  );
}

export function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function taskBusy(task: TaskRecord) {
  return task.status === "queued" || task.status === "running";
}

export function hasPartialTranslationProgress(task: TaskRecord) {
  const completed = task.translation_completed_count ?? 0;
  return completed > 0;
}

function hasConfiguredText(value?: string | null) {
  return Boolean(value?.trim());
}

export function operationRequirementIssues(
  task: TaskRecord,
  operation: TaskOperation,
  context: OperationReadinessContext,
): OperationRequirementIssue[] {
  if (taskBusy(task)) return ["taskBusy"];

  const issues: OperationRequirementIssue[] = [];

  if (operation === "transcribe") {
    if (task.source_type !== "video" && task.source_type !== "audio") issues.push("unsupportedSource");
    if (normalizeAsrConfig(task.settings.asr).engine === "whisper-cpp") {
      if (!hasConfiguredText(task.settings.whisper_model_path)) issues.push("missingWhisperModel");
      if (!context.environmentReady) issues.push("missingEnvironment");
    } else {
      issues.push(...asrConfigurationIssues(task.settings.asr));
      if (!(context.ffmpegReady ?? context.environmentReady)) issues.push("missingFfmpeg");
    }
    return issues;
  }

  if (operation === "translate" || operation === "resume_translate") {
    if (!hasConfiguredText(task.source_srt_path)) issues.push("missingSourceSubtitles");
    if (operation === "resume_translate" && !hasPartialTranslationProgress(task)) {
      issues.push("missingTranslationProgress");
    }
    const provider = task.settings.translation_provider ?? "api";
    if (isCliTranslationProvider(provider)) {
      if (!hasConfiguredText(task.settings.translation_cli_command)) issues.push("missingCliCommand");
      const tool = (task.settings.translation_cli_tool ?? "opencode").trim().toLowerCase();
      if (tool !== "custom" && !hasConfiguredText(task.settings.translation_cli_model)) {
        issues.push("missingCliModel");
      }
      return issues;
    }
    if (isLocalTranslationProvider(provider)) {
      if (!hasConfiguredText(task.settings.translation_local_model_path)) issues.push("missingLocalTranslationModel");
      if (!context.llamaReady) issues.push("missingLocalTranslationEngine");
      return issues;
    }
    if (!hasConfiguredText(task.settings.base_url)) issues.push("missingBaseUrl");
    if (!hasConfiguredText(task.settings.model)) issues.push("missingTranslationModel");
    if (!context.hasApiCredential) issues.push("missingApiKey");
    return issues;
  }

  if (!hasConfiguredText(task.source_srt_path)) issues.push("missingSourceSubtitles");
  return issues;
}

export function canRunOperation(task: TaskRecord, operation: TaskOperation, context: OperationReadinessContext) {
  return operationRequirementIssues(task, operation, context).length === 0;
}

export function operationRequirementIssueLabel(issue: OperationRequirementIssue, t: TFunction) {
  const labels: Record<OperationRequirementIssue, string> = {
    unsupportedAsrEngine: t("requirement.unsupportedAsrEngine"),
    unsupportedAsrDevice: t("requirement.unsupportedAsrDevice"),
    missingAsrPython: t("requirement.missingAsrPython"),
    missingAsrModel: t("requirement.missingAsrModel"),
    missingAsrAligner: t("requirement.missingAsrAligner"),
    missingFfmpeg: t("requirement.missingFfmpeg"),
    taskBusy: t("requirement.taskBusy"),
    unsupportedSource: t("requirement.unsupportedSource"),
    missingSourceSubtitles: t("requirement.missingSourceSubtitles"),
    missingTranslationProgress: t("requirement.missingTranslationProgress"),
    missingWhisperModel: t("requirement.missingWhisperModel"),
    missingEnvironment: t("requirement.missingEnvironment"),
    missingBaseUrl: t("requirement.missingBaseUrl"),
    missingTranslationModel: t("requirement.missingTranslationModel"),
    missingApiKey: t("requirement.missingApiKey"),
    missingCliCommand: t("requirement.missingCliCommand"),
    missingCliModel: t("requirement.missingCliModel"),
    missingLocalTranslationModel: t("requirement.missingLocalTranslationModel"),
    missingLocalTranslationEngine: t("requirement.missingLocalTranslationEngine"),
  };
  return labels[issue];
}

export function operationRequirementSummary(issues: OperationRequirementIssue[], t: TFunction) {
  return issues.map((issue) => operationRequirementIssueLabel(issue, t)).join(t("requirement.separator"));
}

export function formattedTime(seconds: number, locale: Locale) {
  if (!seconds) return "—";
  return new Date(seconds * 1000).toLocaleString(locale);
}

export function operationLabel(operation: TaskOperation, t: TFunction) {
  if (operation === "transcribe") return t("operation.transcribe");
  if (operation === "translate") return t("operation.translate");
  if (operation === "resume_translate") return t("operation.resumeTranslate");
  return t("operation.export");
}
