import { useState, type ReactNode } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog } from "@/components/ui/dialog";
import { createAsrSetupPlan } from "@/lib/asr-setup-plan";
import { AsrSetupDialog } from "./asr-setup-dialog";
import type { useAsrSetupQueue } from "@/hooks/use-asr-setup-queue";
import { runtimeAcknowledgement } from "@/lib/asr-components";
import { ManagedAsrSetup } from "./managed-asr-setup";
import { defaultAsrConfig } from "@/config";
import { enUS } from "@/locales/en-US";
import { zhCN } from "@/locales/zh-CN";
import type { useAsrComponents } from "@/hooks/use-asr-components";
import type { AsrComponentCatalog, AsrComponentStatus } from "@/lib/asr-components";
import type { AsrConfig, TFunction } from "@/types";

const mocks = vi.hoisted(() => ({ manager: {} as ReturnType<typeof useAsrComponents>, queue: {} as ReturnType<typeof useAsrSetupQueue>, change: vi.fn() }));
vi.mock("@/hooks/use-asr-components", () => ({ useAsrComponents: () => mocks.manager }));
vi.mock("@/hooks/use-asr-setup-queue", () => ({ useAsrSetupQueue: () => mocks.queue }));
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
beforeEach(() => { vi.clearAllMocks(); mocks.queue = { generation: 0, status: "idle", currentIndex: 0, completedIds: [], cancelling: false, finishingCurrent: false, busy: false, start: vi.fn(), cancel: vi.fn() }; mocks.manager = { epoch: 0, catalog: fixture(), components: [], consents: [], loading: false, verified: true, busy: false, refresh: vi.fn(), perform: vi.fn(), cancel: vi.fn() }; });
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; });

describe("managed ASR setup UI", () => {
  it("offers compatible components with exact source/license/size disclosure and never fills paths automatically", () => {
    mount(); expect(text()).toContain("Windows CPU pack"); expect(text()).not.toContain("Mac MLX pack"); expect(text()).not.toContain("MLX-only model");
    expect(text()).toContain("Download: 1.0 MB · Installed: 2.0 MB"); expect(text()).toContain("https://example.test/runtime.zip"); expect(text()).toContain("https://example.test/LICENSE");
    expect(renderer!.root.findAllByType("input")).toHaveLength(0); expect(mocks.change).not.toHaveBeenCalled(); expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(buttons("asr.managed.use")).toHaveLength(0); expect(buttons("asr.setup.install")[0].props.disabled).toBe(false);
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
    expect(text()).toContain("Required aligner"); expect(buttons("asr.managed.use")).toHaveLength(0); expect(buttons("asr.setup.install")[0].props.disabled).toBe(false);
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
    const keys = Object.keys(enUS).filter((key) => key.startsWith("asr.managed.") || key.startsWith("asr.terms.") || key.startsWith("asr.setup.")); expect(keys.length).toBeGreaterThan(30);
    keys.forEach((key) => expect(zhCN[key as keyof typeof zhCN]).toBeTruthy());
  });
});

function nodeText(node: ReactTestInstance | string): string {
  return typeof node === "string" ? node : node.children.map(nodeText).join("");
}
function setupCard() {
  return renderer!.root.findAll((node) => node.type === "div" && node.props["data-testid"] === "asr-setup-card")[0];
}
function setupPlan() {
  const catalog = mocks.manager.catalog!;
  return createAsrSetupPlan({ engine: catalog.runtimes[0].engine, runtime: catalog.runtimes[0],
    model: catalog.models.find((item) => item.role === "model" && item.backend === catalog.runtimes[0].backend),
    aligner: catalog.models.find((item) => item.role === "aligner" && item.backend === catalog.runtimes[0].backend),
    components: mocks.manager.components, consents: mocks.manager.consents, verified: mocks.manager.verified })!;
}
function rerender(config = old, disabled = false) {
  act(() => renderer!.update(<Harness config={config} disabled={disabled} />));
}
function select(index: number, value: string) {
  act(() => renderer!.root.findAllByType("select")[index].props.onChange({ target: { value } }));
}

