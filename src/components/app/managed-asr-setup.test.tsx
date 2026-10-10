import { useState, type ReactNode } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog } from "@/components/ui/dialog";
import { runtimeAcknowledgement } from "@/lib/asr-components";
import { ManagedAsrSetup } from "./managed-asr-setup";
import { defaultAsrConfig } from "@/config";
import { enUS } from "@/locales/en-US";
import { zhCN } from "@/locales/zh-CN";
import type { useAsrComponents } from "@/hooks/use-asr-components";
import type { AsrComponentCatalog, AsrComponentStatus } from "@/lib/asr-components";
import type { AsrConfig, TFunction } from "@/types";

const mocks = vi.hoisted(() => ({ manager: {} as ReturnType<typeof useAsrComponents>, change: vi.fn() }));
vi.mock("@/hooks/use-asr-components", () => ({ useAsrComponents: () => mocks.manager }));
vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ open, children }: { open: boolean; children: ReactNode }) => open ? <div role="dialog">{children}</div> : null,
  DialogContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogDescription: ({ children }: { children: ReactNode }) => <p>{children}</p>,
  DialogFooter: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));
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
function useRecipe() {
  const runtime = mocks.manager.catalog!.runtimes[0];
  const updated = { ...runtime, archive: null, plan_sha256: "a".repeat(64), recipe: {
    schema: 1 as const,
    python: { url: "https://github.com/upstream/python.zip", bytes: 1_000_000, sha256: "b".repeat(64), installed_bytes: 3_000_000, max_files: 200, entrypoint: "python.exe", site_packages: "Lib/site-packages", pip_version: "25.3" },
    wheels: [{ name: "engine", version: "1.0", filename: "engine.whl", url: "https://files.pythonhosted.org/engine.whl", bytes: 300_000, sha256: "c".repeat(64), installed_bytes: 500_000, max_files: 20 }],
    terms: [
      { id: "vendor-license", version: "2026-01", sha256: "d".repeat(64), url: "https://example.test/terms-v1", text: "Exact third-party terms requiring acceptance." },
      { id: "second-license", version: "3", sha256: "e".repeat(64), url: "https://example.test/terms-v3", text: "A second exact agreement." },
    ],
  } };
  mocks.manager.catalog!.runtimes[0] = updated;
  return updated;
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
beforeEach(() => { vi.clearAllMocks(); mocks.manager = { epoch: 0, catalog: fixture(), components: [], consents: [], loading: false, verified: true, busy: false, refresh: vi.fn(), perform: vi.fn(), cancel: vi.fn() }; });
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
  it("shows retained downloads and allows explicit cache-only removal even for an unavailable runtime", () => {
    mocks.manager.catalog!.runtimes[0] = { ...mocks.manager.catalog!.runtimes[0], archive: null, unavailable_reason: "Unavailable on this OS" };
    mocks.manager.components = [{ ...installed("runtime"), state: "not_installed", path: null, python_path: null, installed_bytes: 0, cached_bytes: 750_000 }];
    mount(); expect(text()).toContain("Retained downloads: 750,000 bytes"); expect(text()).toContain("Files are checked again before reuse.");
    expect(text()).not.toContain("Verified installed usage"); expect(buttons("asr.managed.install")[0].props.disabled).toBe(true);
    expect(buttons("asr.managed.clearCache")[0].props.disabled).toBe(false); expect(mocks.manager.perform).not.toHaveBeenCalled();
    click("asr.managed.clearCache"); expect(text()).toContain(t("asr.managed.clearCacheHint")); expect(mocks.manager.perform).not.toHaveBeenCalled();
    click("asr.managed.keep"); expect(mocks.manager.perform).not.toHaveBeenCalled();
    click("asr.managed.clearCache"); click("asr.managed.confirmClearCache");
    expect(mocks.manager.perform).toHaveBeenCalledTimes(1); expect(mocks.manager.perform).toHaveBeenCalledWith("remove", "runtime"); expect(mocks.change).not.toHaveBeenCalled();
    mocks.manager.components = [{ ...mocks.manager.components[0], cached_bytes: 0 }]; act(() => renderer!.update(<Harness />)); expect(buttons("asr.managed.clearCache")).toHaveLength(0);
  });
  it("treats older status snapshots without cache bytes as zero while preserving installed removal", () => {
    mocks.manager.components = [{ ...installed("runtime"), state: "not_installed", path: null, python_path: null, installed_bytes: 0 }]; mount();
    expect(text()).not.toContain("Retained downloads:"); expect(buttons("asr.managed.clearCache")).toHaveLength(0); expect(buttons("asr.managed.remove")).toHaveLength(0);
    mocks.manager.components = [{ ...installed("runtime"), cached_bytes: 200_000 }]; act(() => renderer!.update(<Harness />));
    expect(text()).toContain("Verified installed usage: 2,000,000 bytes"); expect(text()).toContain("Retained downloads: 200,000 bytes");
    click("asr.managed.remove"); expect(text()).toContain(t("asr.managed.removeHint")); click("asr.managed.confirmRemove"); expect(mocks.manager.perform).toHaveBeenCalledWith("remove", "runtime");
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
  it("shows the exact runtime terms, plan, sources, sizes and unchecked assent before any install", () => {
    const runtime = useRecipe(); mount(); expect(mocks.manager.perform).not.toHaveBeenCalled();
    click("asr.terms.reviewInstall"); expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(text()).toContain("Exact third-party terms requiring acceptance."); expect(text()).toContain("A second exact agreement.");
    expect(text()).toContain("vendor-license"); expect(text()).toContain("2026-01"); expect(text()).toContain("d".repeat(64)); expect(text()).toContain("e".repeat(64));
    expect(text()).toContain("https://example.test/terms-v1"); expect(text()).toContain("github.com, files.pythonhosted.org"); expect(text()).toContain("1,300,000 bytes"); expect(text()).toContain("Installed size limit");
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false); expect(buttons("asr.terms.acceptInstall")[0].props.disabled).toBe(true);
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const accept = buttons("asr.terms.acceptInstall")[0].props.onClick;
    act(() => { accept(); accept(); });
    expect(mocks.manager.perform).toHaveBeenCalledTimes(1);
    expect(mocks.manager.perform).toHaveBeenCalledWith("install", runtime.id, runtimeAcknowledgement(runtime));
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });
  it("declines, dismisses, and reopens without recording assent or starting a download", () => {
    useRecipe(); mount(); click("asr.terms.reviewInstall");
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.terms.acceptInstall")[0].props.onClick;
    click("asr.terms.decline"); act(() => staleAccept()); expect(mocks.manager.perform).not.toHaveBeenCalled();
    click("asr.terms.reviewInstall"); expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    act(() => renderer!.root.findByType(Dialog).props.onOpenChange(false)); expect(mocks.manager.perform).not.toHaveBeenCalled();
  });
  it("discloses the optional Microsoft source, bytes and platform trust checks before assent", () => {
    const runtime = useRecipe();
    mocks.manager.catalog!.runtimes[0] = { ...runtime, recipe: { ...runtime.recipe!, windows_crt: "msvc-14.44.35211-x64" } };
    mount(); click("asr.terms.reviewInstall");
    expect(text()).toContain("download.visualstudio.microsoft.com");
    expect(text()).toContain("26,935,768 bytes");
    expect(text()).toContain(t("asr.terms.windowsTrust"));
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    expect(mocks.manager.perform).not.toHaveBeenCalled();
    click("asr.terms.decline"); expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(text()).toContain(t("asr.managed.windowsTrustNetwork"));
    const current = mocks.manager.catalog!.runtimes[0];
    mocks.manager.consents = [{ component_id: current.id, plan_sha256: current.plan_sha256!, terms: runtimeAcknowledgement(current)!.acknowledged_terms }];
    mocks.manager.components = [installed("runtime")];
    act(() => renderer!.update(<Harness />));
    expect(text()).toContain(t("asr.managed.windowsTrustNetwork"));
    expect(buttons("asr.managed.repair")[0].props.disabled).toBe(false);
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });
  it("does not infer assent from an installed runtime and only reuses matching backend receipts for repair", () => {
    const runtime = useRecipe(); mocks.manager.components = [installed("runtime")]; mount();
    click("asr.terms.reviewRepair"); expect(mocks.manager.perform).not.toHaveBeenCalled(); click("asr.terms.decline");
    mocks.manager.consents = [{ component_id: runtime.id, plan_sha256: runtime.plan_sha256!, terms: runtimeAcknowledgement(runtime)!.acknowledged_terms }];
    act(() => renderer!.update(<Harness />));
    expect(text()).toContain(t("asr.terms.reuse")); click("asr.managed.repair");
    expect(mocks.manager.perform).toHaveBeenCalledWith("repair", runtime.id, runtimeAcknowledgement(runtime));
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });
  it("requires new assent when a recorded plan or any term hash changes", () => {
    const runtime = useRecipe(); mocks.manager.components = [installed("runtime")];
    mocks.manager.consents = [{ component_id: runtime.id, plan_sha256: "f".repeat(64), terms: runtimeAcknowledgement(runtime)!.acknowledged_terms }];
    mount(); click("asr.terms.reviewRepair"); expect(mocks.manager.perform).not.toHaveBeenCalled();
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.terms.acceptInstall")[0].props.onClick;
    mocks.manager.catalog!.runtimes[0] = { ...runtime, recipe: { ...runtime.recipe!, terms: [{ ...runtime.recipe!.terms[0], version: "2026-02", sha256: "1".repeat(64) }] } };
    act(() => renderer!.update(<Harness />)); act(() => staleAccept());
    expect(mocks.manager.perform).not.toHaveBeenCalled(); expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
    click("asr.terms.reviewRepair"); expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false); expect(text()).toContain("2026-02");
  });
  it("invalidates pending assent on engine switching or unmount", () => {
    useRecipe(); mount(); click("asr.terms.reviewInstall"); act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.terms.acceptInstall")[0].props.onClick;
    act(() => renderer!.update(<ManagedAsrSetup config={{ ...old, engine: "qwen3-asr" }} onChange={mocks.change} disabled={false} t={t} />)); act(() => staleAccept());
    expect(mocks.manager.perform).not.toHaveBeenCalled(); expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });
  it("leaves ordinary model downloads free of runtime assent and disables incomplete runtime terms", () => {
    const runtime = useRecipe(); mocks.manager.catalog!.runtimes[0] = { ...runtime, plan_sha256: null }; mount();
    expect(buttons("asr.terms.reviewInstall")[0].props.disabled).toBe(true);
    click("asr.managed.install"); expect(mocks.manager.perform).toHaveBeenCalledWith("install", "model"); expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });
  it("provides both language strings for every managed setup label", () => {
    const keys = Object.keys(enUS).filter((key) => key.startsWith("asr.managed.") || key.startsWith("asr.terms.")); expect(keys.length).toBeGreaterThan(30);
    keys.forEach((key) => expect(zhCN[key as keyof typeof zhCN]).toBeTruthy());
  });
});
