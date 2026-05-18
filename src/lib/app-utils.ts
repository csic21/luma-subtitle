import type { Locale } from "@/i18n";
import type { DependencyInstallEvent, ModelDownloadEvent, TaskOperation, TaskRecord, TFunction } from "@/types";

export type OperationRequirementIssue =
  | "taskBusy"
  | "unsupportedSource"
  | "missingSourceSubtitles"
  | "missingWhisperModel"
  | "missingEnvironment"
  | "missingBaseUrl"
  | "missingTranslationModel"
  | "missingApiKey";

export type OperationReadinessContext = {
  environmentReady: boolean;
  hasApiCredential: boolean;
};

export function fileName(path?: string | null) {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
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
    if (task.source_type !== "video") issues.push("unsupportedSource");
    if (!hasConfiguredText(task.settings.whisper_model_path)) issues.push("missingWhisperModel");
    if (!context.environmentReady) issues.push("missingEnvironment");
    return issues;
  }

  if (operation === "translate") {
    if (!hasConfiguredText(task.source_srt_path)) issues.push("missingSourceSubtitles");
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
    taskBusy: t("requirement.taskBusy"),
    unsupportedSource: t("requirement.unsupportedSource"),
    missingSourceSubtitles: t("requirement.missingSourceSubtitles"),
    missingWhisperModel: t("requirement.missingWhisperModel"),
    missingEnvironment: t("requirement.missingEnvironment"),
    missingBaseUrl: t("requirement.missingBaseUrl"),
    missingTranslationModel: t("requirement.missingTranslationModel"),
    missingApiKey: t("requirement.missingApiKey"),
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

export function operationLabel(operation: "transcribe" | "translate" | "export", t: TFunction) {
  if (operation === "transcribe") return t("operation.transcribe");
  if (operation === "translate") return t("operation.translate");
  return t("operation.export");
}

