import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";

import { defaultSettings, translationLocalModelPresets } from "@/config";
import * as api from "@/lib/tauri-api";
import type { DependencyInstallEvent, DownloadStatus, ModelDownloadEvent, SettingsState } from "@/types";
import { useSettingsPageState } from "./use-settings-page-state";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/lib/updater", () => ({
  checkAppUpdate: vi.fn(async () => null),
  currentAppVersion: vi.fn(async () => "1.2.2"),
  downloadAndInstallAppUpdate: vi.fn(),
  relaunchApp: vi.fn(),
}));
vi.mock("@/lib/tauri-api", () => ({
  checkEnvironment: vi.fn(),
  downloadStatus: vi.fn(),
  downloadTranslationModel: vi.fn(),
  downloadWhisperModel: vi.fn(),
  installDependencies: vi.fn(),
  installLlamaCpp: vi.fn(),
  loadSettings: vi.fn(),
  openPath: vi.fn(),
  saveSettings: vi.fn(),
  selectTranslationModel: vi.fn(),
  selectWhisperModel: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

const translate = (key: string) => key;
const preset = translationLocalModelPresets[0];
const modelRunning: ModelDownloadEvent = {
  preset_id: preset.id,
  file_name: preset.fileName,
  status: "running",
  message: "Downloading model",
  progress: 0.4,
};
const dependencyRunning: DependencyInstallEvent = {
  item: "llama.cpp",
  status: "running",
  message: "Installing engine",
  progress: 0.3,
};
const modelCompleted: ModelDownloadEvent = { ...modelRunning, status: "completed", progress: 1 };
const dependencyCompleted: DependencyInstallEvent = { ...dependencyRunning, status: "completed", progress: 1 };
const listeners = new Map<string, (event: { payload: ModelDownloadEvent | DependencyInstallEvent }) => void>();
let state: ReturnType<typeof useSettingsPageState>;
let renderer: ReactTestRenderer | undefined;

function Probe() {
  state = useSettingsPageState(translate);
  return null;
}

async function mount(settings: SettingsState = defaultSettings) {
  vi.mocked(api.loadSettings).mockResolvedValue(settings);
  await act(async () => { renderer = create(<Probe />); });
}

async function emit(name: string, payload: ModelDownloadEvent | DependencyInstallEvent) {
  await act(async () => { listeners.get(name)!({ payload }); });
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {}, setInterval, clearInterval });
  listeners.clear();
  vi.mocked(listen).mockImplementation(async (name, handler) => {
    listeners.set(name, handler as (event: { payload: ModelDownloadEvent | DependencyInstallEvent }) => void);
    return () => { listeners.delete(name); };
  });
  vi.mocked(api.checkEnvironment).mockResolvedValue({
    ffmpeg_path: "/ffmpeg", whisper_path: null, llama_path: "/llama", gpu_name: null,
    cuda_driver: null, resource_dir: "", config_dir: "", sidecar_dir: "", model_dir: "",
  });
  vi.mocked(api.downloadStatus).mockResolvedValue({});
  vi.mocked(api.installLlamaCpp).mockResolvedValue("/llama");
  vi.mocked(api.downloadTranslationModel).mockResolvedValue("/model.gguf");
  vi.mocked(api.saveSettings).mockImplementation(async (settings) => settings);
});

