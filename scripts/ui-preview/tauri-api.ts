/** QA only. Synthetic data and memory-only edits; all native/network actions are blocked. */
import { defaultSettings } from "@/config";
import type {
  AsrBackendStatus, AsrConfig, DownloadStatus, EnvironmentState, QueueSettings,
  SettingsState, SourceSubtitleEdit, SourceSubtitleSegment, SubtitlePreview,
  TaskOperation, TaskRecord, TaskSettingsSnapshot, TranslationCliStatus,
} from "@/types";
import type { AsrComponentCatalog, AsrComponentStatus, AsrComponentsSnapshot, AsrInstallConsent } from "@/lib/asr-components";
import rawCatalog from "../../src-tauri/resources/asr/catalog.json";
import { emitFixture } from "./events";

const scenario = new URLSearchParams(location.search).get("scenario") || "completed";
const clone = <T,>(value: T): T => structuredClone(value);
const blocked = (operation: string): never => { throw new Error(`Developer fixture blocks ${operation}. No files, downloads, credentials, external requests or native actions were used.`); };
let settings: SettingsState = {
  ...clone(defaultSettings), whisper_model_path: "C:\\LumaFixture\\models\\ggml-large-v3-turbo.bin",
  has_api_key: true, api_key_scopes: ["api|https://api.openai.com"], legacy_api_key_available: false,
};
let queueSettings: QueueSettings = { max_concurrency: 2, auto_start_next: false };
const environment: EnvironmentState = {
  ffmpeg_path: "C:\\LumaFixture\\bin\\ffmpeg.exe", whisper_path: "C:\\LumaFixture\\bin\\whisper-cli.exe",
  llama_path: "C:\\LumaFixture\\bin\\llama-cli.exe", llama_backend: "cpu", gpu_name: null, cuda_driver: null,
  resource_dir: "C:\\LumaFixture\\resources", config_dir: "C:\\LumaFixture\\config",
  sidecar_dir: "C:\\LumaFixture\\bin", model_dir: "C:\\LumaFixture\\models",
  downloaded_model_files: ["ggml-large-v3-turbo.bin"], downloaded_translation_model_files: [],
};

