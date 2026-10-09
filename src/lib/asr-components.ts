import { isAbsoluteLocalPath } from "@/lib/asr-config";
import type { AsrConfig } from "@/types";

export type AsrComponentFile = { path: string; url: string; bytes: number; sha256: string };
export type AsrComponentInfo = {
  id: string;
  version: string;
  label: string;
  engine: string;
  backend: string;
  license: string;
  license_url: string;
  installed_bytes: number;
  unavailable_reason?: string | null;
};
export type AsrRuntimeComponent = AsrComponentInfo & {
  platform: string;
  min_os_version?: string | null;
  device: AsrConfig["device"];
  max_files: number;
  entrypoint: string;
  archive?: { url: string; bytes: number; sha256: string } | null;
};
export type AsrModelComponent = AsrComponentInfo & {
  role: "model" | "aligner";
  files: AsrComponentFile[];
};
export type AsrComponentCatalog = {
  schema: 1;
  platform: string;
  runtimes: AsrRuntimeComponent[];
  models: AsrModelComponent[];
};
export type AsrComponentStatus = {
  id: string;
  kind: "runtime" | "model";
  state: "not_installed" | "installed" | "damaged";
  version: string | null;
  path: string | null;
  python_path: string | null;
  installed_bytes: number;
  error: string | null;
};
export type AsrComponentProgress = {
  request_id: string;
  component_id: string;
  phase: "preparing" | "downloading" | "verifying" | "extracting" | "testing" | "activating" | "removing" | "complete" | "cancelled" | "error";
  downloaded_bytes: number;
  total_bytes: number;
  message: string;
};
export type AsrComponentsSnapshot = {
  components: AsrComponentStatus[];
  operation: AsrComponentProgress | null;
};
export type AsrComponentAction = "install" | "repair" | "remove";

export function componentDownloadBytes(component: AsrRuntimeComponent | AsrModelComponent) {
  return "files" in component ? component.files.reduce((total, file) => total + file.bytes, 0) : component.archive?.bytes ?? 0;
}

export function isTerminalComponentPhase(phase: AsrComponentProgress["phase"]) {
  return phase === "complete" || phase === "cancelled" || phase === "error";
}

export function componentProgressPercent(progress: AsrComponentProgress) {
  if (progress.phase !== "downloading" || progress.total_bytes <= 0) return null;
  return Math.max(0, Math.min(100, Math.floor(progress.downloaded_bytes / progress.total_bytes * 100)));
}

// Only exact, verified managed selections produce configuration paths. External paths
// are never guessed, migrated, deleted, or reinterpreted as managed components.
export function managedAsrConfig(
  current: AsrConfig,
  runtime: AsrRuntimeComponent | undefined,
  model: AsrModelComponent | undefined,
  aligner: AsrModelComponent | undefined,
  statuses: AsrComponentStatus[],
): AsrConfig | null {
  if (!runtime || !model || runtime.engine !== current.engine || model.engine !== current.engine || model.backend !== runtime.backend || model.role !== "model") return null;
  const runtimeStatus = statuses.find((item) => item.id === runtime.id && item.kind === "runtime" && item.state === "installed" && item.version === runtime.version);
  const modelStatus = statuses.find((item) => item.id === model.id && item.kind === "model" && item.state === "installed" && item.version === model.version);
  if (!runtimeStatus?.python_path || !modelStatus?.path || !isAbsoluteLocalPath(runtimeStatus.python_path) || !isAbsoluteLocalPath(modelStatus.path)) return null;
  let alignerPath = "";
  if (current.engine === "qwen3-asr") {
    if (!aligner || aligner.role !== "aligner" || aligner.engine !== current.engine || aligner.backend !== runtime.backend) return null;
    const status = statuses.find((item) => item.id === aligner.id && item.kind === "model" && item.state === "installed" && item.version === aligner.version);
    if (!status?.path || !isAbsoluteLocalPath(status.path)) return null;
    alignerPath = status.path;
  }
  return { ...current, python_path: runtimeStatus.python_path, model_path: modelStatus.path, aligner_path: alignerPath, device: runtime.device };
}
