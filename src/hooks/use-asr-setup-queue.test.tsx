import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAsrSetupQueue, type AsrSetupQueueResult } from "./use-asr-setup-queue";
import type { AsrComponentCancelResult, AsrComponentResult, AsrComponentsRefreshResult } from "./use-asr-components";
import { asrSetupSelectionKey, createAsrSetupPlan } from "@/lib/asr-setup-plan";
import { runtimeAcknowledgement, type AsrComponentCatalog, type AsrComponentStatus, type AsrInstallConsent, type AsrModelComponent, type AsrRecordedConsent, type AsrRuntimeComponent } from "@/lib/asr-components";

const runtime: AsrRuntimeComponent = { id: "runtime", version: "1", label: "Runtime", platform: "windows-x64", engine: "qwen3-asr", backend: "qwen-asr", device: "cpu", license: "MIT", license_url: "https://example.test/license", installed_bytes: 200, max_files: 10, entrypoint: "python.exe", archive: { url: "https://example.test/runtime.zip", bytes: 100, sha256: "a".repeat(64) } };
const model: AsrModelComponent = { id: "model", version: "1", label: "Model", engine: runtime.engine, backend: runtime.backend, role: "model", license: "MIT", license_url: "https://example.test/license", installed_bytes: 500, files: [{ path: "model.bin", url: "https://example.test/model.bin", bytes: 500, sha256: "b".repeat(64) }] };
const aligner: AsrModelComponent = { ...model, id: "aligner", role: "aligner" };
const recipe: AsrRuntimeComponent = { ...runtime, archive: null, plan_sha256: "c".repeat(64), recipe: { schema: 1,
  python: { url: "https://example.test/python.zip", bytes: 100, sha256: "d".repeat(64), installed_bytes: 200, max_files: 10, entrypoint: "python.exe", site_packages: "Lib/site-packages", pip_version: "25" }, wheels: [],
  terms: [{ id: "terms", version: "1", sha256: "e".repeat(64), url: "https://example.test/terms", text: "The full required terms." }] } };
function installed(id: string): AsrComponentStatus { return { id, kind: id === "runtime" ? "runtime" : "model", state: "installed", version: "1", path: `/private/${id}`, python_path: id === "runtime" ? "/private/runtime/python" : null, installed_bytes: 100, error: null }; }
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
let catalog: AsrComponentCatalog;
let components: AsrComponentStatus[];
let consents: AsrRecordedConsent[];
let queue: ReturnType<typeof useAsrSetupQueue>;
let renderer: ReactTestRenderer | undefined;
const controller = { refresh: vi.fn<() => Promise<AsrComponentsRefreshResult>>(), perform: vi.fn<(action: "install" | "repair" | "remove", id: string, consent?: AsrInstallConsent) => Promise<AsrComponentResult>>(), cancel: vi.fn<() => Promise<AsrComponentCancelResult>>() };
const selection = () => ({ engine: runtime.engine, runtime: catalog.runtimes[0], model: catalog.models[0], aligner: catalog.models[1] });
const plan = () => createAsrSetupPlan({ ...selection(), components, consents, verified: true })!;
const snapshot = (): AsrComponentsRefreshResult => ({ status: "success", catalog, components: [...components], consents: [...consents] });
function Probe({ selectionKey }: { selectionKey: string }) { queue = useAsrSetupQueue(controller, selectionKey); return null; }
async function mount() { await act(async () => { renderer = create(<Probe selectionKey={asrSetupSelectionKey(selection())} />); }); }
async function change(selectionKey: string) { await act(async () => { renderer!.update(<Probe selectionKey={selectionKey} />); }); }
async function run(p = plan(), accepted?: Record<string, AsrInstallConsent>) { let result!: AsrSetupQueueResult; await act(async () => { result = await queue.start(p, accepted); }); return result; }
beforeEach(() => {
  vi.resetAllMocks(); catalog = { schema: 1, platform: runtime.platform, runtimes: [runtime], models: [model, aligner] }; components = []; consents = [];
  controller.refresh.mockImplementation(async () => snapshot());
  controller.perform.mockImplementation(async (_action, id) => { const component = installed(id); components = [...components.filter((item) => item.id !== id), component]; return { status: "success", component }; });
  controller.cancel.mockResolvedValue({ status: "accepted" });
});
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; });

