import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listen, type EventCallback } from "@tauri-apps/api/event";
import { useAsrComponents } from "./use-asr-components";
import { asrComponentCatalog, asrComponentStatus, cancelAsrComponent, installAsrComponent, removeAsrComponent, repairAsrComponent } from "@/lib/tauri-api";
import type { AsrComponentCatalog, AsrComponentProgress, AsrComponentStatus, AsrComponentsSnapshot } from "@/lib/asr-components";

vi.mock("@/lib/tauri-api", () => ({ asrComponentCatalog: vi.fn(), asrComponentStatus: vi.fn(), cancelAsrComponent: vi.fn(), installAsrComponent: vi.fn(), removeAsrComponent: vi.fn(), repairAsrComponent: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
const catalog: AsrComponentCatalog = { schema: 1, platform: "windows-x64", runtimes: [], models: [] };
const installed: AsrComponentStatus = { id: "runtime", kind: "runtime", state: "installed", version: "1", path: "/private/runtime", python_path: "/private/runtime/python", installed_bytes: 20, error: null };
const empty: AsrComponentsSnapshot = { components: [], operation: null };
let state: ReturnType<typeof useAsrComponents>;
let renderer: ReactTestRenderer | undefined;
let callbacks: EventCallback<AsrComponentProgress>[];
let unlisten: ReturnType<typeof vi.fn>;
let nextId: number;
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function Probe({ engine = "whisper-accelerated" }: { engine?: string }) { state = useAsrComponents(engine, (key) => key); return null; }
async function mount(engine?: string) { await act(async () => { renderer = create(<Probe engine={engine} />); }); }
async function change(engine: string) { await act(async () => renderer!.update(<Probe engine={engine} />)); }
function progress(requestId: string, phase: AsrComponentProgress["phase"] = "downloading", componentId = "runtime"): AsrComponentProgress { return { request_id: requestId, component_id: componentId, phase, downloaded_bytes: 25, total_bytes: 100, message: phase }; }
function emit(payload: AsrComponentProgress, index = callbacks.length - 1) { act(() => callbacks[index]({ payload, event: "asr-component-progress", id: 1 })); }
beforeEach(() => {
  vi.resetAllMocks(); callbacks = []; unlisten = vi.fn(); nextId = 0;
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} }); vi.stubGlobal("crypto", { randomUUID: () => `request-${++nextId}` });
  vi.mocked(listen).mockImplementation(async (_name, callback) => { callbacks.push(callback as EventCallback<AsrComponentProgress>); return unlisten; });
  vi.mocked(asrComponentCatalog).mockResolvedValue(catalog); vi.mocked(asrComponentStatus).mockResolvedValue(empty); vi.mocked(cancelAsrComponent).mockResolvedValue(true);
});
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; vi.unstubAllGlobals(); });

