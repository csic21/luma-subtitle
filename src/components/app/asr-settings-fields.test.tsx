import { useState, type ReactNode } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AsrSettingsFields } from "./asr-settings-fields";
import { defaultAsrConfig } from "@/config";
import { enUS } from "@/locales/en-US";
import type { AsrConfig, TFunction } from "@/types";

const mocks = vi.hoisted(() => ({ check: vi.fn(), release: vi.fn() }));
vi.mock("@/hooks/use-asr-backend-check", () => ({ useAsrBackendCheck: () => ({ checking: false, releasing: false, check: mocks.check, release: mocks.release }) }));
vi.mock("@/components/ui/scroll-area", () => ({ ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, disabled, children }: { value: string; onValueChange: (value: string) => void; disabled: boolean; children: ReactNode }) => <select value={value} disabled={disabled} onChange={(event) => onValueChange(event.target.value)}>{children}</select>,
  SelectTrigger: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectValue: () => null,
  SelectContent: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectGroup: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectItem: ({ children, ...props }: { children: ReactNode; value: string; disabled?: boolean }) => <option {...props}>{children}</option>,
}));
const t: TFunction = (key) => enUS[key as keyof typeof enUS] ?? key;
let renderer: ReactTestRenderer | undefined;
function Harness({ initial, disabled }: { initial?: AsrConfig; disabled?: boolean }) {
  const [value, setValue] = useState(initial);
  return <AsrSettingsFields value={value} onChange={setValue} disabled={disabled} t={t}><input aria-label="legacy-model" value="/models/ggml-large-v3-turbo-q5_0.bin" readOnly /><p>Turbo preset</p></AsrSettingsFields>;
}
function mount(initial?: AsrConfig, disabled = false) { act(() => { renderer = create(<Harness initial={initial} disabled={disabled} />); }); }
function selectEngine(engine: string) { act(() => renderer!.root.findAllByType("select")[0].props.onChange({ target: { value: engine } })); }
function text() { return JSON.stringify(renderer!.toJSON()); }
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; vi.clearAllMocks(); });

describe("shared transcription settings", () => {
  it("keeps no-Python Whisper and Turbo visible by default and retains them after switching back", () => {
    mount(); expect(text()).toContain("Turbo preset"); expect(renderer!.root.findAllByType("button").some((button) => button.children.includes(t("asr.release")))).toBe(true); expect(renderer!.root.findAllByType("input")).toHaveLength(1);
    selectEngine("qwen3-asr"); expect(renderer!.root.findAllByProps({ "aria-label": "legacy-model" })).toHaveLength(0); expect(renderer!.root.findAllByType("input")).toHaveLength(3);
    expect(text()).toContain("Mac: Qwen uses CPU"); expect(text()).toContain("Offline setup guide"); expect(text()).toContain("3.72 / 6.54 GB");
    selectEngine("whisper-cpp"); expect(renderer!.root.findByProps({ "aria-label": "legacy-model" }).props.value).toContain("turbo");
    expect(mocks.check).not.toHaveBeenCalled(); expect(mocks.release).not.toHaveBeenCalled();
  });

  it("requires explicit paths for optional checks and disables Qwen Metal", () => {
    mount({ ...defaultAsrConfig, engine: "qwen3-asr" });
    const check = () => renderer!.root.findAllByType("button").find((button) => button.children.includes(t("asr.check")))!;
    expect(check().props.disabled).toBe(true);
    expect(renderer!.root.findByProps({ value: "metal" }).props.disabled).toBe(true);
    for (const input of renderer!.root.findAllByType("input")) act(() => input.props.onChange({ target: { value: "/local/path" } }));
    expect(check().props.disabled).toBe(false);
    act(() => check().props.onClick()); expect(mocks.check).toHaveBeenCalledTimes(1);
  });

  it("shows model-format guidance for accelerated Whisper and preserves disabled task controls", () => {
    mount({ ...defaultAsrConfig, engine: "whisper-accelerated" }, true);
    expect(text()).toContain("CTranslate2"); expect(text()).toContain("ggml .bin");
    expect(renderer!.root.findAllByType("input")).toHaveLength(2);
    expect(renderer!.root.findAllByType("input").every((input) => input.props.disabled)).toBe(true);
    expect(renderer!.root.findAllByType("select").every((select) => select.props.disabled)).toBe(true);
    expect(renderer!.root.findAllByType("button").every((button) => button.props.disabled)).toBe(true);
  });

  it("preserves existing advanced paths without selecting or rewriting managed components", () => {
    mount({ ...defaultAsrConfig, engine: "qwen3-asr", python_path: "/old/python", model_path: "/old/qwen", aligner_path: "/old/aligner", device: "cuda" });
    expect(renderer!.root.findAllByType("input").map((input) => input.props.value)).toEqual(["/old/python", "/old/qwen", "/old/aligner"]);
    const advanced = renderer!.root.findAllByType("details").find((details) => details.findAllByType("summary").some((summary) => summary.children.includes(t("asr.managed.advanced"))))!;
    expect(advanced.props.open).toBeUndefined(); expect(text()).toContain(t("asr.qwenCpuMemory"));
    selectEngine("whisper-cpp"); selectEngine("qwen3-asr");
    expect(renderer!.root.findAllByType("input").map((input) => input.props.value)).toEqual(["/old/python", "/old/qwen", "/old/aligner"]);
    expect(mocks.check).not.toHaveBeenCalled();
  });

  it("surfaces unknown engines instead of rendering misleading Whisper controls", () => {
    mount({ ...defaultAsrConfig, engine: "new-future-engine" });
    expect(text()).toContain("new-future-engine"); expect(text()).toContain(t("requirement.unsupportedAsrEngine"));
    expect(renderer!.root.findAllByType("input")).toHaveLength(0);
    expect(renderer!.root.findAllByProps({ "aria-label": "legacy-model" })).toHaveLength(0);
  });
});
