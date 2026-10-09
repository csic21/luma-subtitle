import { useState, type ReactNode } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ManagedAsrSetup } from "./managed-asr-setup";
import { defaultAsrConfig } from "@/config";
import { enUS } from "@/locales/en-US";
import { zhCN } from "@/locales/zh-CN";
import type { useAsrComponents } from "@/hooks/use-asr-components";
import type { AsrComponentCatalog, AsrComponentStatus } from "@/lib/asr-components";
import type { AsrConfig, TFunction } from "@/types";

const mocks = vi.hoisted(() => ({ manager: {} as ReturnType<typeof useAsrComponents>, change: vi.fn() }));
vi.mock("@/hooks/use-asr-components", () => ({ useAsrComponents: () => mocks.manager }));
vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, disabled, children }: { value: string; onValueChange: (value: string) => void; disabled: boolean; children: ReactNode }) => <select value={value} disabled={disabled} onChange={(event) => onValueChange(event.target.value)}>{children}</select>,
  SelectTrigger: ({ children }: { children: ReactNode }) => <>{children}</>, SelectValue: () => null,
  SelectContent: ({ children }: { children: ReactNode }) => <>{children}</>, SelectGroup: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectItem: ({ children, ...props }: { children: ReactNode; value: string; disabled?: boolean }) => <option {...props}>{children}</option>,
}));
const t: TFunction = (key, values = {}) => Object.entries(values).reduce<string>((text, [key, value]) => text.split(`{${key}}`).join(String(value)), enUS[key as keyof typeof enUS] ?? key);
const old: AsrConfig = { ...defaultAsrConfig, engine: "whisper-accelerated", python_path: "/custom/python", model_path: "/external/model", device: "cuda" };
let renderer: ReactTestRenderer | undefined;
function fixture(engine = "whisper-accelerated"): AsrComponentCatalog {
  const base = { engine, version: "1", license: "MIT", license_url: "https://example.test/LICENSE", installed_bytes: 2_000_000, backend: "faster-whisper" };
  return { schema: 1, platform: "windows-x64", runtimes: [
    { ...base, id: "runtime", label: "Windows CPU pack", platform: "windows-x64", device: "cpu", entrypoint: "python.exe", max_files: 5, archive: { url: "https://example.test/runtime.zip", bytes: 1_000_000, sha256: "a".repeat(64) } },
    { ...base, id: "mac-runtime", label: "Mac MLX pack", platform: "macos-arm64", device: "metal", backend: "mlx-whisper", entrypoint: "bin/python", max_files: 5 },
  ], models: [
    { ...base, id: "model", label: "Compatible model", role: "model", files: [{ path: "model.bin", url: "https://example.test/model.bin", bytes: 2_000_000, sha256: "b".repeat(64) }] },
    { ...base, id: "mlx-model", label: "MLX-only model", backend: "mlx-whisper", role: "model", files: [] },
    ...(engine === "qwen3-asr" ? [{ ...base, id: "aligner", label: "Required aligner", role: "aligner" as const, files: [{ path: "model.safetensors", url: "https://example.test/aligner.safetensors", bytes: 2_000_000, sha256: "c".repeat(64) }] }] : []),
  ] };
}
function installed(id: string): AsrComponentStatus { return { id, state: "installed", kind: id === "runtime" ? "runtime" : "model", version: "1", path: `/private/${id}`, python_path: id === "runtime" ? "/private/runtime/python" : null, installed_bytes: 2_000_000, error: null }; }
function Harness({ config = old, disabled = false }: { config?: AsrConfig; disabled?: boolean }) {
  const [value, setValue] = useState(config);
  return <ManagedAsrSetup config={value} disabled={disabled} t={t} onChange={(next) => { mocks.change(next); setValue(next); }} />;
}
function mount(config = old, disabled = false) { act(() => { renderer = create(<Harness config={config} disabled={disabled} />); }); }
function buttons(key: string) { return renderer!.root.findAllByType("button").filter((button) => button.children.includes(t(key))); }
function click(key: string, index = 0) { act(() => buttons(key)[index].props.onClick()); }
function text() { return JSON.stringify(renderer!.toJSON()); }
beforeEach(() => { vi.clearAllMocks(); mocks.manager = { epoch: 0, catalog: fixture(), components: [], loading: false, verified: true, busy: false, refresh: vi.fn(), perform: vi.fn(), cancel: vi.fn() }; });
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; });