const prose = [
  ["Good design begins with listening. Before we draw anything, we ask people about their day.", "好的设计始于倾听。在动手之前，我们先了解人们的一天。", 5400],
  ["What are they trying to do? And what gets in their way?", "他们想完成什么？又是什么阻碍了他们？", 4200],
  ["Sometimes the most useful change is removing a step, rather than adding another feature.", "有时，最有用的改进是减少一个步骤，而不是增加一项功能。", 5800],
  ["That is where our first prototype started.", "我们的第一个原型就从这里开始。", 3200],
  ["We brought it back to the same people. This time, we watched quietly while they used it.", "我们把原型带回给同一批人。这一次，我们安静地观察他们如何使用。", 5700],
  ["The small pauses told us more than the answers to our questions.", "那些短暂的停顿，比问题的答案更能说明问题。", 4500],
  ["A clear interface should help you focus on your work, then get out of the way.", "清晰的界面应该帮助你专注于工作，然后退到幕后。", 4800],
] as const;
let position = 0;
const sourceSegments: SourceSubtitleSegment[] = Array.from({ length: 125 }, (_, index) => {
  const [text, , duration] = prose[index % prose.length];
  const cue = { id: index + 1, start_ms: position, end_ms: position + duration, text };
  position += duration;
  return cue;
});
const translatedSegments = sourceSegments.map((cue, index) => ({ ...cue, text: prose[index % prose.length][1] }));
const timestamp = (ms: number) => `${String(Math.floor(ms / 3600000)).padStart(2, "0")}:${String(Math.floor(ms / 60000) % 60).padStart(2, "0")}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")},${String(ms % 1000).padStart(3, "0")}`;
const toSrt = (segments: SourceSubtitleSegment[]) => segments.map(cue => `${cue.id}\n${timestamp(cue.start_ms)} --> ${timestamp(cue.end_ms)}\n${cue.text}`).join("\n\n");
let preview: SubtitlePreview = {
  source_srt: scenario === "empty" ? "" : toSrt(sourceSegments),
  source_segments: scenario === "empty" ? [] : sourceSegments,
  translated_srt: scenario === "empty" ? "" : toSrt(translatedSegments),
  translated_segments: scenario === "empty" ? [] : translatedSegments,
  source_file_name: "Designing for people.srt", translated_file_name: "Designing for people.zh-CN.srt",
};
const taskSettings = (): TaskSettingsSnapshot => {
  const { has_api_key: _hasKey, api_key_scopes: _scopes, legacy_api_key_available: _legacy, bind_legacy_api_key: _bind, ...rest } = settings;
  return { ...clone(rest), output_dir: "C:\\LumaFixture\\exports" };
};
const baseTask: TaskRecord = {
  id: "fixture", source_type: "video", video_path: "C:\\LumaFixture\\Designing for people.mp4",
  file_name: scenario === "long" ? "An exceptionally long interview filename with production notes and a complete multilingual subtitle review - 最终校对版本.mp4" : "Designing for people.mp4",
  status: scenario === "running" ? "running" : scenario === "failed" ? "failed" : "completed",
  stage: scenario === "running" ? "translate-shard" : "completed",
  message: scenario === "running" ? "正在翻译字幕，已完成 56 / 125 条" : scenario === "long" ? "Long task progress message / Detailed processing information that must stay readable and reachable. ".repeat(10) : "字幕已生成，可以校对或导出",
  progress: scenario === "running" ? 0.45 : 1,
  settings: taskSettings(), source_srt_path: scenario === "empty" ? null : "C:\\LumaFixture\\source.srt",
  translated_srt_path: scenario === "empty" ? null : "C:\\LumaFixture\\translated.srt",
  source_file_name: preview.source_file_name, translated_file_name: preview.translated_file_name,
  output_dir: "C:\\LumaFixture\\exports", segment_count: scenario === "empty" ? 0 : 125,
  translation_completed_count: scenario === "running" ? 56 : scenario === "empty" ? 0 : 125,
  result_revision: 1,
  error: scenario === "failed" ? "示例错误：长错误信息必须自动换行，并且保留重新尝试操作。" : scenario === "long" ? `Long unbroken diagnostic: ${"fixture-error-token-".repeat(70)}` : null,
  created_at: 1791877200, updated_at: 1791877500,
};
const tasks: TaskRecord[] = [
  baseTask,
  { ...clone(baseTask), id: "fixture-running", file_name: "Studio interview.wav", source_type: "audio", video_path: null, audio_path: "C:\\LumaFixture\\Studio interview.wav", status: "running", stage: "transcribing", progress: 0.42, message: "正在转写音频…", translated_srt_path: null, error: null },
  { ...clone(baseTask), id: "fixture-queued", file_name: "Workshop highlights.mp4", status: "queued", stage: "created", progress: 0, message: "等待处理", source_srt_path: null, translated_srt_path: null, error: null },
  { ...clone(baseTask), id: "fixture-failed", file_name: "Product walkthrough.mp4", status: "failed", stage: "translate-shard", progress: 0.68, message: "翻译未完成，可以重试", error: "Synthetic translation failure; no external request was sent." },
  { ...clone(baseTask), id: "fixture-cancelled", file_name: "Field notes.srt", source_type: "srt", video_path: null, srt_path: "C:\\LumaFixture\\Field notes.srt", status: "cancelled", stage: "cancelled", progress: 0.3, message: "已取消", error: null },
];
const findTask = (id: string) => {
  const task = tasks.find(item => item.id === id);
  if (!task) throw new Error("Synthetic task was not found.");
  return task;
};
const publishTask = (task: TaskRecord) => { emitFixture("task-updated", clone(task)); return clone(task); };

export async function listTasks() { return clone(tasks); }
export async function getTask(id: string): Promise<TaskRecord> { if (scenario === "loading") return new Promise(() => {}); return clone(findTask(id)); }
export async function getTaskLogs(_id: string) { return ["[fixture] Synthetic media imported.", "[fixture] Source subtitles generated: 125 cues.", "[fixture] Translation generated. No network or native work was performed."]; }
export async function subtitlePreview(_id: string) { return clone(preview); }
export async function loadSettings() { return clone(settings); }
export async function saveSettings(payload: SettingsState & { api_key: string }) {
  if (payload.api_key.trim() || payload.bind_legacy_api_key) return blocked("credential entry or binding");
  const { api_key: _key, ...next } = payload;
  settings = clone(next);
  return clone(settings);
}
export async function loadQueueSettings() { return clone(queueSettings); }
export async function saveQueueSettings(maxConcurrency: number, autoStartNext: boolean) { queueSettings = { max_concurrency: maxConcurrency, auto_start_next: autoStartNext }; return clone(queueSettings); }
export async function checkEnvironment() { return clone(environment); }
export async function downloadStatus(): Promise<DownloadStatus> { return { model: null, dependency: null }; }
export async function checkAsrBackend(_config: AsrConfig): Promise<AsrBackendStatus> { return { ready: true, backend: "synthetic-whisper-cpp", device: "cpu", model_bytes: 1_500_000_000, aligner_bytes: 0, total_bytes: 1_500_000_000, estimated_weight_memory_bytes: 1_500_000_000, capabilities: { offline: true, word_timestamps: false, supported_languages: null }, warnings: [] }; }
export async function releaseAsrBackend(): Promise<boolean> { return blocked("native model release"); }
export async function checkTranslationCli(_command: string, _tool: string): Promise<TranslationCliStatus> { return { available: true, path: "C:\\LumaFixture\\fixture-cli.exe", version: "fixture-only" }; }
export async function listTranslationCliModels(_command: string) { return ["fixture/model-a", "fixture/model-b"]; }
export async function applyCurrentSettingsToTask(id: string) { const task = findTask(id); task.settings = taskSettings(); return publishTask(task); }
export async function updateTaskSettings(id: string, next: Omit<TaskSettingsSnapshot, "output_dir">) { const task = findTask(id); task.settings = { ...task.settings, ...clone(next) }; return publishTask(task); }

