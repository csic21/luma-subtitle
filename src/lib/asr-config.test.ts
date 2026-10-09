import { describe, expect, it } from "vitest";
import { defaultAsrConfig, defaultSettings, whisperModelPresets } from "@/config";
import { asrConfigurationIssues, isAbsoluteLocalPath, normalizeAsrConfig, normalizeSettings } from "./asr-config";
import { normalizeTaskSettings, shouldReplaceTaskSettingsDraft, taskCreatePayload, taskSettingsEqual, taskSettingsUpdatePayload } from "./task-data";
import { operationRequirementIssues } from "./app-utils";
import type { AsrConfig, TaskRecord } from "@/types";

const qwen: AsrConfig = { engine: "qwen3-asr", python_path: "/venv/bin/python", model_path: "/models/qwen", aligner_path: "/models/aligner", device: "cpu" };
const legacySettings = { ...defaultSettings, asr: undefined, whisper_model_path: "/models/ggml-large-v3-turbo-q5_0.bin" };
const context = { environmentReady: false, ffmpegReady: true, hasApiCredential: false, llamaReady: false };
function task(asr?: AsrConfig): TaskRecord {
  return { id: "1", source_type: "video", file_name: "clip.mp4", status: "created", stage: "created", message: "", progress: 0, created_at: 1, updated_at: 1, settings: { ...legacySettings, whisper_model_path: "", asr } };
}

describe("ASR settings compatibility", () => {
  it("migrates old global/task JSON without changing the legacy model or requiring Python", () => {
    expect(normalizeSettings(legacySettings).asr).toEqual(defaultAsrConfig);
    expect(normalizeTaskSettings(legacySettings).asr).toEqual(defaultAsrConfig);
    expect(normalizeSettings(legacySettings).whisper_model_path).toBe(legacySettings.whisper_model_path);
    expect(asrConfigurationIssues()).toEqual([]);
    expect(whisperModelPresets.some((preset) => preset.id === "large-v3-turbo-q5_0")).toBe(true);
  });

  it("deep merges partial nested settings, including a missing/null nested value", () => {
    expect(normalizeAsrConfig({ engine: "qwen3-asr", model_path: "/models/qwen" })).toEqual({ ...defaultAsrConfig, engine: "qwen3-asr", model_path: "/models/qwen" });
    expect(normalizeAsrConfig(null)).toEqual(defaultAsrConfig);
    expect(normalizeAsrConfig()).not.toBe(defaultAsrConfig);
  });

  it("preserves unknown engines rather than silently choosing Whisper", () => {
    const unknown = normalizeAsrConfig({ engine: "future-engine" });
    expect(unknown.engine).toBe("future-engine");
    expect(asrConfigurationIssues(unknown)).toEqual(["unsupportedAsrEngine"]);
    expect(operationRequirementIssues(task(unknown), "transcribe", context)).toEqual(["unsupportedAsrEngine"]);
  });

  it("propagates nested config through create and update payloads while preserving the legacy path", () => {
    const settings = { ...legacySettings, asr: qwen };
    for (const kind of ["audio", "video", "srt"] as const) {
      const payload = taskCreatePayload(settings, `/clip.${kind}`, kind);
      expect(payload.asr).toEqual(qwen);
      expect(payload.asr).not.toBe(qwen);
      expect(payload.whisper_model_path).toBe(legacySettings.whisper_model_path);
    }
    expect(taskSettingsUpdatePayload(settings).asr).toEqual(qwen);
    expect(taskCreatePayload(legacySettings, "/clip.mp4").asr).toEqual(defaultAsrConfig);
    expect(taskSettingsUpdatePayload(legacySettings).asr).toEqual(defaultAsrConfig);
  });

  it("compares nested values, treats defaults as equal, and preserves dirty drafts", () => {
    expect(taskSettingsEqual(legacySettings, { ...legacySettings, asr: { ...defaultAsrConfig } })).toBe(true);
    const settings = { ...legacySettings, asr: qwen };
    expect(taskSettingsEqual(settings, { ...settings, asr: { ...qwen } })).toBe(true);
    for (const field of ["engine", "python_path", "model_path", "aligner_path", "device"] as const) {
      const draft = { ...settings, asr: { ...qwen, [field]: `${qwen[field]}-changed` } };
      expect(taskSettingsEqual(settings, draft)).toBe(false);
      expect(shouldReplaceTaskSettingsDraft({ id: "1", settings }, draft, { id: "1" })).toBe(false);
    }
  });
});

describe("optional ASR readiness", () => {
  it("requires explicit local paths, rejects relative paths and online repository IDs", () => {
    for (const value of ["python", "./venv/python", "~/models", "Qwen/Qwen3-ASR-0.6B", "https://example.com/model"]) {
      expect(isAbsoluteLocalPath(value)).toBe(false);
    }
    for (const value of ["/opt/python", "C:\\venv\\Scripts\\python.exe", "D:/models/qwen", "\\\\server\\models\\qwen"]) {
      expect(isAbsoluteLocalPath(value)).toBe(true);
    }
    expect(asrConfigurationIssues({ ...qwen, python_path: "python", model_path: "Qwen/model", aligner_path: "" })).toEqual(["missingAsrPython", "missingAsrModel", "missingAsrAligner"]);
  });

  it("allows configured optional engines without a legacy model or whisper.cpp executable", () => {
    expect(operationRequirementIssues(task(qwen), "transcribe", context)).toEqual([]);
    expect(operationRequirementIssues(task({ ...qwen, engine: "whisper-accelerated", aligner_path: "" }), "transcribe", context)).toEqual([]);
    expect(operationRequirementIssues(task(qwen), "transcribe", { ...context, ffmpegReady: false })).toEqual(["missingFfmpeg"]);
  });

  it("requires Qwen alignment and rejects Qwen Metal without silently changing device", () => {
    expect(asrConfigurationIssues({ ...qwen, aligner_path: "", device: "metal" })).toEqual(["missingAsrAligner", "unsupportedAsrDevice"]);
    expect(normalizeAsrConfig({ ...qwen, device: "metal" }).device).toBe("metal");
  });

  it("keeps the old transcription guard and unrelated translation/export behavior", () => {
    expect(operationRequirementIssues(task(), "transcribe", context)).toEqual(["missingWhisperModel", "missingEnvironment"]);
    const sourceTask = { ...task(qwen), source_srt_path: "/source.srt" };
    expect(operationRequirementIssues(sourceTask, "export", { ...context, ffmpegReady: false })).toEqual([]);
    expect(operationRequirementIssues({ ...sourceTask, source_type: "srt" }, "transcribe", context)).toEqual(["unsupportedSource"]);
  });
});
