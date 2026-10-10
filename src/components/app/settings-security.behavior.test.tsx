import { useState, type ReactNode } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { ModelApiSettingsCard } from "./settings";
import { defaultSettings, whisperModelPresets, translationLocalModelPresets } from "@/config";
import { enUS } from "@/locales/en-US";
import type { SettingsState, TFunction } from "@/types";
import * as api from "@/lib/tauri-api";
vi.mock("@/lib/tauri-api", () => ({ checkTranslationCli: vi.fn(), listTranslationCliModels: vi.fn() }));
vi.mock("./asr-settings-fields", () => ({ AsrSettingsFields: ({ children }: {
        children: ReactNode;
    }) => <div>{children}</div> }));
vi.mock("./shared", () => ({ DownloadProgress: () => null, StatusBadge: () => null, SectionTitle: ({ title }: {
        title: string;
    }) => <h2>{title}</h2>, FieldBlock: ({ label, children }: {
        label: string;
        children: ReactNode;
    }) => <div>{label}{children}</div>, IconAction: ({ label, children, ...props }: {
        label: string;
        children: ReactNode;
    }) => <button aria-label={label} {...props}>{children}</button> }));
vi.mock("@/components/ui/checkbox", () => ({ Checkbox: ({ checked, onCheckedChange }: {
        checked: boolean;
        onCheckedChange: (v: boolean) => void;
    }) => <input type="checkbox" checked={checked} onChange={(event) => onCheckedChange(event.target.checked)}/> }));
vi.mock("@/components/ui/select", () => ({ Select: ({ value, onValueChange, children }: {
        value: string;
        onValueChange: (v: string) => void;
        children: ReactNode;
    }) => <select value={value} onChange={(event) => onValueChange(event.target.value)}>{children}</select>, SelectTrigger: ({ children }: {
        children: ReactNode;
    }) => <>{children}</>, SelectValue: () => null, SelectContent: ({ children }: {
        children: ReactNode;
    }) => <>{children}</>, SelectGroup: ({ children }: {
        children: ReactNode;
    }) => <>{children}</>, SelectItem: ({ value, children }: {
        value: string;
        children: ReactNode;
    }) => <option value={value}>{children}</option> }));
const t: TFunction = (key, values) => Object.entries(values ?? {}).reduce<string>((text, [key, value]) => text.replace(`{${key}}`, String(value)), enUS[key as keyof typeof enUS] ?? key);
let renderer: ReactTestRenderer | undefined;
let current: SettingsState;
function Harness({ initial }: {
    initial: SettingsState;
}) {
    const [settings, setSettings] = useState(initial);
    const [apiKey, setApiKey] = useState("");
    current = settings;
    return <ModelApiSettingsCard settings={settings} setSettings={setSettings} apiKey={apiKey} setApiKey={setApiKey} hasApiCredential={false} downloadedTranslationModelFiles={new Set()} downloadedWhisperModelFiles={new Set()} llamaInstalling={false} llamaReady={false} modelDownload={null} modelDownloading={false} selectedTranslationPreset={translationLocalModelPresets[0]} selectedWhisperPreset={whisperModelPresets[0]} t={t} translationPresetId={translationLocalModelPresets[0].id} whisperPresetId={whisperModelPresets[0].id} onDownloadTranslationPreset={() => { }} onDownloadWhisperPreset={() => { }} onInstallLocalTranslation={() => { }} onPickTranslationModel={() => { }} onPickWhisperModel={() => { }} onSaveSettings={() => { }} setTranslationPresetId={() => { }} setWhisperPresetId={() => { }}/>;
}
async function mount(initial: SettingsState) { await act(async () => { renderer = create(<Harness initial={initial}/>); }); }
const text = () => JSON.stringify(renderer!.toJSON());
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("window", { __TAURI_INTERNALS__: {} }); });
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; vi.unstubAllGlobals(); });
describe("security settings UX", () => {
    it("shows the reviewed version and unsupported-version errors instead of ready status", async () => {
        await mount({ ...defaultSettings, translation_provider: "cli", translation_cli_model: "openai/gpt-4o-mini" });
        expect(text()).toContain("1.18.35");
        expect(text()).toContain("blocks tools, MCP and subagents");
        vi.mocked(api.checkTranslationCli).mockResolvedValue({ available: false, path: "/opencode", version: "1.18.34", error: "Unsupported OpenCode version. Use reviewed 1.18.35." });
        await act(async () => { await renderer!.root.findAllByType("button").find(b => b.props["aria-label"] === t("settings.cliCheck"))!.props.onClick(); });
        expect(text()).toContain("Unsupported OpenCode version");
        expect(text()).toContain("CLI unavailable");
        expect(text()).not.toContain("CLI available ·");
    });
    it("surfaces a bounded model-list probe failure and keeps the current model", async () => {
        await mount({ ...defaultSettings, translation_provider: "cli", translation_cli_model: "openai/gpt-4o-mini" });
        vi.mocked(api.listTranslationCliModels).mockRejectedValue(new Error("CLI model probe exceeded its time/output limit"));
        await act(async () => { await renderer!.root.findAllByType("button").find(b => b.props["aria-label"] === t("settings.cliLoadModels"))!.props.onClick(); });
        expect(text()).toContain("time/output limit");
        expect(current.translation_cli_model).toBe("openai/gpt-4o-mini");
    });
    it("requires explicit legacy binding and resets that choice if the origin is edited", async () => {
        await mount({ ...defaultSettings, legacy_api_key_available: true });
        expect(text()).toContain("previously saved, unbound key belongs to https://api.openai.com");
        expect(current.bind_legacy_api_key).not.toBe(true);
        const checkboxes = renderer!.root.findAllByType("input").filter(input => input.props.type === "checkbox");
        act(() => checkboxes[checkboxes.length - 1]!.props.onChange({ target: { checked: true } }));
        expect(current.bind_legacy_api_key).toBe(true);
        const url = renderer!.root.findAllByType("input").find(input => input.props.value === "https://api.openai.com")!;
        act(() => url.props.onChange({ target: { value: "https://another.test" } }));
        expect(current.bind_legacy_api_key).toBe(false);
        expect(text()).toContain("https://another.test");
    });
});
