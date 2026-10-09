import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useTaskDetailState } from "./use-task-detail-state";
import * as api from "@/lib/tauri-api";
import { defaultSettings } from "@/config";
import type { SubtitlePreview, TaskRecord, TFunction } from "@/types";

const mocks = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: TaskRecord }) => void>(), resume: () => {} }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, handler) => { mocks.handlers.set(name, handler); return () => mocks.handlers.delete(name); }) }));
vi.mock("./use-app-resume", () => ({ useAppResume: (resume: () => void) => { mocks.resume = resume; } }));
vi.mock("@/lib/tauri-api", () => ({
  applyCurrentSettingsToTask: vi.fn(), cancelTask: vi.fn(), checkEnvironment: vi.fn(), getTask: vi.fn(), getTaskLogs: vi.fn(), loadSettings: vi.fn(),
  openPath: vi.fn(), runTaskOperation: vi.fn(), selectTranslationModel: vi.fn(), selectWhisperModel: vi.fn(), subtitlePreview: vi.fn(), updateTaskSettings: vi.fn(),
}));
const t: TFunction = (key, values) => `${key}${values ? ` ${JSON.stringify(values)}` : ""}`;
const task: TaskRecord = {
  id: "task-1", source_type: "srt", file_name: "clip.srt", status: "completed", stage: "completed", message: "done", progress: 1,
  settings: { ...defaultSettings, target_language: "Chinese", whisper_model_path: "/model.bin", base_url: "https://api.openai.com", model: "model" },
  source_srt_path: "/source.srt", translated_srt_path: "/translated.srt", result_revision: 1, created_at: 1, updated_at: 1,
};
const preview: SubtitlePreview = { source_srt: "source", source_file_name: "source.srt", source_segments: [{ id: 1, start_ms: 0, end_ms: 1000, text: "source" }], translated_srt: "old translation", translated_segments: [{ id: 1, start_ms: 0, end_ms: 1000, text: "old translation" }] };
let state: ReturnType<typeof useTaskDetailState>;
let renderer: ReactTestRenderer | undefined;
function Probe() { state = useTaskDetailState("task-1", t); return null; }
async function mount() { await act(async () => { renderer = create(<Probe />); }); }
async function emit(next: TaskRecord) { await act(async () => { mocks.handlers.get("task-updated")?.({ payload: next }); }); }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }

beforeEach(() => {
  vi.clearAllMocks(); mocks.handlers.clear(); vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  vi.mocked(api.getTask).mockResolvedValue(task);
  vi.mocked(api.getTaskLogs).mockResolvedValue([]);
  vi.mocked(api.subtitlePreview).mockResolvedValue(preview);
  vi.mocked(api.loadSettings).mockResolvedValue({ ...defaultSettings, has_api_key: true });
  vi.mocked(api.checkEnvironment).mockResolvedValue({ ffmpeg_path: "/ffmpeg", whisper_path: "/whisper", llama_path: "/llama", gpu_name: null, cuda_driver: null, resource_dir: "", config_dir: "", sidecar_dir: "", model_dir: "" });
});
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; vi.unstubAllGlobals(); });

describe("task detail behavior", () => {
  it("refreshes same-path subtitle revisions without changing the selected tab", async () => {
    await mount();
    act(() => state.setSubtitleView("parallel"));
    vi.mocked(api.subtitlePreview).mockResolvedValue({ ...preview, translated_srt: "new translation" });
    await emit({ ...task, result_revision: 2 });
    expect(api.subtitlePreview).toHaveBeenCalledTimes(2);
    expect(state.subtitlePreview?.translated_srt).toBe("new translation");
    expect(state.subtitleView).toBe("parallel");
    await emit({ ...task, result_revision: 2, progress: 0.7 });
    expect(api.subtitlePreview).toHaveBeenCalledTimes(2);
  });

  it("refreshes a changed result on resume and resets an invalid translated view", async () => {
    await mount(); act(() => state.setSubtitleView("translated"));
    vi.mocked(api.getTask).mockResolvedValue({ ...task, result_revision: 2, translated_srt_path: null });
    vi.mocked(api.subtitlePreview).mockResolvedValue({ ...preview, translated_srt: null, translated_segments: null });
    await act(async () => { mocks.resume(); });
    expect(api.subtitlePreview).toHaveBeenCalledTimes(2);
    expect(state.subtitleView).toBe("source");
  });

  it("saves a dirty draft before running and ignores duplicate clicks during the save", async () => {
    await mount();
    const saved = { ...task, settings: { ...task.settings, target_language: "English" } };
    act(() => state.setSettingsDraft(saved.settings));
    const save = deferred<TaskRecord>(); vi.mocked(api.updateTaskSettings).mockReturnValue(save.promise);
    let run!: Promise<void>;
    act(() => { run = state.runOperation("translate"); void state.runOperation("translate"); });
    expect(api.updateTaskSettings).toHaveBeenCalledTimes(1);
    expect(api.runTaskOperation).not.toHaveBeenCalled();
    expect(state.commandPending).toBe(true);
    vi.mocked(api.getTask).mockResolvedValue(saved);
    await act(async () => { save.resolve(saved); await run; });
    expect(api.runTaskOperation).toHaveBeenCalledExactlyOnceWith("task-1", "translate");
    expect(state.taskSettingsDirty).toBe(false);
    expect(state.commandPending).toBe(false);
  });

  it("keeps the draft and does not start a task when saving fails", async () => {
    await mount();
    act(() => state.setSettingsDraft({ ...task.settings, target_language: "English" }));
    vi.mocked(api.updateTaskSettings).mockRejectedValue(new Error("disk full"));
    await act(async () => { await state.runOperation("translate"); });
    expect(api.runTaskOperation).not.toHaveBeenCalled();
    expect(state.taskSettingsDirty).toBe(true);
    expect(state.notice).toContain("notice.settingsSaveFailed");
    expect(state.notice).toContain("disk full");
    expect(state.commandPending).toBe(false);
  });

  it("does not save an unchanged draft and shows saved translations after direct correction", async () => {
    await mount();
    await act(async () => { await state.runOperation("translate"); });
    expect(api.updateTaskSettings).not.toHaveBeenCalled();
    await act(async () => { await state.sourceSubtitlesSaved({ ...task, result_revision: 2 }, "translated"); });
    expect(state.subtitleView).toBe("translated");
    expect(state.notice).toContain("notice.translatedSubtitlesSaved");
  });
});
