import { defaultAsrConfig, defaultSettings } from "@/config";
import type { AsrConfig, SettingsState } from "@/types";

export const asrEngines = ["whisper-cpp", "whisper-accelerated", "qwen3-asr"] as const;

export function normalizeAsrConfig(config?: Partial<AsrConfig> | null): AsrConfig {
  return {
    engine: config?.engine ?? defaultAsrConfig.engine,
    python_path: config?.python_path ?? defaultAsrConfig.python_path,
    model_path: config?.model_path ?? defaultAsrConfig.model_path,
    aligner_path: config?.aligner_path ?? defaultAsrConfig.aligner_path,
    device: config?.device ?? defaultAsrConfig.device,
  };
}

export function normalizeSettings(settings: Partial<SettingsState>): SettingsState {
  return { ...defaultSettings, ...settings, asr: normalizeAsrConfig(settings.asr) };
}

export function asrConfigKey(config?: Partial<AsrConfig> | null) {
  return JSON.stringify(normalizeAsrConfig(config));
}

export function asrConfigsEqual(left?: AsrConfig, right?: AsrConfig) {
  return asrConfigKey(left) === asrConfigKey(right);
}

export function isAbsoluteLocalPath(value: string) {
  // No shell expansion, model repository IDs, or relative paths are accepted.
  return /^(\/|[a-zA-Z]:[\\/]|\\\\[^\\]+\\[^\\]+)/.test(value.trim());
}

export type AsrConfigurationIssue =
  | "unsupportedAsrEngine"
  | "unsupportedAsrDevice"
  | "missingAsrPython"
  | "missingAsrModel"
  | "missingAsrAligner";

export function asrConfigurationIssues(config?: AsrConfig): AsrConfigurationIssue[] {
  const asr = normalizeAsrConfig(config);
  if (!asrEngines.some((engine) => engine === asr.engine)) return ["unsupportedAsrEngine"];
  if (asr.engine === "whisper-cpp") return [];
  const issues: AsrConfigurationIssue[] = [];
  if (!isAbsoluteLocalPath(asr.python_path)) issues.push("missingAsrPython");
  if (!isAbsoluteLocalPath(asr.model_path)) issues.push("missingAsrModel");
  if (asr.engine === "qwen3-asr" && !isAbsoluteLocalPath(asr.aligner_path)) issues.push("missingAsrAligner");
  if (!["auto", "cpu", "cuda", "metal"].includes(asr.device) || (asr.engine === "qwen3-asr" && asr.device === "metal")) {
    issues.push("unsupportedAsrDevice");
  }
  return issues;
}