describe("safe sequential one-click ASR setup", () => {
  it("installs in dependency order with concurrency one and returns a final fresh verified snapshot", async () => {
    let concurrent = 0; let peak = 0;
    controller.perform.mockImplementation(async (_action, id) => { peak = Math.max(peak, ++concurrent); await Promise.resolve(); components.push(installed(id)); concurrent--; return { status: "success", component: installed(id) }; });
    await mount(); const result = await run();
    expect(result.status).toBe("success"); expect(peak).toBe(1);
    expect(controller.perform.mock.calls.map((call) => call[1])).toEqual(["runtime", "model", "aligner"]);
    expect(controller.refresh).toHaveBeenCalledTimes(2); expect(queue.completedIds).toEqual(["runtime", "model", "aligner"]); expect(queue.currentIndex).toBe(3); expect(queue.busy).toBe(false);
    if (result.status === "success") expect(result.components).toEqual(components);
  });
  it("deduplicates start synchronously before inventory or commands resolve", async () => {
    const pending = deferred<AsrComponentsRefreshResult>(); controller.refresh.mockReturnValueOnce(pending.promise); await mount();
    let first!: Promise<AsrSetupQueueResult>; let duplicate!: Promise<AsrSetupQueueResult>;
    act(() => { first = queue.start(plan()); duplicate = queue.start(plan()); });
    expect(await duplicate).toEqual({ status: "busy" }); expect(controller.refresh).toHaveBeenCalledTimes(1); expect(queue.busy).toBe(true);
    await act(async () => { pending.resolve(snapshot()); await first; }); expect(controller.perform).toHaveBeenCalledTimes(3);
  });
  it("stops on failure, preserves completed work, and retries using a refreshed missing-only plan", async () => {
    controller.perform.mockImplementationOnce(async (_action, id) => { components.push(installed(id)); return { status: "success", component: installed(id) }; }).mockResolvedValueOnce({ status: "error", error: "Checksum mismatch" });
    const preview = plan(); await mount(); expect(await run(preview)).toEqual({ status: "error", error: "Checksum mismatch" });
    expect(controller.perform.mock.calls.map((call) => call[1])).toEqual(["runtime", "model"]); expect(queue.completedIds).toEqual(["runtime"]); expect(queue.error).toBe("Checksum mismatch");
    expect(await run(preview)).toMatchObject({ status: "success" });
    expect(controller.perform.mock.calls.map((call) => call[1])).toEqual(["runtime", "model", "model", "aligner"]);
    expect(queue.plan?.steps.map((step) => step.component.id)).toEqual(["model", "aligner"]);
    expect(queue.error).toBeUndefined();
  });
  it("does not mistake swallowed failure, busy, or cancellation results for success", async () => {
    await mount(); for (const result of [{ status: "error" as const, error: "Failed" }, { status: "busy" as const }, { status: "cancelled" as const }]) {
      controller.perform.mockResolvedValueOnce(result); const before = controller.perform.mock.calls.length;
      expect(await run()).toEqual(result); expect(controller.perform.mock.calls.length).toBe(before + 1);
    }
  });
  it("fails closed when inventory refresh is unavailable", async () => {
    controller.refresh.mockResolvedValue({ status: "error", error: "Inventory unavailable" }); await mount();
    expect(await run()).toEqual({ status: "error", error: "Inventory unavailable" }); expect(controller.perform).not.toHaveBeenCalled();
  });
  it("does not start while a recovered backend operation is running", async () => {
    controller.refresh.mockResolvedValue({ ...snapshot(), status: "success", catalog, components, consents, operation: { request_id: "external", component_id: "runtime", phase: "downloading", downloaded_bytes: 0, total_bytes: 100, message: "" } });
    await mount(); expect(await run()).toEqual({ status: "busy" }); expect(controller.perform).not.toHaveBeenCalled();
  });
  it("waits for final integrity verification and rejects a partial or damaged final installation", async () => {
    const final = deferred<AsrComponentsRefreshResult>(); controller.refresh.mockImplementationOnce(async () => snapshot()).mockReturnValueOnce(final.promise); await mount();
    let request!: Promise<AsrSetupQueueResult>; await act(async () => { request = queue.start(plan()); });
    expect(queue.busy).toBe(true); expect(queue.status).toBe("running");
    components[1] = { ...components[1], state: "damaged" };
    await act(async () => { final.resolve(snapshot()); expect(await request).toEqual({ status: "error", error: "asr.managed.setupVerificationFailed" }); });
    expect(queue.status).toBe("error");
  });
  it("drops steps that become installed before starting without redownloading them", async () => {
    const preview = plan(); components = [installed("runtime"), installed("model")]; await mount();
    expect(await run(preview)).toMatchObject({ status: "success" }); expect(controller.perform.mock.calls.map((call) => call[1])).toEqual(["aligner"]);
  });
  it("rejects new repairs or installs that were absent from the reviewed plan", async () => {
    components = [installed("runtime")]; const preview = plan(); components[0] = { ...components[0], state: "damaged" }; await mount();
    expect(await run(preview)).toEqual({ status: "stale" }); expect(controller.perform).not.toHaveBeenCalled();
  });
  it("rejects catalog, source, byte-size, and term changes before any command", async () => {
    const preview = plan(); catalog = { ...catalog, models: [{ ...model, files: [{ ...model.files[0], bytes: 999 }] }, aligner] }; await mount();
    // Model metadata changes the selection key as well as the refreshed plan.
    expect(await run(preview)).toEqual({ status: "stale" }); expect(controller.perform).not.toHaveBeenCalled();
  });
  it("requires the full exact explicit receipt and forwards it unchanged", async () => {
    catalog.runtimes = [recipe]; const preview = plan(); await mount();
    expect(await run(preview)).toEqual({ status: "error", error: "asr.managed.setupConsentRequired" }); expect(controller.perform).not.toHaveBeenCalled();
    const consent = runtimeAcknowledgement(recipe)!;
    expect(await run(preview, { runtime: { ...consent, acknowledged_terms: [] } })).toEqual({ status: "error", error: "asr.managed.setupConsentRequired" });
    expect(await run(preview, { runtime: consent })).toMatchObject({ status: "success" }); expect(controller.perform).toHaveBeenCalledWith("install", "runtime", consent);
  });
  it("uses only a fresh matching backend receipt for repair and still forwards its exact plan and terms", async () => {
    catalog.runtimes = [recipe]; components = [{ ...installed("runtime"), state: "damaged" }]; const consent = runtimeAcknowledgement(recipe)!;
    consents = [{ component_id: "runtime", plan_sha256: consent.plan_sha256, terms: consent.acknowledged_terms }]; await mount();
    expect(await run()).toMatchObject({ status: "success" }); expect(controller.perform).toHaveBeenCalledWith("repair", "runtime", consent);
  });
  it("rejects revoked backend receipts instead of silently adding a consent requirement", async () => {
    catalog.runtimes = [recipe]; components = [{ ...installed("runtime"), state: "damaged" }]; const consent = runtimeAcknowledgement(recipe)!;
    consents = [{ component_id: "runtime", plan_sha256: consent.plan_sha256, terms: consent.acknowledged_terms }]; const preview = plan(); consents = []; await mount();
    expect(await run(preview)).toEqual({ status: "stale" }); expect(controller.perform).not.toHaveBeenCalled();
  });
  it("copies explicit acceptance before awaiting refresh", async () => {
    catalog.runtimes = [recipe]; const pending = deferred<AsrComponentsRefreshResult>(); controller.refresh.mockReturnValueOnce(pending.promise); await mount();
    const consent = runtimeAcknowledgement(recipe)!; const exact = structuredClone(consent); let request!: Promise<AsrSetupQueueResult>;
    act(() => { request = queue.start(plan(), { runtime: consent }); }); consent.acknowledged_terms.length = 0;
    await act(async () => { pending.resolve(snapshot()); await request; }); expect(controller.perform).toHaveBeenCalledWith("install", "runtime", exact);
  });
  it("latches cancel before the first command while refresh is pending", async () => {
    const pending = deferred<AsrComponentsRefreshResult>(); controller.refresh.mockReturnValueOnce(pending.promise); await mount(); let request!: Promise<AsrSetupQueueResult>;
    act(() => { request = queue.start(plan()); }); await act(async () => { await queue.cancel(); });
    await act(async () => { pending.resolve(snapshot()); expect(await request).toEqual({ status: "cancelled" }); });
    expect(controller.perform).not.toHaveBeenCalled(); expect(queue.cancelling).toBe(false);
  });
  it("cancels the current step only once and never starts the next step", async () => {
    const pending = deferred<AsrComponentResult>(); controller.perform.mockReturnValueOnce(pending.promise); await mount(); let request!: Promise<AsrSetupQueueResult>;
    await act(async () => { request = queue.start(plan()); }); await act(async () => { await queue.cancel(); await queue.cancel(); });
    expect(controller.cancel).toHaveBeenCalledTimes(1); expect(queue.cancelling).toBe(true);
    await act(async () => { pending.resolve({ status: "cancelled" }); expect(await request).toEqual({ status: "cancelled" }); }); expect(controller.perform).toHaveBeenCalledTimes(1);
  });
  it("preserves a completed activation after cancel-too-late but stops the remaining queue", async () => {
    const pending = deferred<AsrComponentResult>(); controller.perform.mockReturnValueOnce(pending.promise);
    controller.cancel.mockResolvedValue({ status: "too_late" }); await mount(); let request!: Promise<AsrSetupQueueResult>;
    await act(async () => { request = queue.start(plan()); }); await act(async () => { await queue.cancel(); });
    expect(queue.finishingCurrent).toBe(true);
    await act(async () => { components.push(installed("runtime")); pending.resolve({ status: "success", component: installed("runtime") }); expect(await request).toEqual({ status: "cancelled" }); });
    expect(queue.completedIds).toEqual(["runtime"]); expect(controller.perform).toHaveBeenCalledTimes(1);
  });
  it("honors cancellation exactly at a successful command boundary", async () => {
    controller.perform.mockImplementationOnce(async (_action, id) => { components.push(installed(id)); await queue.cancel(); return { status: "success", component: installed(id) }; }); await mount();
    expect(await run()).toEqual({ status: "cancelled" }); expect(controller.perform).toHaveBeenCalledTimes(1); expect(queue.completedIds).toEqual(["runtime"]);
  });
  it("lets a failed current-step cancellation be retried while retaining no-next", async () => {
    const pending = deferred<AsrComponentResult>(); controller.perform.mockReturnValueOnce(pending.promise);
    controller.cancel.mockResolvedValueOnce({ status: "error", error: "IPC unavailable" }); await mount(); let request!: Promise<AsrSetupQueueResult>;
    await act(async () => { request = queue.start(plan()); }); await act(async () => { await queue.cancel(); });
    expect(queue.cancelling).toBe(false); expect(queue.error).toBe("IPC unavailable"); await act(async () => { await queue.cancel(); }); expect(controller.cancel).toHaveBeenCalledTimes(2);
    await act(async () => { pending.resolve({ status: "success", component: installed("runtime") }); expect(await request).toEqual({ status: "cancelled" }); }); expect(controller.perform).toHaveBeenCalledTimes(1);
  });
  it("invalidates an active selection through A → B → A and discards stale completion", async () => {
    const pending = deferred<AsrComponentResult>(); controller.perform.mockReturnValueOnce(pending.promise); await mount(); const key = asrSetupSelectionKey(selection()); let request!: Promise<AsrSetupQueueResult>;
    await act(async () => { request = queue.start(plan()); }); await change("selection-B"); await change(key);
    expect(controller.cancel).toHaveBeenCalledTimes(1); await act(async () => { pending.resolve({ status: "success", component: installed("runtime") }); expect(await request).toEqual({ status: "stale" }); });
    expect(controller.perform).toHaveBeenCalledTimes(1); expect(queue.completedIds).toEqual([]); expect(queue.status).toBe("idle");
  });
  it("invalidates an old confirmation after A → B → A even with identical metadata", async () => {
    await mount(); const oldGeneration = queue.generation; const preview = plan(); const key = asrSetupSelectionKey(selection()); const oldStart = queue.start;
    await change("selection-B"); await change(key);
    expect(await queue.start(preview, {}, oldGeneration)).toEqual({ status: "stale" }); expect(await oldStart(preview)).toEqual({ status: "stale" }); expect(controller.perform).not.toHaveBeenCalled();
    expect(await run()).toMatchObject({ status: "success" });
  });
  it("cancels on unmount and does not start later steps or publish late failures", async () => {
    const pending = deferred<AsrComponentResult>(); controller.perform.mockReturnValueOnce(pending.promise); await mount(); let request!: Promise<AsrSetupQueueResult>;
    await act(async () => { request = queue.start(plan()); }); const statusBefore = queue.status; act(() => renderer!.unmount()); renderer = undefined;
    expect(controller.cancel).toHaveBeenCalledTimes(1); pending.reject(new Error("late failure")); expect(await request).toEqual({ status: "stale" }); expect(controller.perform).toHaveBeenCalledTimes(1); expect(queue.status).toBe(statusBefore);
  });
});
