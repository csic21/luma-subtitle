import { isAbsoluteLocalPath } from "@/lib/asr-config";
import type { AsrConfig } from "@/types";

export type AsrTermAcknowledgement = { id: string; version: string; sha256: string };
export type AsrComponentTerm = AsrTermAcknowledgement & { url: string; text: string; raw_sha256?: string | null; source_encoding?: string | null };
export type AsrInstallConsent = { plan_sha256: string; acknowledged_terms: AsrTermAcknowledgement[] };
export type AsrRecordedConsent = { component_id: string; plan_sha256: string; terms: AsrTermAcknowledgement[] };
export type AsrRuntimeRecipe = {
  schema: 1;
  python: { url: string; bytes: number; sha256: string; installed_bytes: number; max_files: number; entrypoint: string; site_packages: string; pip_version: string };
  wheels: { name: string; version: string; filename: string; url: string; bytes: number; sha256: string; installed_bytes: number; max_files: number }[];
  terms: AsrComponentTerm[];
};

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
  recipe?: AsrRuntimeRecipe | null;
  plan_sha256?: string | null;
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
  cached_bytes?: number;
  error: string | null;
};
export type AsrComponentProgress = {
  request_id: string;
  component_id: string;
  phase: "preparing" | "downloading" | "verifying" | "extracting" | "assembling" | "testing" | "activating" | "removing" | "complete" | "cancelled" | "error";
  downloaded_bytes: number;
  total_bytes: number;
  message: string;
};
export type AsrComponentsSnapshot = {
  components: AsrComponentStatus[];
  consents?: AsrRecordedConsent[];
  operation: AsrComponentProgress | null;
};
export type AsrComponentAction = "install" | "repair" | "remove";

export function componentDownloadBytes(component: AsrRuntimeComponent | AsrModelComponent) {
  if ("files" in component) return component.files.reduce((total, file) => total + file.bytes, 0);
  if (component.recipe) return component.recipe.python.bytes + component.recipe.wheels.reduce((total, wheel) => total + wheel.bytes, 0);
  return component.archive?.bytes ?? 0;
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

export function componentSources(component: AsrRuntimeComponent | AsrModelComponent): string[] {
  if ("files" in component) return [...new Set(component.files.map((file) => file.url))];
  if (component.recipe) return [component.recipe.python.url, ...component.recipe.wheels.map((wheel) => wheel.url)];
  return component.archive ? [component.archive.url] : [];
}

export function runtimeAcknowledgement(runtime: AsrRuntimeComponent): AsrInstallConsent | null {
  if (!runtime.recipe || !runtime.plan_sha256 || !/^[a-f0-9]{64}$/i.test(runtime.plan_sha256)) return null;
  const terms = runtime.recipe.terms;
  if (terms.length === 0 || new Set(terms.map((term) => term.id)).size !== terms.length || terms.some((term) => !term.id || !term.version || !term.text || !/^https:\/\//.test(term.url) || !/^[a-f0-9]{64}$/i.test(term.sha256))) return null;
  return { plan_sha256: runtime.plan_sha256, acknowledged_terms: runtime.recipe.terms.map(({ id, version, sha256 }) => ({ id, version, sha256 })) };
}

function exactTermsKey(terms: AsrTermAcknowledgement[]) {
  return JSON.stringify(terms.map(({ id, version, sha256 }) => JSON.stringify([id, version, sha256])).sort());
}

export function hasMatchingRuntimeConsent(runtime: AsrRuntimeComponent, consents: AsrRecordedConsent[]) {
  const required = runtimeAcknowledgement(runtime);
  return !!required && consents.some((consent) => consent.component_id === runtime.id && consent.plan_sha256 === required.plan_sha256 && exactTermsKey(consent.terms) === exactTermsKey(required.acknowledged_terms));
}

// Includes displayed content as well as its backend fingerprint. A stale dialog
// must not acknowledge a changed component, source, size, or term set.
export function runtimeConfirmationKey(runtime: AsrRuntimeComponent) {
  return JSON.stringify(runtime);
}