function applyEdits(segments: SourceSubtitleSegment[], edits: SourceSubtitleEdit[]) {
  const editsById = new Map(edits.map(edit => [edit.id, edit.text]));
  return segments.map(cue => ({ ...cue, text: editsById.get(cue.id) ?? cue.text }));
}
export async function saveSourceSubtitles(id: string, originalSourceSrt: string, edits: SourceSubtitleEdit[]) {
  if (originalSourceSrt !== preview.source_srt) throw new Error("Synthetic source changed; reload the editor.");
  preview.source_segments = applyEdits(preview.source_segments, edits);
  preview.source_srt = toSrt(preview.source_segments);
  const task = findTask(id); task.result_revision = (task.result_revision ?? 0) + 1;
  return publishTask(task);
}
export async function saveTranslatedSubtitles(id: string, originalSourceSrt: string, originalTranslatedSrt: string, edits: SourceSubtitleEdit[]) {
  if (originalSourceSrt !== preview.source_srt || originalTranslatedSrt !== preview.translated_srt) throw new Error("Synthetic subtitles changed; reload the editor.");
  preview.translated_segments = applyEdits(preview.translated_segments ?? [], edits);
  preview.translated_srt = toSrt(preview.translated_segments);
  const task = findTask(id); task.result_revision = (task.result_revision ?? 0) + 1;
  return publishTask(task);
}

export async function selectOutputDir(): Promise<string | null> { return blocked("file picking"); }
export const selectVideo = selectOutputDir;
export const selectAudio = selectOutputDir;
export const selectSrt = selectOutputDir;
export const selectWhisperModel = selectOutputDir;
export const selectTranslationModel = selectOutputDir;
export async function createVideoTask(_request: unknown): Promise<TaskRecord> { return blocked("task import"); }
export const createAudioTask = createVideoTask;
export const createSrtTask = createVideoTask;
export async function runTaskOperation(_id: string, _operation: TaskOperation) { return blocked("processing or export"); }
export async function runTaskOperations(_ids: string[], _operation: TaskOperation) { return blocked("batch processing"); }
export async function cancelTask(_id: string) { return blocked("native task cancellation"); }
export async function deleteTask(_id: string) { return blocked("task deletion"); }
export async function openPath(_path: string) { return blocked("opening native paths"); }
export async function downloadWhisperModel(_presetId: string): Promise<string> { return blocked("model download"); }
export const downloadTranslationModel = downloadWhisperModel;
export async function installDependencies(): Promise<string[]> { return blocked("dependency installation"); }
export async function installLlamaCpp(): Promise<string> { return blocked("local translation installation"); }

const raw = rawCatalog as unknown as AsrComponentCatalog;
const catalog: AsrComponentCatalog = { ...raw, platform: "windows-x64", runtimes: raw.runtimes.map(runtime => ({ ...runtime, plan_sha256: "0".repeat(64) })) };
export async function asrComponentCatalog() { return clone(catalog); }
export async function asrComponentStatus(): Promise<AsrComponentsSnapshot> { return { components: [...catalog.runtimes, ...catalog.models].map(component => ({ id: component.id, kind: "platform" in component ? "runtime" : "model", state: "not_installed", version: null, path: null, python_path: null, installed_bytes: 0, cached_bytes: 0, error: null })), consents: [], operation: null }; }
export async function installAsrComponent(_id: string, _requestId: string, _consent?: AsrInstallConsent): Promise<AsrComponentStatus> { return blocked("ASR installation and consent persistence"); }
export const repairAsrComponent = installAsrComponent;
export async function removeAsrComponent(_id: string, _requestId: string): Promise<AsrComponentStatus> { return blocked("ASR removal"); }
export async function cancelAsrComponent(_requestId: string): Promise<boolean> { return blocked("ASR cancellation"); }
