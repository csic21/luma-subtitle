import { describe, expect, it } from "vitest";
import { defaultAsrConfig } from "@/config";
import { componentDownloadBytes, componentProgressPercent, componentSources, hasMatchingRuntimeConsent, managedAsrConfig, runtimeAcknowledgement, runtimeConfirmationKey, type AsrComponentProgress, type AsrComponentStatus, type AsrModelComponent, type AsrRuntimeComponent } from "./asr-components";

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

const recipeRuntime: AsrRuntimeComponent = { ...runtime, archive: null, plan_sha256: "1".repeat(64), recipe: {
  schema: 1,
  python: { url: "https://example.test/python.zip", bytes: 100, sha256: "2".repeat(64), installed_bytes: 200, max_files: 5, entrypoint: "python.exe", site_packages: "Lib/site-packages", pip_version: "25.3" },
  wheels: [{ name: "engine", version: "1", filename: "engine.whl", url: "https://example.test/engine.whl", bytes: 20, sha256: "3".repeat(64), installed_bytes: 40, max_files: 2 }],
  terms: [
    { id: "vendor-license", version: "2026-01", sha256: "4".repeat(64), url: "https://example.test/terms", text: "Exact vendor terms." },
    { id: "other-license", version: "2", sha256: "5".repeat(64), url: "https://example.test/other", text: "Exact other terms." },
  ],
} };

describe("exact runtime consent", () => {
  it("sums direct upstream inputs and keeps source URLs visible", () => {
    expect(componentDownloadBytes(recipeRuntime)).toBe(120);
    expect(componentSources(recipeRuntime)).toEqual(["https://example.test/python.zip", "https://example.test/engine.whl"]);
  });
  it("includes the exact optional Microsoft package in declared sources and bytes", () => {
    const withCrt: AsrRuntimeComponent = { ...recipeRuntime, recipe: { ...recipeRuntime.recipe!, windows_crt: "msvc-14.44.35211-x64" } };
    expect(componentDownloadBytes(withCrt)).toBe(120 + 25_635_768);
    expect(componentSources(withCrt).slice(-1)[0]).toBe("https://download.visualstudio.microsoft.com/download/pr/73aabf2e-9532-4f68-99f7-3247081a619c/CC0FF0EB1DC3F5188AE6300FAEF32BF5BEEBA4BDD6E8E445A9184072096B713B/VC_redist.x64.exe");
    expect(runtimeAcknowledgement(withCrt)).not.toBeNull();
    expect(runtimeConfirmationKey(withCrt)).not.toBe(runtimeConfirmationKey(recipeRuntime));
    expect(runtimeAcknowledgement({ ...withCrt, platform: "macos-arm64" })).toBeNull();
    expect(runtimeAcknowledgement({ ...withCrt, recipe: { ...withCrt.recipe!, windows_crt: "unknown" as "msvc-14.44.35211-x64" } })).toBeNull();
    expect(componentDownloadBytes({ ...recipeRuntime, recipe: { ...recipeRuntime.recipe!, windows_crt: null } })).toBe(120);
  });
  it("constructs only the complete exact plan and term identity tuples", () => {
    expect(runtimeAcknowledgement(recipeRuntime)).toEqual({ plan_sha256: "1".repeat(64), acknowledged_terms: [
      { id: "vendor-license", version: "2026-01", sha256: "4".repeat(64) },
      { id: "other-license", version: "2", sha256: "5".repeat(64) },
    ] });
    expect(runtimeAcknowledgement({ ...recipeRuntime, plan_sha256: null })).toBeNull();
    expect(runtimeAcknowledgement({ ...recipeRuntime, recipe: { ...recipeRuntime.recipe!, terms: [] } })).toBeNull();
    expect(runtimeAcknowledgement({ ...recipeRuntime, recipe: { ...recipeRuntime.recipe!, terms: [{ ...recipeRuntime.recipe!.terms[0], text: "" }] } })).toBeNull();
  });
  it("reuses only the same component, same plan, and complete unique exact term set", () => {
    const terms = runtimeAcknowledgement(recipeRuntime)!.acknowledged_terms;
    const receipt = { component_id: recipeRuntime.id, plan_sha256: recipeRuntime.plan_sha256!, terms };
    expect(hasMatchingRuntimeConsent(recipeRuntime, [receipt])).toBe(true);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [{ ...receipt, terms: [...terms].reverse() }])).toBe(true);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [])).toBe(false);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [{ ...receipt, component_id: "other-runtime" }])).toBe(false);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [{ ...receipt, plan_sha256: "6".repeat(64) }])).toBe(false);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [{ ...receipt, terms: [terms[0]] }])).toBe(false);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [{ ...receipt, terms: [terms[0], terms[0]] }])).toBe(false);
    expect(hasMatchingRuntimeConsent(recipeRuntime, [{ ...receipt, terms: [terms[0], { ...terms[1], sha256: "6".repeat(64) }] }])).toBe(false);
  });
  it("invalidates a displayed plan when sources, bytes, version, or terms change", () => {
    const key = runtimeConfirmationKey(recipeRuntime);
    expect(runtimeConfirmationKey({ ...recipeRuntime, version: "2" })).not.toBe(key);
    expect(runtimeConfirmationKey({ ...recipeRuntime, installed_bytes: 999 })).not.toBe(key);
    expect(runtimeConfirmationKey({ ...recipeRuntime, recipe: { ...recipeRuntime.recipe!, python: { ...recipeRuntime.recipe!.python, url: "https://example.test/changed" } } })).not.toBe(key);
  });
});
