import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAsrBackendCheck } from "./use-asr-backend-check";
import { checkAsrBackend, releaseAsrBackend } from "@/lib/tauri-api";
import { defaultAsrConfig } from "@/config";
import type { AsrBackendStatus, AsrConfig, TFunction } from "@/types";

vi.mock("@/lib/tauri-api", () => ({ checkAsrBackend: vi.fn(), releaseAsrBackend: vi.fn() }));
const t: TFunction = (key) => key;
const config = { ...defaultAsrConfig, engine: "whisper-accelerated", python_path: "/python", model_path: "/model" };
const status: AsrBackendStatus = { ready: true, backend: "faster-whisper", device: "cpu", model_bytes: 10, aligner_bytes: 0, total_bytes: 10, capabilities: { offline: true, word_timestamps: true, supported_languages: null }, warnings: [] };
let state: ReturnType<typeof useAsrBackendCheck>;
let renderer: ReactTestRenderer | undefined;
function Probe({ value }: { value: AsrConfig }) { state = useAsrBackendCheck(value, t); return null; }
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function mount() { act(() => { renderer = create(<Probe value={config} />); }); }
function change(value: AsrConfig) { act(() => renderer!.update(<Probe value={value} />)); }
beforeEach(() => { vi.resetAllMocks(); vi.stubGlobal("window", { __TAURI_INTERNALS__: {} }); });
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; vi.unstubAllGlobals(); });

describe("local ASR capability checks", () => {
  it("blocks duplicate check and release clicks within the same configuration", async () => {
    const pending = deferred<AsrBackendStatus>(); vi.mocked(checkAsrBackend).mockReturnValue(pending.promise); mount();
    let request!: Promise<void>; act(() => { request = state.check(); void state.check(); void state.release(); });
    expect(checkAsrBackend).toHaveBeenCalledTimes(1); expect(releaseAsrBackend).not.toHaveBeenCalled();
    await act(async () => { pending.resolve(status); await request; });
    vi.mocked(releaseAsrBackend).mockResolvedValue(true); await act(async () => state.release()); expect(releaseAsrBackend).toHaveBeenCalledTimes(1);
  });

  it("reports busy while checking and displays the matching result", async () => {
    const pending = deferred<AsrBackendStatus>(); vi.mocked(checkAsrBackend).mockReturnValue(pending.promise);
    mount(); let request!: Promise<void>;
    act(() => { request = state.check(); });
    expect(state.checking).toBe(true);
    expect(checkAsrBackend).toHaveBeenCalledWith(config);
    await act(async () => { pending.resolve(status); await request; });
    expect(state.checking).toBe(false); expect(state.result).toEqual(status);
  });

  it("drops results when any field changes, including changing back to the original value", async () => {
    const pending = deferred<AsrBackendStatus>(); vi.mocked(checkAsrBackend).mockReturnValue(pending.promise);
    mount(); let request!: Promise<void>; act(() => { request = state.check(); });
    change({ ...config, model_path: "/different" }); expect(state.checking).toBe(false);
    change(config);
    await act(async () => { pending.resolve(status); await request; });
    expect(state.result).toBeUndefined();
  });

  it("does not let an old completion hide a newer pending check", async () => {
    const old = deferred<AsrBackendStatus>(); const next = deferred<AsrBackendStatus>();
    vi.mocked(checkAsrBackend).mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    mount(); let first!: Promise<void>; let second!: Promise<void>;
    act(() => { first = state.check(); }); change({ ...config, device: "cuda" }); act(() => { second = state.check(); });
    await act(async () => { old.resolve(status); await first; });
    expect(state.checking).toBe(true); expect(state.result).toBeUndefined();
    await act(async () => { next.resolve({ ...status, device: "cuda" }); await second; });
    expect(state.result?.device).toBe("cuda");
  });

  it("discards stale failures and completions after unmount", async () => {
    const pending = deferred<AsrBackendStatus>(); vi.mocked(checkAsrBackend).mockReturnValue(pending.promise);
    mount(); let request!: Promise<void>; act(() => { request = state.check(); });
    act(() => renderer!.unmount()); renderer = undefined;
    await act(async () => { pending.reject(new Error("old error")); await request; });
    expect(state.error).toBeUndefined();
  });

  it("clears prior results immediately after config changes and reports actionable failures", async () => {
    vi.mocked(checkAsrBackend).mockResolvedValue(status); mount();
    await act(async () => state.check()); expect(state.result?.ready).toBe(true);
    change({ ...config, python_path: "/other/python" }); expect(state.result).toBeUndefined();
    vi.mocked(checkAsrBackend).mockRejectedValue(new Error("Missing faster-whisper in selected Python"));
    await act(async () => state.check()); expect(state.error).toContain("Missing faster-whisper"); expect(state.checking).toBe(false);
  });

  it("releases memory explicitly, clears previous checks, and reports busy failures", async () => {
    vi.mocked(checkAsrBackend).mockResolvedValue(status); vi.mocked(releaseAsrBackend).mockResolvedValue(true); mount();
    await act(async () => state.check()); await act(async () => state.release());
    expect(releaseAsrBackend).toHaveBeenCalledTimes(1); expect(state.notice).toBe("asr.released"); expect(state.result).toBeUndefined();
    vi.mocked(releaseAsrBackend).mockRejectedValue(new Error("Transcription is running"));
    await act(async () => state.release()); expect(state.error).toBe("Transcription is running");
  });

  it("reports desktop runtime requirements without invoking a backend in a web preview", async () => {
    vi.stubGlobal("window", {}); mount(); await act(async () => state.check());
    expect(checkAsrBackend).not.toHaveBeenCalled(); expect(state.error).toBe("notice.requireTauriConfig");
  });
});