describe("managed ASR component controller", () => {
  it("only reads on opening and listens before allowing explicit installs", async () => {
    await mount(); expect(state.loading).toBe(false); expect(state.catalog).toEqual(catalog);
    expect(listen).toHaveBeenCalledWith("asr-component-progress", expect.any(Function));
    expect(installAsrComponent).not.toHaveBeenCalled(); expect(repairAsrComponent).not.toHaveBeenCalled(); expect(removeAsrComponent).not.toHaveBeenCalled();
  });
  it("coalesces duplicate clicks, filters request and component IDs, and uses the returned paths", async () => {
    const pending = deferred<AsrComponentStatus>(); vi.mocked(installAsrComponent).mockReturnValue(pending.promise); await mount();
    let first!: Promise<void>; act(() => { first = state.perform("install", "runtime"); void state.perform("install", "runtime"); });
    expect(installAsrComponent).toHaveBeenCalledTimes(1); expect(installAsrComponent).toHaveBeenCalledWith("runtime", "request-1"); expect(state.busy).toBe(true);
    emit(progress("stale")); expect(state.operation?.phase).toBe("preparing");
    emit(progress("request-1", "downloading", "other")); expect(state.operation?.phase).toBe("preparing");
    emit(progress("request-1")); expect(state.operation?.downloaded_bytes).toBe(25);
    emit(progress("request-1", "complete")); expect(state.busy).toBe(true);
    await act(async () => { pending.resolve(installed); await first; });
    expect(state.busy).toBe(false); expect(state.components).toEqual([installed]);
    emit(progress("request-1")); expect(state.operation).toBeUndefined();
  });
  it("waits for terminal cancellation and prevents repeated cancellation clicks", async () => {
    const pending = deferred<AsrComponentStatus>(); vi.mocked(installAsrComponent).mockReturnValue(pending.promise); await mount();
    let first!: Promise<void>; act(() => { first = state.perform("install", "runtime"); });
    await act(async () => { await state.cancel(); await state.cancel(); });
    expect(cancelAsrComponent).toHaveBeenCalledTimes(1); expect(cancelAsrComponent).toHaveBeenCalledWith("request-1"); expect(state.busy).toBe(true); expect(state.cancelling).toBe(true);
    emit(progress("request-1", "cancelled"));
    await act(async () => { pending.reject(new Error("cancelled")); await first; });
    expect(state.busy).toBe(false); expect(state.error).toBeUndefined(); expect(state.notice).toBe("asr.managed.phase.cancelled");
  });
  it("keeps real cancellation errors retryable", async () => {
    vi.mocked(installAsrComponent).mockReturnValue(new Promise(() => {})); vi.mocked(cancelAsrComponent).mockRejectedValueOnce(new Error("cancel failed")); await mount();
    act(() => { void state.perform("install", "runtime"); }); await act(async () => state.cancel());
    expect(state.error).toBe("cancel failed"); expect(state.cancelling).toBe(false); expect(state.busy).toBe(true);
    await act(async () => state.cancel()); expect(cancelAsrComponent).toHaveBeenCalledTimes(2);
  });
  it("recovers a failed repair and allows a fresh retry", async () => {
    vi.mocked(repairAsrComponent).mockRejectedValueOnce(new Error("checksum mismatch")).mockResolvedValueOnce(installed);
    vi.mocked(asrComponentStatus).mockResolvedValueOnce(empty).mockResolvedValueOnce({ components: [{ ...installed, state: "damaged" }], operation: null });
    await mount(); await act(async () => state.perform("repair", "runtime"));
    expect(state.error).toBe("checksum mismatch"); expect(state.components[0].state).toBe("damaged"); expect(state.busy).toBe(false);
    await act(async () => state.perform("repair", "runtime")); expect(state.components[0].state).toBe("installed"); expect(state.error).toBeUndefined();
  });
  it("does not enable selection from stale integrity results after failed repair and failed refresh", async () => {
    vi.mocked(asrComponentStatus).mockResolvedValueOnce({ components: [installed], operation: null }).mockRejectedValueOnce(new Error("integrity check unavailable"));
    vi.mocked(repairAsrComponent).mockRejectedValueOnce(new Error("repair failed")); await mount(); expect(state.verified).toBe(true);
    await act(async () => state.perform("repair", "runtime")); expect(state.verified).toBe(false); expect(state.error).toBe("repair failed");
    vi.mocked(asrComponentStatus).mockResolvedValueOnce({ components: [installed], operation: null }); await act(async () => state.refresh()); expect(state.verified).toBe(true);
  });
  it("removes only the requested managed component", async () => {
    vi.mocked(removeAsrComponent).mockResolvedValue({ ...installed, state: "not_installed", path: null, python_path: null }); await mount();
    await act(async () => state.perform("remove", "runtime"));
    expect(removeAsrComponent).toHaveBeenCalledWith("runtime", "request-1"); expect(state.components[0].state).toBe("not_installed");
  });
  it("cancels on engine switching and rejects old events and completions through A → B → A", async () => {
    const old = deferred<AsrComponentStatus>(); const next = deferred<AsrComponentStatus>(); vi.mocked(installAsrComponent).mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    await mount(); let first!: Promise<void>; act(() => { first = state.perform("install", "runtime"); });
    await change("qwen3-asr"); expect(cancelAsrComponent).toHaveBeenCalledWith("request-1"); await change("whisper-accelerated");
    let second!: Promise<void>; act(() => { second = state.perform("install", "runtime"); });
    emit(progress("request-1", "error"), 0); await act(async () => { old.resolve(installed); await first; });
    expect(state.busy).toBe(true); expect(state.components).toEqual([]); expect(state.error).toBeUndefined(); expect(state.operation?.request_id).toBe("request-2");
    await act(async () => { next.resolve(installed); await second; }); expect(state.components).toEqual([installed]);
  });
  it("unsubscribes and cancels an owned install on unmount without publishing late failure", async () => {
    const pending = deferred<AsrComponentStatus>(); vi.mocked(installAsrComponent).mockReturnValue(pending.promise); await mount();
    let first!: Promise<void>; act(() => { first = state.perform("install", "runtime"); }); act(() => renderer!.unmount()); renderer = undefined;
    expect(unlisten).toHaveBeenCalledTimes(1); expect(cancelAsrComponent).toHaveBeenCalledWith("request-1");
    await act(async () => { pending.reject(new Error("late failure")); await first; }); expect(state.error).toBeUndefined();
  });
  it("does not leak a listener that finishes registering after unmount", async () => {
    const registration = deferred<() => void>(); vi.mocked(listen).mockReturnValue(registration.promise); await mount();
    act(() => renderer!.unmount()); renderer = undefined; await act(async () => registration.resolve(unlisten));
    expect(unlisten).toHaveBeenCalledTimes(1); expect(asrComponentCatalog).not.toHaveBeenCalled();
  });
  it("drops stale catalog requests after engine changes", async () => {
    const old = deferred<AsrComponentCatalog>(); vi.mocked(asrComponentCatalog).mockReturnValueOnce(old.promise).mockResolvedValueOnce({ ...catalog, platform: "macos-arm64" });
    await mount(); await change("qwen3-asr"); await act(async () => old.resolve(catalog)); expect(state.catalog?.platform).toBe("macos-arm64");
  });
  it("observes a pending operation after remount and refreshes on its terminal event", async () => {
    vi.mocked(asrComponentStatus).mockResolvedValueOnce({ components: [], operation: progress("recovered") }).mockResolvedValueOnce({ components: [installed], operation: null }); await mount();
    expect(state.busy).toBe(true); await act(async () => state.perform("install", "other")); expect(installAsrComponent).not.toHaveBeenCalled();
    await act(async () => emit(progress("recovered", "complete"))); expect(state.busy).toBe(false); expect(state.components).toEqual([installed]);
    expect(cancelAsrComponent).not.toHaveBeenCalled();
  });
  it("reconciles terminal events received before a delayed status response", async () => {
    const snapshot = deferred<AsrComponentsSnapshot>(); vi.mocked(asrComponentStatus).mockReturnValueOnce(snapshot.promise).mockResolvedValueOnce({ components: [installed], operation: null }); await mount();
    emit(progress("recovered", "complete")); await act(async () => snapshot.resolve({ components: [], operation: progress("recovered") }));
    expect(state.busy).toBe(false); expect(state.components).toEqual([installed]);
  });
  it("reconciles an external completion during hashing even when status already says no operation", async () => {
    const snapshot = deferred<AsrComponentsSnapshot>(); vi.mocked(asrComponentStatus).mockReturnValueOnce(snapshot.promise).mockResolvedValueOnce({ components: [installed], operation: null }); await mount();
    emit(progress("external", "complete")); await act(async () => snapshot.resolve({ components: [], operation: null }));
    expect(state.busy).toBe(false); expect(state.components).toEqual([installed]); expect(asrComponentStatus).toHaveBeenCalledTimes(2);
  });
  it("adopts an external operation that starts while the status response is pending", async () => {
    const snapshot = deferred<AsrComponentsSnapshot>(); vi.mocked(asrComponentStatus).mockReturnValueOnce(snapshot.promise); await mount();
    emit(progress("external", "downloading")); await act(async () => snapshot.resolve(empty));
    expect(state.busy).toBe(true); expect(state.operation?.request_id).toBe("external");
  });
  it("supports refreshing after initial status or listener failure", async () => {
    vi.mocked(listen).mockRejectedValueOnce(new Error("event bridge unavailable")); await mount(); expect(state.error).toContain("event bridge unavailable");
    await act(async () => state.refresh()); expect(state.loading).toBe(false); expect(state.error).toBeUndefined(); expect(state.catalog).toEqual(catalog);
  });
  it("shows desktop requirements without installation attempts in a preview", async () => {
    vi.stubGlobal("window", {}); await mount(); expect(state.error).toBe("notice.requireTauriConfig"); expect(listen).not.toHaveBeenCalled(); expect(asrComponentCatalog).not.toHaveBeenCalled();
    await act(async () => state.perform("install", "runtime")); expect(installAsrComponent).not.toHaveBeenCalled();
  });
});