describe("managed ASR setup UI", () => {
  it("offers compatible components with exact source/license/size disclosure and never fills paths automatically", () => {
    mount(); expect(text()).toContain("Windows CPU pack"); expect(text()).not.toContain("Mac MLX pack"); expect(text()).not.toContain("MLX-only model");
    expect(text()).toContain("Download: 1.0 MB · Installed: 2.0 MB"); expect(text()).toContain("https://example.test/runtime.zip"); expect(text()).toContain("https://example.test/LICENSE");
    expect(renderer!.root.findAllByType("input")).toHaveLength(0); expect(mocks.change).not.toHaveBeenCalled(); expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(buttons("asr.managed.use")[0].props.disabled).toBe(true);
    click("asr.managed.install"); expect(mocks.manager.perform).toHaveBeenCalledWith("install", "runtime");
    click("asr.managed.install", 1); expect(mocks.manager.perform).toHaveBeenCalledWith("install", "model");
  });
  it("uses verified private paths only on explicit selection, preserving external settings before that", () => {
    mocks.manager.components = [installed("runtime"), installed("model")]; mount(); expect(mocks.change).not.toHaveBeenCalled();
    click("asr.managed.use"); expect(mocks.change).toHaveBeenCalledWith({ ...old, python_path: "/private/runtime/python", model_path: "/private/model", aligner_path: "", device: "cpu" });
    expect(buttons("asr.managed.selected")[0].props.disabled).toBe(true); expect(text()).toContain(t("asr.managed.saveHint")); expect(old.python_path).toBe("/custom/python");
  });
  it("requires a separate Qwen aligner installation before use", () => {
    mocks.manager.catalog = fixture("qwen3-asr"); mocks.manager.components = [installed("runtime"), installed("model")]; mount({ ...old, engine: "qwen3-asr" });
    expect(text()).toContain("Required aligner"); expect(buttons("asr.managed.use")[0].props.disabled).toBe(true);
    click("asr.managed.install"); expect(mocks.manager.perform).toHaveBeenCalledWith("install", "aligner");
    mocks.manager.components = [...mocks.manager.components, installed("aligner")]; act(() => renderer!.update(<Harness config={{ ...old, engine: "qwen3-asr" }} />));
    click("asr.managed.use"); expect(mocks.change.mock.calls[0][0].aligner_path).toBe("/private/aligner");
  });
  it("shows damaged state, repairs explicitly, and confirms removal without rewriting configs", () => {
    mocks.manager.components = [{ ...installed("runtime"), state: "damaged", error: "Missing executable" }]; mount();
    expect(text()).toContain("Needs repair"); expect(text()).toContain("Missing executable"); click("asr.managed.repair"); expect(mocks.manager.perform).toHaveBeenCalledWith("repair", "runtime");
    click("asr.managed.remove"); expect(mocks.manager.perform).not.toHaveBeenCalledWith("remove", "runtime"); expect(text()).toContain(t("asr.managed.removeHint"));
    click("asr.managed.keep"); expect(buttons("asr.managed.confirmRemove")).toHaveLength(0); click("asr.managed.remove"); click("asr.managed.confirmRemove");
    expect(mocks.manager.perform).toHaveBeenCalledWith("remove", "runtime"); expect(mocks.change).not.toHaveBeenCalled();
  });
  it("shows real transfer bytes and cancellation while disabling repeated operations and selection changes", () => {
    mocks.manager.busy = true; mocks.manager.operation = { request_id: "request", component_id: "runtime", phase: "downloading", downloaded_bytes: 500_000, total_bytes: 1_000_000, message: "Downloading runtime.zip" }; mount();
    expect(renderer!.root.findByProps({ role: "progressbar" }).props["aria-valuenow"]).toBe(50);
    expect(text()).toContain("500 KB"); expect(text()).toContain("1.0 MB"); expect(renderer!.root.findAllByType("select").every((select) => select.props.disabled)).toBe(true); expect(buttons("asr.managed.install").every((button) => button.props.disabled)).toBe(true);
    click("asr.managed.cancel"); expect(mocks.manager.cancel).toHaveBeenCalledTimes(1);
  });
  it("keeps unpublished downloads unavailable and preserves advanced local configs", () => {
    mocks.manager.catalog!.runtimes[0] = { ...mocks.manager.catalog!.runtimes[0], archive: null, unavailable_reason: "Awaiting verified pack" }; mount();
    expect(text()).toContain("Awaiting verified pack"); expect(buttons("asr.managed.install")[0].props.disabled).toBe(true); expect(mocks.change).not.toHaveBeenCalled();
  });
  it("reports unsupported platforms and offers refresh for failures", () => {
    mocks.manager.catalog!.platform = "unsupported"; mocks.manager.error = "Catalog unavailable"; mount();
    expect(text()).toContain(t("asr.managed.unsupported")); expect(text()).toContain("Catalog unavailable"); expect(buttons("asr.managed.install")).toHaveLength(0);
    click("asr.managed.refresh"); expect(mocks.manager.refresh).toHaveBeenCalledTimes(1);
  });
  it("disables actions when task settings cannot be edited", () => {
    mocks.manager.components = [installed("runtime"), installed("model")]; mount(old, true);
    expect(renderer!.root.findAllByType("button").every((button) => button.props.disabled)).toBe(true); expect(renderer!.root.findAllByType("select").every((select) => select.props.disabled)).toBe(true);
  });
  it("provides both language strings for every managed setup label", () => {
    const keys = Object.keys(enUS).filter((key) => key.startsWith("asr.managed.")); expect(keys.length).toBeGreaterThan(30);
    keys.forEach((key) => expect(zhCN[key as keyof typeof zhCN]).toBeTruthy());
  });
});