describe("one-click ASR setup", () => {
  it("keeps the default card compact and advanced management collapsed", () => {
    mount();
    const card = setupCard();
    expect(nodeText(card)).toContain(t("asr.setup.whisperTitle"));
    expect(nodeText(card)).toContain("Windows · CPU");
    expect(nodeText(card)).toContain(t("asr.setup.experimental"));
    expect(nodeText(card)).toContain("2 components to download · 3.0 MB");
    expect(nodeText(card)).not.toContain("Installed:");
    expect(nodeText(card)).not.toContain("https://");
    expect(card.findAllByType("select")).toHaveLength(0);
    expect(card.findAllByType("button")).toHaveLength(1);
    expect(nodeText(card.findByType("button"))).toBe(t("asr.setup.install"));
    const advanced = renderer!.root.findAllByType("details").find((node) =>
      node.findAllByType("summary").some((summary) => nodeText(summary) === t("asr.setup.advanced")))!;
    expect(advanced.props.open).toBeFalsy();
    expect(advanced.findAllByType("select")).toHaveLength(2);
    expect(renderer!.root.findAllByType("details").every((node) => !node.props.open)).toBe(true);
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it("excludes the installed runtime from the 3.72 GB Qwen download summary", () => {
    mocks.manager.catalog = fixture("qwen3-asr");
    const catalog = mocks.manager.catalog;
    catalog.runtimes[0] = { ...catalog.runtimes[0], archive: { ...catalog.runtimes[0].archive!, bytes: 411_133_181 } };
    const model = catalog.models.find((item) => item.id === "model")!;
    model.files = [{ ...model.files[0], bytes: 1_880_618_159 }];
    const aligner = catalog.models.find((item) => item.id === "aligner")!;
    aligner.files = [{ ...aligner.files[0], bytes: 1_840_070_940 }];
    mocks.manager.components = [{ ...installed("runtime"), installed_bytes: 411_133_181 }];
    mount({ ...old, engine: "qwen3-asr" });
    const summary = nodeText(setupCard());
    expect(summary).toContain("2 components to download · 3.72 GB");
    expect(summary).toContain("RuntimeReady");
    expect(summary).not.toContain("4.13 GB");
    expect(summary).not.toContain("411.1 MB");
    expect(summary).toContain(t("asr.setup.qwenHint"));
    expect(buttons("asr.managed.use")).toHaveLength(0);
    click("asr.setup.install");
    expect(nodeText(renderer!.root.findByProps({ role: "dialog" }))).toContain("Automatically install 2 components · 3.72 GB.");
    expect(renderer!.root.findByType(AsrSetupDialog).props.plan.steps.map((step: { component: { id: string } }) => step.component.id)).toEqual(["model", "aligner"]);
    expect(renderer!.root.findAllByType(Checkbox)).toHaveLength(0);
    expect(mocks.queue.start).not.toHaveBeenCalled();
  });

  it("reviews one frozen download plan with exact runtime terms and model licenses before starting once", () => {
    const runtime = useRecipe();
    mocks.manager.catalog!.models[0] = { ...mocks.manager.catalog!.models[0], license: "Apache-2.0", license_url: "https://example.test/model-LICENSE" };
    mount();
    click("asr.setup.install");
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(1);
    expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(mocks.queue.start).not.toHaveBeenCalled();
    const dialog = renderer!.root.findByProps({ role: "dialog" });
    const plan = renderer!.root.findByType(AsrSetupDialog).props.plan;
    expect(Object.isFrozen(plan)).toBe(true);
    expect(plan.steps.map((step: { component: { id: string } }) => step.component.id)).toEqual(["runtime", "model"]);
    expect(nodeText(dialog)).toContain("Automatically install 2 components · 3.3 MB.");
    expect(nodeText(dialog)).toContain("Apache-2.0");
    expect(dialog.findAllByType("a").map((link) => link.props.href)).toContain("https://example.test/model-LICENSE");
    expect(dialog.findAllByType("pre").map(nodeText)).toEqual(runtime.recipe!.terms.map((term) => term.text));
    for (const term of runtime.recipe!.terms) {
      expect(nodeText(dialog)).toContain(term.id);
      expect(nodeText(dialog)).toContain(term.version);
      expect(nodeText(dialog)).toContain(term.sha256);
      expect(dialog.findAllByType("a").map((link) => link.props.href)).toContain(term.url);
    }
    expect(dialog.findAllByType("details").every((node) => !node.props.open)).toBe(true);
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    expect(buttons("asr.setup.start")[0].props.disabled).toBe(true);
    click("asr.setup.start");
    expect(mocks.queue.start).not.toHaveBeenCalled();
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange("indeterminate"));
    expect(buttons("asr.setup.start")[0].props.disabled).toBe(true);
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const accept = buttons("asr.setup.start")[0].props.onClick;
    act(() => { accept(); accept(); });
    expect(mocks.queue.start).toHaveBeenCalledTimes(1);
    expect(mocks.queue.start).toHaveBeenCalledWith(plan, { runtime: runtimeAcknowledgement(runtime) }, 0);
    expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(mocks.change).not.toHaveBeenCalled();
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });

  it("starts model-only setup without fabricating runtime consent", () => {
    useRecipe();
    mocks.manager.components = [installed("runtime")];
    mount(); click("asr.setup.install");
    expect(renderer!.root.findAllByType(Checkbox)).toHaveLength(0);
    expect(renderer!.root.findAllByType("pre")).toHaveLength(0);
    expect(buttons("asr.setup.start")[0].props.disabled).toBe(false);
    const plan = renderer!.root.findByType(AsrSetupDialog).props.plan;
    click("asr.setup.start");
    expect(mocks.queue.start).toHaveBeenCalledWith(plan, {}, 0);
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it("blocks the primary setup action when exact runtime acknowledgement is incomplete", () => {
    const runtime = useRecipe();
    mocks.manager.catalog!.runtimes[0] = { ...runtime, plan_sha256: null };
    mount();
    expect(nodeText(setupCard())).toContain(t("asr.setup.unavailable"));
    expect(buttons("asr.setup.install")[0].props.disabled).toBe(true);
    click("asr.setup.install");
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
    expect(mocks.queue.start).not.toHaveBeenCalled();
    expect(mocks.manager.perform).not.toHaveBeenCalled();
  });

  it("discloses optional Windows certificate checks and Microsoft downloads in the combined confirmation", () => {
    const runtime = useRecipe();
    mocks.manager.catalog!.runtimes[0] = { ...runtime, recipe: { ...runtime.recipe!, windows_crt: "msvc-14.44.35211-x64" } };
    mount(); click("asr.setup.install");
    const dialog = renderer!.root.findByProps({ role: "dialog" });
    expect(nodeText(dialog)).toContain(t("asr.setup.windowsTrust"));
    expect(dialog.findAllByType("a").some((link) => new URL(link.props.href).hostname === "download.visualstudio.microsoft.com")).toBe(true);
    expect(nodeText(dialog)).toContain("26,935,768 bytes");
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    expect(mocks.queue.start).not.toHaveBeenCalled();
  });

  it.each(["back", "dismiss"])("discards assent on %s and starts every reopened confirmation unchecked", (closeMethod) => {
    useRecipe(); mount(); click("asr.setup.install");
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.setup.start")[0].props.onClick;
    if (closeMethod === "back") click("asr.setup.back");
    else act(() => renderer!.root.findByType(Dialog).props.onOpenChange(false));
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
    act(() => staleAccept());
    click("asr.setup.install");
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    expect(buttons("asr.setup.start")[0].props.disabled).toBe(true);
    act(() => staleAccept());
    expect(mocks.queue.start).not.toHaveBeenCalled();
    expect(mocks.manager.perform).not.toHaveBeenCalled();
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it("rejects stale confirmation callbacks after model A to B to A selection changes", () => {
    const runtime = useRecipe();
    const model = mocks.manager.catalog!.models[0];
    mocks.manager.catalog!.models.push({ ...model, id: "model-b", label: "Second compatible model" });
    mount(); click("asr.setup.install");
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.setup.start")[0].props.onClick;
    const staleParentAccept = renderer!.root.findByType(AsrSetupDialog).props.onAccept;
    select(1, "model-b"); select(1, "model");
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
    act(() => { staleAccept(); staleParentAccept({ runtime: runtimeAcknowledgement(runtime) }); });
    expect(mocks.queue.start).not.toHaveBeenCalled();
    click("asr.setup.install");
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    act(() => staleParentAccept({ runtime: runtimeAcknowledgement(runtime) }));
    expect(mocks.queue.start).not.toHaveBeenCalled();
  });

  it("rejects pending consent when exact terms change under the same runtime ID", () => {
    const runtime = useRecipe(); mount(); click("asr.setup.install");
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = renderer!.root.findByType(AsrSetupDialog).props.onAccept;
    mocks.manager.catalog!.runtimes[0] = { ...runtime, recipe: { ...runtime.recipe!, terms: [{ ...runtime.recipe!.terms[0], text: "Replacement exact agreement.", version: "2026-02", sha256: "f".repeat(64) }] } };
    rerender();
    act(() => staleAccept({ runtime: runtimeAcknowledgement(runtime) }));
    expect(mocks.queue.start).not.toHaveBeenCalled();
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
    click("asr.setup.install");
    expect(renderer!.root.findByType(Checkbox).props.checked).toBe(false);
    expect(renderer!.root.findAllByType("pre").map(nodeText)).toEqual(["Replacement exact agreement."]);
    expect(nodeText(renderer!.root.findByProps({ role: "dialog" }))).not.toContain("Exact third-party terms requiring acceptance.");
  });

  it.each(["engine", "unmount"])("invalidates pending one-click consent on %s", (change) => {
    const runtime = useRecipe();
    act(() => { renderer = create(<ManagedAsrSetup config={old} onChange={mocks.change} disabled={false} t={t} />); });
    click("asr.setup.install");
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.setup.start")[0].props.onClick;
    const staleParentAccept = renderer!.root.findByType(AsrSetupDialog).props.onAccept;
    if (change === "unmount") act(() => renderer!.unmount());
    else {
      mocks.manager.catalog = fixture("qwen3-asr");
      act(() => renderer!.update(<ManagedAsrSetup config={{ ...old, engine: "qwen3-asr" }} onChange={mocks.change} disabled={false} t={t} />));
    }
    act(() => { staleAccept(); staleParentAccept({ runtime: runtimeAcknowledgement(runtime) }); });
    expect(mocks.queue.start).not.toHaveBeenCalled();
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it("disables confirmation after settings are locked, including a previously captured callback", () => {
    const runtime = useRecipe(); mount(); click("asr.setup.install");
    act(() => renderer!.root.findByType(Checkbox).props.onCheckedChange(true));
    const staleAccept = buttons("asr.setup.start")[0].props.onClick;
    const staleParentAccept = renderer!.root.findByType(AsrSetupDialog).props.onAccept;
    rerender(old, true);
    expect(renderer!.root.findByType(Checkbox).props.disabled).toBe(true);
    expect(buttons("asr.setup.start")[0].props.disabled).toBe(true);
    act(() => { staleAccept(); staleParentAccept({ runtime: runtimeAcknowledgement(runtime) }); });
    expect(mocks.queue.start).not.toHaveBeenCalled();
  });

  it("keeps selectors and per-component operations locked across the whole queue", () => {
    const plan = setupPlan();
    mocks.queue = { ...mocks.queue, status: "running", busy: true, plan, currentIndex: 1, completedIds: ["runtime"] };
    mocks.manager.components = [installed("runtime")];
    mocks.manager.operation = { request_id: "model-request", component_id: "model", phase: "downloading", downloaded_bytes: 500_000, total_bytes: 2_000_000, message: "Downloading model" };
    mount();
    expect(nodeText(setupCard())).toContain("Step 2 of 2");
    expect(nodeText(setupCard())).toContain("1.5 MB / 3.0 MB");
    expect(renderer!.root.findByProps({ role: "progressbar" }).props["aria-valuenow"]).toBe(50);
    expect(renderer!.root.findAllByType("select").every((node) => node.props.disabled)).toBe(true);
    for (const key of ["asr.managed.install", "asr.managed.repair", "asr.managed.remove", "asr.managed.refresh"]) {
      expect(buttons(key).length).toBeGreaterThan(0);
      expect(buttons(key).every((node) => node.props.disabled)).toBe(true);
    }
    expect(buttons("asr.setup.install")).toHaveLength(0);
    expect(buttons("asr.managed.use")).toHaveLength(0);
    expect(buttons("asr.managed.cancel")).toHaveLength(1);
    click("asr.managed.cancel");
    expect(mocks.queue.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.manager.cancel).not.toHaveBeenCalled();
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it("keeps a cancelling activation visible while preventing another cancellation", () => {
    mocks.queue = { ...mocks.queue, status: "running", busy: true, cancelling: true, finishingCurrent: true, plan: setupPlan() };
    mocks.manager.operation = { request_id: "request", component_id: "runtime", phase: "activating", downloaded_bytes: 1_000_000, total_bytes: 1_000_000, message: "Activating" };
    mount();
    expect(nodeText(setupCard())).toContain(t("asr.setup.stopping"));
    expect(nodeText(setupCard())).toContain(t("asr.setup.finishingCurrent"));
    expect(buttons("asr.managed.cancelling")[0].props.disabled).toBe(true);
    expect(renderer!.root.findAllByType("select").every((node) => node.props.disabled)).toBe(true);
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it.each(["cancelled", "error", "stale"] as const)("shows %s and previews only missing work on retry", (status) => {
    const originalPlan = setupPlan();
    mocks.manager.components = [installed("runtime")];
    mocks.queue = { ...mocks.queue, status, plan: originalPlan, completedIds: ["runtime"], ...(status === "error" ? { error: "Checksum mismatch: model.bin" } : {}) };
    mount();
    expect(nodeText(setupCard())).toContain(t(`asr.setup.${status}`));
    expect(nodeText(setupCard())).toContain("1 components to download · 2.0 MB");
    expect(buttons("asr.setup.retry")[0].props.disabled).toBe(false);
    expect(buttons("asr.managed.use")).toHaveLength(0);
    if (status === "error") expect(text()).toContain("Checksum mismatch: model.bin");
    click("asr.setup.retry");
    expect(renderer!.root.findByType(AsrSetupDialog).props.plan.steps.map((step: { component: { id: string } }) => step.component.id)).toEqual(["model"]);
    expect(mocks.queue.start).not.toHaveBeenCalled();
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it("shows a short actionable error on the card while retaining full diagnostics in advanced details", () => {
    const firstLine = "Checksum mismatch: " + "x".repeat(260);
    const detail = "Detailed verification diagnostics for model.bin";
    mocks.queue = { ...mocks.queue, status: "error", plan: setupPlan(), error: `${firstLine}\n${detail}` };
    mount();
    expect(nodeText(setupCard())).toContain(firstLine.slice(0, 240));
    expect(nodeText(setupCard())).not.toContain(firstLine);
    expect(nodeText(setupCard())).not.toContain(detail);
    expect(nodeText(renderer!.root)).toContain(`${firstLine}\n${detail}`);
    expect(buttons("asr.setup.retry")[0].props.disabled).toBe(false);
  });

  it("never applies after queue success until a complete verified selection exists", () => {
    mocks.queue = { ...mocks.queue, status: "success", plan: setupPlan(), completedIds: ["runtime", "model"] };
    mocks.manager.components = [installed("runtime"), installed("model")];
    mocks.manager.verified = false;
    mount();
    expect(buttons("asr.managed.use")).toHaveLength(0);
    expect(buttons("asr.setup.install")[0].props.disabled).toBe(true);
    expect(mocks.change).not.toHaveBeenCalled();
    mocks.manager.verified = true;
    mocks.manager.components[1] = { ...installed("model"), path: "relative/model" };
    rerender();
    expect(buttons("asr.managed.use")).toHaveLength(0);
    expect(nodeText(setupCard())).not.toContain("Speech modelReady");
    expect(mocks.change).not.toHaveBeenCalled();
    mocks.manager.components[1] = installed("model");
    rerender();
    expect(nodeText(setupCard())).toContain(t("asr.setup.allReady"));
    expect(buttons("asr.managed.use")[0].props.disabled).toBe(false);
    expect(mocks.change).not.toHaveBeenCalled();
    click("asr.managed.use");
    expect(mocks.change).toHaveBeenCalledTimes(1);
    expect(mocks.change).toHaveBeenCalledWith({ ...old, python_path: "/private/runtime/python", model_path: "/private/model", aligner_path: "", device: "cpu" });
    expect(buttons("asr.managed.selected")[0].props.disabled).toBe(true);
    expect(nodeText(setupCard())).toContain(t("asr.managed.saveHint"));
  });

  it("does not expose Apply for mismatched versions or missing Qwen aligner paths", () => {
    mocks.manager.catalog = fixture("qwen3-asr");
    mocks.manager.components = [installed("runtime"), { ...installed("model"), version: "old" }, installed("aligner")];
    mount({ ...old, engine: "qwen3-asr" });
    expect(buttons("asr.managed.use")).toHaveLength(0);
    mocks.manager.components[1] = installed("model");
    mocks.manager.components[2] = { ...installed("aligner"), path: null };
    rerender({ ...old, engine: "qwen3-asr" });
    expect(buttons("asr.managed.use")).toHaveLength(0);
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it.each([
    ["runtime", "Runtime"], ["model", "Speech model"], ["aligner", "Timing model"],
  ])("requires repair instead of showing Ready or Apply when installed %s has an error", (id, label) => {
    mocks.manager.catalog = fixture("qwen3-asr");
    mocks.manager.components = [installed("runtime"), installed("model"), installed("aligner")]
      .map((status) => status.id === id ? { ...status, error: "Verification failed" } : status);
    mount({ ...old, engine: "qwen3-asr" });
    expect(nodeText(setupCard())).not.toContain(`${label}Ready`);
    expect(nodeText(setupCard())).not.toContain(t("asr.setup.allReady"));
    expect(buttons("asr.managed.use")).toHaveLength(0);
    click("asr.setup.install");
    const plan = renderer!.root.findByType(AsrSetupDialog).props.plan;
    expect(plan.steps).toHaveLength(1);
    expect(plan.steps[0]).toMatchObject({ action: "repair", component: { id } });
    expect(mocks.change).not.toHaveBeenCalled();
  });
  it("shows cancellation IPC failure on the running card and keeps Cancel available", () => {
    mocks.queue = { ...mocks.queue, status: "running", busy: true, plan: setupPlan(), error: "Cancellation request failed" };
    mount();
    expect(nodeText(setupCard())).toContain("Cancellation request failed");
    expect(buttons("asr.managed.cancel")[0].props.disabled).toBe(false);
    click("asr.managed.cancel");
    expect(mocks.queue.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.change).not.toHaveBeenCalled();
  });

  it.each(["install", "apply"] as const)("rejects a captured %s action after model A → B → A", (action) => {
    mocks.manager.catalog!.models.push({ ...mocks.manager.catalog!.models[0], id: "other-model", label: "Other model" });
    if (action === "apply") mocks.manager.components = [installed("runtime"), installed("model"), installed("other-model")];
    mount();
    const stale = buttons(action === "apply" ? "asr.managed.use" : "asr.setup.install")[0].props.onClick;
    act(() => renderer!.root.findAllByType("select")[1].props.onChange({ target: { value: "other-model" } }));
    act(() => renderer!.root.findAllByType("select")[1].props.onChange({ target: { value: "model" } }));
    act(() => stale());
    expect(mocks.change).not.toHaveBeenCalled();
    expect(renderer!.root.findAllByProps({ role: "dialog" })).toHaveLength(0);
  });

});
