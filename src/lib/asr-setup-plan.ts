import { isAbsoluteLocalPath } from "@/lib/asr-config";
import {
  componentDownloadBytes, hasMatchingRuntimeConsent, runtimeAcknowledgement,
  type AsrComponentCatalog, type AsrComponentStatus, type AsrInstallConsent,
  type AsrModelComponent, type AsrRecordedConsent, type AsrRuntimeComponent,
} from "@/lib/asr-components";

export type AsrSetupSelection = {
  engine: string;
  runtime?: AsrRuntimeComponent;
  model?: AsrModelComponent;
  aligner?: AsrModelComponent;
};
export type AsrSetupStep = Readonly<{
  component: AsrRuntimeComponent | AsrModelComponent;
  action: "install" | "repair";
  downloadBytes: number;
  consentRequired: boolean;
  consent?: AsrInstallConsent;
}>;
export type AsrSetupPlan = Readonly<{
  key: string;
  engine: string;
  runtime: AsrRuntimeComponent;
  model: AsrModelComponent;
  aligner?: AsrModelComponent;
  selected: readonly (AsrRuntimeComponent | AsrModelComponent)[];
  steps: readonly AsrSetupStep[];
  downloadBytes: number;
  readyCount: number;
}>;
export type AsrSetupInventory = {
  catalog: AsrComponentCatalog;
  components: AsrComponentStatus[];
  consents: AsrRecordedConsent[];
};

// Exact metadata, including the full displayed terms, sources and byte counts.
// This deliberately excludes inventory so an unchanged selection can be retried.
export function asrSetupSelectionKey({ engine, runtime, model, aligner }: AsrSetupSelection) {
  return JSON.stringify({ engine, runtime, model, aligner: engine === "qwen3-asr" ? aligner : undefined });
}

function freezeCopy<T>(value: T): T {
  const copy = JSON.parse(JSON.stringify(value)) as T;
  const freeze = (item: unknown) => {
    if (item && typeof item === "object") {
      Object.values(item).forEach(freeze);
      Object.freeze(item);
    }
  };
  freeze(copy);
  return copy;
}

export function isAsrSetupComponentReady(component: AsrRuntimeComponent | AsrModelComponent, statuses: AsrComponentStatus[]) {
  const kind = "files" in component ? "model" : "runtime";
  const status = statuses.find((item) => item.id === component.id && item.kind === kind);
  if (status?.state !== "installed" || status.version !== component.version || status.error) return false;
  const path = kind === "runtime" ? status.python_path : status.path;
  return !!path && isAbsoluteLocalPath(path);
}

function hasDownloadableArtifacts(component: AsrRuntimeComponent | AsrModelComponent) {
  const artifacts = "files" in component ? component.files : component.recipe
    ? [component.recipe.python, ...component.recipe.wheels] : component.archive ? [component.archive] : [];
  const bytes = componentDownloadBytes(component);
  return Number.isSafeInteger(bytes) && bytes > 0 && artifacts.length > 0 && artifacts.every((artifact) => Number.isSafeInteger(artifact.bytes) && artifact.bytes > 0
    && /^https:\/\//.test(artifact.url) && /^[a-f0-9]{64}$/i.test(artifact.sha256));
}

export function createAsrSetupPlan(input: AsrSetupSelection & {
  components: AsrComponentStatus[];
  consents: AsrRecordedConsent[];
  verified: boolean;
}): AsrSetupPlan | null {
  const { engine, runtime, model, components, consents, verified } = input;
  const aligner = engine === "qwen3-asr" ? input.aligner : undefined;
  if (!verified || !runtime || !model || runtime.engine !== engine || model.engine !== engine
    || runtime.backend !== model.backend || model.role !== "model"
    || (engine === "qwen3-asr" && (!aligner || aligner.engine !== engine || aligner.backend !== runtime.backend || aligner.role !== "aligner"))) return null;
  const selected = [runtime, model, ...(aligner ? [aligner] : [])];
  if (new Set(selected.map((component) => component.id)).size !== selected.length
    || selected.some((component) => component.unavailable_reason)) return null;
  const steps: AsrSetupStep[] = [];
  for (const component of selected) {
    if (isAsrSetupComponentReady(component, components)) continue;
    if (!hasDownloadableArtifacts(component)) return null;
    const kind = "files" in component ? "model" : "runtime";
    const status = components.find((item) => item.id === component.id && item.kind === kind);
    const action = status && status.state !== "not_installed" ? "repair" : "install";
    const consent = kind === "runtime" && runtime.recipe ? runtimeAcknowledgement(runtime) : undefined;
    // A recipe with missing/invalid exact terms must never become installable.
    if (kind === "runtime" && runtime.recipe && !consent) return null;
    const consentRequired = !!consent && !(action === "repair" && hasMatchingRuntimeConsent(runtime, consents));
    steps.push({ component, action, downloadBytes: componentDownloadBytes(component), consentRequired, ...(consent ? { consent } : {}) });
  }
  const downloadBytes = steps.reduce((sum, step) => sum + step.downloadBytes, 0);
  if (!Number.isSafeInteger(downloadBytes)) return null;
  // Raw cache sizes are not verified reusable bytes. Only the installer may
  // validate/reuse cache files; exclude ready components, never guess cache hits.
  return freezeCopy({ key: asrSetupSelectionKey({ engine, runtime, model, aligner }), engine, runtime, model, ...(aligner ? { aligner } : {}), selected, steps,
    downloadBytes, readyCount: selected.length - steps.length });
}

export function replanAsrSetup(plan: AsrSetupPlan, inventory: AsrSetupInventory): AsrSetupPlan | null {
  const runtime = inventory.catalog.runtimes.find((item) => item.id === plan.runtime.id);
  if (runtime?.platform !== inventory.catalog.platform) return null;
  const refreshed = createAsrSetupPlan({ engine: plan.engine, runtime,
    model: inventory.catalog.models.find((item) => item.id === plan.model.id),
    aligner: plan.aligner ? inventory.catalog.models.find((item) => item.id === plan.aligner!.id) : undefined,
    components: inventory.components, consents: inventory.consents, verified: true });
  return refreshed?.key === plan.key ? refreshed : null;
}

export function matchesAsrSetupConsent(expected: AsrInstallConsent, supplied?: AsrInstallConsent) {
  const termsKey = (consent: AsrInstallConsent) => JSON.stringify(consent.acknowledged_terms.map(({ id, version, sha256 }) => JSON.stringify([id, version, sha256])).sort());
  return !!supplied && expected.plan_sha256 === supplied.plan_sha256 && termsKey(expected) === termsKey(supplied);
}
