/** Developer fixture only: every operation below changes memory, never files. */
import rawCatalog from "../../src-tauri/resources/asr/catalog.json";
import { componentDownloadBytes, type AsrComponentCatalog, type AsrComponentProgress, type AsrComponentStatus, type AsrInstallConsent, type AsrComponentsSnapshot } from "@/lib/asr-components";
import { emitFixtureProgress } from "./events";
const params = new URLSearchParams(location.search);
export const scenario = params.get("scenario") ?? "runtime-installed";
const raw = rawCatalog as unknown as AsrComponentCatalog;
// The native API adds plan hashes. A deterministic fixture-only fingerprint makes
// the real dialog valid without representing a backend receipt or accepting terms.
export const catalog: AsrComponentCatalog = { ...raw, platform: "windows-x64", runtimes: raw.runtimes.map((runtime) => ({ ...runtime, plan_sha256: "0".repeat(64) })) };
const all = [...catalog.runtimes, ...catalog.models];
const installedIds = new Set(scenario === "fresh" ? [] : scenario === "ready" ? ["qwen3-asr-cpu-windows-x64", "qwen3-asr-0-6b", "qwen3-forced-aligner-0-6b"] : ["qwen3-asr-cpu-windows-x64"]);
const status = (id: string): AsrComponentStatus => {
  const component = all.find((item) => item.id === id)!;
  const installed = installedIds.has(id);
  const runtime = "platform" in component;
  return { id, kind: runtime ? "runtime" : "model", state: installed ? "installed" : "not_installed", version: installed ? component.version : null, path: installed ? `C:\\LumaFixture\\${id}` : null, python_path: installed && runtime ? `C:\\LumaFixture\\${id}\\python.exe` : null, installed_bytes: installed ? (runtime ? 1_204_621_461 : component.installed_bytes) : 0, cached_bytes: installed && runtime ? componentDownloadBytes(component) : 0, error: null };
};
type Pending = { id: string; requestId: string; resolve: (status: AsrComponentStatus) => void; reject: (error: Error) => void; progress: AsrComponentProgress };
let pending: Pending | undefined;
export const calls: string[] = [];
export const subscribe = (callback: () => void) => { listeners.add(callback); return () => { listeners.delete(callback); }; };
const listeners = new Set<() => void>();
let revision = 0;
export const getRevision = () => revision;
const notify = () => { revision += 1; for (const callback of listeners) callback(); };
export const activeId = () => pending?.id ?? null;
export async function asrComponentCatalog() { return catalog; }
export async function asrComponentStatus(): Promise<AsrComponentsSnapshot> { return { components: all.map((item) => status(item.id)), consents: [], operation: pending?.progress ?? null }; }
export function installAsrComponent(id: string, requestId: string, _consent?: AsrInstallConsent): Promise<AsrComponentStatus> {
  if (pending) return Promise.reject(new Error("Fixture invariant violated: simultaneous installs"));
  // Even accidental interaction with real runtime acceptance cannot install it.
  if (catalog.runtimes.some((runtime) => runtime.id === id)) return Promise.reject(new Error("Developer fixture blocks runtime installation and consent persistence."));
  calls.push(`start:${id}`);
  const total = componentDownloadBytes(all.find((item) => item.id === id)!);
  return new Promise((resolve, reject) => {
    const progress: AsrComponentProgress = { request_id: requestId, component_id: id, phase: "downloading", downloaded_bytes: Math.round(total * 0.32), total_bytes: total, message: "In-memory simulated download; no network." };
    pending = { id, requestId, resolve, reject, progress };
    emitFixtureProgress(progress);
    notify();
  });
}
export const repairAsrComponent = installAsrComponent;
export async function removeAsrComponent() { throw new Error("Developer fixture does not remove components."); }
export async function cancelAsrComponent(requestId: string) {
  if (!pending || pending.requestId !== requestId) return false;
  const current = pending; pending = undefined;
  calls.push(`cancel:${current.id}`);
  emitFixtureProgress({ ...current.progress, phase: "cancelled" });
  current.reject(new Error("Cancelled by fixture control")); notify(); return true;
}
export function finishCurrent() {
  if (!pending) return;
  const current = pending; pending = undefined; installedIds.add(current.id);
  calls.push(`complete:${current.id}`);
  emitFixtureProgress({ ...current.progress, phase: "complete", downloaded_bytes: current.progress.total_bytes });
  current.resolve(status(current.id)); notify();
}
export function failCurrent() {
  if (!pending) return;
  const current = pending; pending = undefined;
  calls.push(`error:${current.id}`);
  emitFixtureProgress({ ...current.progress, phase: "error" });
  current.reject(new Error("Simulated network interruption. No files were downloaded.")); notify();
}