afterEach(() => {
  act(() => { renderer?.unmount(); });
  renderer = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("settings download status ordering", () => {
  it.each(["completed", "failed"] as const)("preserves %s events when an older running poll finishes later", async (status) => {
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await mount();

    const model = { ...modelRunning, status, error: status === "failed" ? "Model failed" : null };
    const dependency = { ...dependencyRunning, status, error: status === "failed" ? "Engine failed" : null };
    await emit("model-download-event", model);
    await emit("dependency-install-event", dependency);
    await act(async () => { poll.resolve({ model: modelRunning, dependency: dependencyRunning }); });

    expect(state.modelDownload).toEqual(model);
    expect(state.dependencyInstall).toEqual(dependency);
    expect(state.modelDownloading).toBe(false);
    expect(state.dependencyInstalling).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("accepts an unaffected status channel without replacing a newer event", async () => {
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await mount();
    await emit("model-download-event", modelCompleted);
    await act(async () => { poll.resolve({ model: modelRunning, dependency: dependencyCompleted }); });
    expect(state.modelDownload).toEqual(modelCompleted);
    expect(state.dependencyInstall).toEqual(dependencyCompleted);
  });

  it("starts polling an unaffected running channel after another channel receives an event", async () => {
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await mount();
    await emit("model-download-event", modelCompleted);
    await act(async () => { poll.resolve({ model: modelRunning, dependency: dependencyRunning }); });
    expect(state.modelDownload).toEqual(modelCompleted);
    expect(state.dependencyInstall).toEqual(dependencyRunning);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("ignores an older poll that finishes after a newer terminal poll", async () => {
    await mount();
    await emit("dependency-install-event", dependencyRunning);
    const older = deferred<DownloadStatus>();
    const newer = deferred<DownloadStatus>();
    vi.mocked(api.downloadStatus).mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    await act(async () => { newer.resolve({ dependency: dependencyCompleted }); });
    await act(async () => { older.resolve({ dependency: dependencyRunning }); });
    expect(state.dependencyInstall).toEqual(dependencyCompleted);
    expect(state.dependencyInstalling).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("does not stop newer polling when an older idle snapshot resolves", async () => {
    await mount();
    await emit("dependency-install-event", dependencyRunning);
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    await emit("model-download-event", modelRunning);
    await act(async () => { poll.resolve({ dependency: dependencyCompleted }); });
    expect(state.modelDownloading).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("does not restart polling when an initial request resolves after unmount", async () => {
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await mount();
    act(() => { renderer!.unmount(); });
    renderer = undefined;
    await act(async () => { poll.resolve({ dependency: dependencyRunning }); });
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("local translation install locking", () => {
  it("coalesces repeated clicks before React rerenders and ignores old terminal events for unlocking", async () => {
    const install = deferred<string>();
    vi.mocked(api.installLlamaCpp).mockReturnValueOnce(install.promise);
    await mount({ ...defaultSettings, translation_local_model_path: "/existing.gguf" });
    const onClick = state.installLocalTranslation;
    let pending!: Promise<void>;
    await act(async () => {
      pending = onClick();
      await onClick();
    });
    expect(api.installLlamaCpp).toHaveBeenCalledTimes(1);
    await emit("dependency-install-event", dependencyCompleted);
    expect(state.dependencyInstalling).toBe(true);
    await act(async () => { await state.installLocalTranslation(); });
    expect(api.installLlamaCpp).toHaveBeenCalledTimes(1);
    await act(async () => { install.resolve("/llama"); await pending; });
    expect(state.dependencyInstalling).toBe(false);
    expect(state.settings.translation_provider).toBe("local");
    expect(api.downloadTranslationModel).not.toHaveBeenCalled();
  });

  it("keeps the install locked through the model download and settings save", async () => {
    const download = deferred<string>();
    const save = deferred<SettingsState>();
    vi.mocked(api.downloadTranslationModel).mockReturnValueOnce(download.promise);
    vi.mocked(api.saveSettings).mockReturnValueOnce(save.promise);
    await mount({ ...defaultSettings, translation_local_model_path: "" });
    let pending!: Promise<void>;
    await act(async () => { pending = state.installLocalTranslation(); });
    expect(api.downloadTranslationModel).toHaveBeenCalledTimes(1);
    expect(state.dependencyInstalling).toBe(true);
    await emit("model-download-event", modelCompleted);
    await act(async () => { await state.installLocalTranslation(); });
    expect(api.installLlamaCpp).toHaveBeenCalledTimes(1);
    await act(async () => { download.resolve("/model.gguf"); });
    expect(api.saveSettings).toHaveBeenCalledTimes(1);
    expect(state.dependencyInstalling).toBe(true);
    await act(async () => { await state.installLocalTranslation(); });
    expect(api.installLlamaCpp).toHaveBeenCalledTimes(1);
    await act(async () => {
      save.resolve({ ...defaultSettings, translation_provider: "local", translation_local_model_path: "/model.gguf" });
      await pending;
    });
    expect(state.dependencyInstalling).toBe(false);
    expect(state.modelDownloading).toBe(false);
    expect(state.settings.translation_local_model_path).toBe("/model.gguf");
  });

  it("does not restore running status from a poll begun before command completion", async () => {
    const install = deferred<string>();
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.installLlamaCpp).mockReturnValueOnce(install.promise);
    await mount({ ...defaultSettings, translation_local_model_path: "/existing.gguf" });
    let pending!: Promise<void>;
    await act(async () => { pending = state.installLocalTranslation(); });
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    await act(async () => { install.resolve("/llama"); await pending; });
    await act(async () => { poll.resolve({ dependency: dependencyRunning }); });
    expect(state.dependencyInstall?.status).toBe("completed");
    expect(state.dependencyInstalling).toBe(false);
  });

  it("releases the lock after failure so a retry can complete", async () => {
    vi.mocked(api.installLlamaCpp).mockRejectedValueOnce(new Error("Engine install failed"));
    await mount({ ...defaultSettings, translation_local_model_path: "/existing.gguf" });
    await act(async () => { await state.installLocalTranslation(); });
    expect(state.dependencyInstall?.status).toBe("failed");
    expect(state.dependencyInstalling).toBe(false);
    expect(state.notice).toContain("Engine install failed");
    await act(async () => { await state.installLocalTranslation(); });
    expect(api.installLlamaCpp).toHaveBeenCalledTimes(2);
    expect(state.dependencyInstall?.status).toBe("completed");
    expect(state.dependencyInstalling).toBe(false);
  });
});

describe("settled commands and delayed events", () => {
  it.each(["completed", "failed"] as const)("does not resurrect a %s engine install from delayed events", async (status) => {
    if (status === "failed") vi.mocked(api.installLlamaCpp).mockRejectedValueOnce(new Error("Engine failed"));
    await mount({ ...defaultSettings, translation_local_model_path: "/existing.gguf" });
    await act(async () => { await state.installLocalTranslation(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    const settled = state.dependencyInstall;
    await emit("dependency-install-event", dependencyRunning);
    await emit("dependency-install-event", dependencyCompleted);
    expect(state.dependencyInstall).toEqual(settled);
    expect(state.dependencyInstall?.status).toBe(status);
    expect(state.dependencyInstalling).toBe(false);
    expect(vi.getTimerCount()).toBe(0);

    const retry = deferred<string>();
    vi.mocked(api.installLlamaCpp).mockReturnValueOnce(retry.promise);
    let pending!: Promise<void>;
    await act(async () => { pending = state.installLocalTranslation(); });
    await emit("dependency-install-event", dependencyRunning);
    expect(state.dependencyInstall).toEqual(dependencyRunning);
    expect(state.dependencyInstalling).toBe(true);
    await act(async () => { retry.resolve("/llama"); await pending; });
    expect(api.installLlamaCpp).toHaveBeenCalledTimes(2);
  });

  it("locks a direct model download through stale terminal snapshots, events, and settings save", async () => {
    const download = deferred<string>();
    const save = deferred<SettingsState>();
    vi.mocked(api.downloadTranslationModel).mockReturnValueOnce(download.promise);
    vi.mocked(api.saveSettings).mockReturnValueOnce(save.promise);
    await mount();
    const onClick = state.downloadTranslationPreset;
    let pending!: Promise<void>;
    await act(async () => { pending = onClick(); await onClick(); });
    expect(api.downloadTranslationModel).toHaveBeenCalledTimes(1);
    vi.mocked(api.downloadStatus).mockResolvedValueOnce({ model: modelCompleted });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    await emit("model-download-event", modelCompleted);
    expect(state.modelDownloading).toBe(true);
    await act(async () => { await state.downloadTranslationPreset(); });
    expect(api.downloadTranslationModel).toHaveBeenCalledTimes(1);
    await act(async () => { download.resolve("/model.gguf"); });
    expect(state.modelDownloading).toBe(true);
    await act(async () => { await state.downloadTranslationPreset(); });
    expect(api.downloadTranslationModel).toHaveBeenCalledTimes(1);
    await act(async () => {
      save.resolve({ ...defaultSettings, translation_provider: "local", translation_local_model_path: "/model.gguf" });
      await pending;
    });
    const settled = state.modelDownload;
    await emit("model-download-event", modelRunning);
    expect(state.modelDownload).toEqual(settled);
    expect(state.modelDownloading).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("preserves a model command result against an older running poll and permits a retry", async () => {
    const download = deferred<string>();
    const poll = deferred<DownloadStatus>();
    vi.mocked(api.downloadTranslationModel).mockReturnValueOnce(download.promise);
    await mount();
    let pending!: Promise<void>;
    await act(async () => { pending = state.downloadTranslationPreset(); });
    vi.mocked(api.downloadStatus).mockReturnValueOnce(poll.promise);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    await act(async () => { download.reject(new Error("Model failed")); await pending; });
    await act(async () => { poll.resolve({ model: modelRunning }); });
    await emit("model-download-event", modelCompleted);
    expect(state.modelDownload?.status).toBe("failed");
    expect(state.modelDownloading).toBe(false);
    await act(async () => { await state.downloadTranslationPreset(); });
    expect(api.downloadTranslationModel).toHaveBeenCalledTimes(2);
    expect(state.modelDownload?.status).toBe("completed");
    expect(state.modelDownloading).toBe(false);
  });
});
