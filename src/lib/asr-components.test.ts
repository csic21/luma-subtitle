import { describe, expect, it } from "vitest";
import { defaultAsrConfig } from "@/config";
import { componentDownloadBytes, componentProgressPercent, managedAsrConfig, type AsrComponentProgress, type AsrComponentStatus, type AsrModelComponent, type AsrRuntimeComponent } from "./asr-components";

export const runtime: AsrRuntimeComponent = { id: "whisper-runtime", version: "1", label: "Whisper CPU", platform: "windows-x64", engine: "whisper-accelerated", backend: "faster-whisper", device: "cpu", license: "MIT", license_url: "https://example.test/LICENSE", installed_bytes: 200, max_files: 10, entrypoint: "python.exe", archive: { url: "https://example.test/runtime.zip", bytes: 100, sha256: "a".repeat(64) } };
export const model: AsrModelComponent = { id: "whisper-model", version: "1", label: "Whisper Turbo", engine: "whisper-accelerated", backend: "faster-whisper", role: "model", license: "MIT", license_url: "https://example.test/LICENSE", installed_bytes: 500, files: [{ path: "model.bin", url: "https://example.test/model.bin", bytes: 500, sha256: "b".repeat(64) }] };
export const statuses: AsrComponentStatus[] = [
  { id: runtime.id, kind: "runtime", state: "installed", version: "1", path: "/private/runtime", python_path: "/private/runtime/python", installed_bytes: 200, error: null },
  { id: model.id, kind: "model", state: "installed", version: "1", path: "/private/model", python_path: null, installed_bytes: 500, error: null },
];
const oldConfig = { ...defaultAsrConfig, engine: "whisper-accelerated", python_path: "/custom/python", model_path: "/external/model", aligner_path: "/external/aligner", device: "cuda" as const };

describe("managed ASR selections", () => {
  it("uses installed returned paths and the selected runtime device without changing the original config", () => {
    expect(managedAsrConfig(oldConfig, runtime, model, undefined, statuses)).toEqual({ ...oldConfig, python_path: "/private/runtime/python", model_path: "/private/model", aligner_path: "", device: "cpu" });
    expect(oldConfig.python_path).toBe("/custom/python"); expect(oldConfig.model_path).toBe("/external/model"); expect(oldConfig.device).toBe("cuda");
  });
  it("does not reinterpret model formats, engine IDs, roles, or unverified versions", () => {
    expect(managedAsrConfig(oldConfig, runtime, { ...model, backend: "mlx-whisper" }, undefined, statuses)).toBeNull();
    expect(managedAsrConfig({ ...oldConfig, engine: "whisper-cpp" }, runtime, model, undefined, statuses)).toBeNull();
    expect(managedAsrConfig(oldConfig, runtime, { ...model, role: "aligner" }, undefined, statuses)).toBeNull();
    expect(managedAsrConfig(oldConfig, { ...runtime, version: "2" }, model, undefined, statuses)).toBeNull();
  });
  it("requires all selected components installed with absolute local paths", () => {
    expect(managedAsrConfig(oldConfig, runtime, model, undefined, [])).toBeNull();
    expect(managedAsrConfig(oldConfig, runtime, model, undefined, statuses.map((item) => ({ ...item, state: "damaged" })))).toBeNull();
    expect(managedAsrConfig(oldConfig, runtime, model, undefined, [{ ...statuses[0], python_path: "python" }, statuses[1]])).toBeNull();
  });
  it("requires matching Qwen ASR and aligner before enabling a complete selection", () => {
    const qwenRuntime = { ...runtime, engine: "qwen3-asr", backend: "qwen-asr" };
    const qwenModel = { ...model, engine: "qwen3-asr", backend: "qwen-asr" };
    const aligner = { ...qwenModel, id: "aligner", role: "aligner" as const };
    const config = { ...oldConfig, engine: "qwen3-asr" };
    expect(managedAsrConfig(config, qwenRuntime, qwenModel, undefined, statuses)).toBeNull();
    expect(managedAsrConfig(config, qwenRuntime, qwenModel, aligner, statuses)).toBeNull();
    expect(managedAsrConfig(config, qwenRuntime, qwenModel, aligner, [...statuses, { ...statuses[1], id: "aligner", path: "/private/aligner" }])?.aligner_path).toBe("/private/aligner");
  });
  it("calculates actual declared download bytes separately from installed bytes", () => {
    expect(componentDownloadBytes(runtime)).toBe(100); expect(componentDownloadBytes({ ...runtime, archive: null })).toBe(0);
    expect(componentDownloadBytes({ ...model, files: [...model.files, { ...model.files[0], path: "config.json", bytes: 17 }] })).toBe(517);
  });
  it("only displays byte progress while downloading, without fake extraction or validation percentages", () => {
    const progress: AsrComponentProgress = { request_id: "id", component_id: "runtime", phase: "downloading", downloaded_bytes: 33, total_bytes: 100, message: "" };
    expect(componentProgressPercent(progress)).toBe(33); expect(componentProgressPercent({ ...progress, downloaded_bytes: 101 })).toBe(100);
    expect(componentProgressPercent({ ...progress, total_bytes: 0 })).toBeNull(); expect(componentProgressPercent({ ...progress, phase: "testing" })).toBeNull();
  });
});
