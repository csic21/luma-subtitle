import { useEffect, useState, type ReactNode } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsPage } from "./SettingsPage";
import { Tabs } from "@/components/ui/tabs";
import { defaultSettings, translationLocalModelPresets, whisperModelPresets } from "@/config";
import { enUS } from "@/locales/en-US";
import type { AppUpdateState, SettingsState, TFunction } from "@/types";
import * as api from "@/lib/tauri-api";

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(), save: vi.fn(), install: vi.fn(), refresh: vi.fn(), open: vi.fn(),
  checkUpdate: vi.fn(), installUpdate: vi.fn(), asrMount: vi.fn(), asrUnmount: vi.fn(),
}));
vi.mock("react-router-dom", () => ({ useNavigate: () => mocks.navigate }));
vi.mock("@/i18n", () => ({ useI18n: () => ({ t }) }));
vi.mock("@/lib/tauri-api", () => ({ checkTranslationCli: vi.fn(), listTranslationCliModels: vi.fn() }));
vi.mock("@/hooks/use-settings-page-state", () => ({ useSettingsPageState: () => {
  const [settings, setSettings] = useState<SettingsState>({ ...defaultSettings });
  const [apiKey, setApiKey] = useState("");
  current = settings;
  return {
    settings, setSettings, apiKey, setApiKey, saveSettings: () => mocks.save(settings, apiKey),
    env: { sidecar_dir: "/luma/tools" }, envRows: [["FFmpeg", "/luma/tools/ffmpeg"]],
    environmentReady: false, refreshEnvironment: mocks.refresh, openManagedDir: mocks.open,
    installDependencies: mocks.install, dependencyInstall: null, dependencyInstalling: false,
    appUpdate: update, appUpdating: false, checkForUpdates: mocks.checkUpdate, installUpdate: mocks.installUpdate,
    hasApiCredential: false, llamaReady: false, modelDownload: null, modelDownloading: false,
    downloadedWhisperModelFiles: new Set(), downloadedTranslationModelFiles: new Set(),
    selectedWhisperPreset: whisperModelPresets[0], selectedTranslationPreset: translationLocalModelPresets[0],
    whisperPresetId: whisperModelPresets[0].id, translationPresetId: translationLocalModelPresets[0].id,
    setWhisperPresetId: vi.fn(), setTranslationPresetId: vi.fn(), pickWhisperModel: vi.fn(), downloadWhisperPreset: vi.fn(),
    pickTranslationModel: vi.fn(), downloadTranslationPreset: vi.fn(), installLocalTranslation: vi.fn(), notice: "",
  };
} }));
vi.mock("@/components/app/asr-settings-fields", () => ({ AsrSettingsFields: ({ children }: { children: ReactNode }) => {
  useEffect(() => { mocks.asrMount(); return () => { mocks.asrUnmount(); }; }, []);
  return <div>{children}</div>;
} }));
vi.mock("@/components/app/shared", () => ({
  NoticeAlert: () => null, DownloadProgress: () => null,
  StatusBadge: ({ label }: { label: string }) => <span>{label}</span>,
  SectionTitle: ({ title }: { title: string }) => <h2>{title}</h2>,
  FieldBlock: ({ label, children }: { label: string; children: ReactNode }) => <div>{label}{children}</div>,
  IconAction: ({ label, children, ...props }: { label: string; children: ReactNode }) => <button aria-label={label} {...props}>{children}</button>,
}));
vi.mock("@/components/ui/checkbox", () => ({ Checkbox: ({ checked, onCheckedChange }: { checked: boolean; onCheckedChange: (value: boolean) => void }) => <input type="checkbox" checked={checked} onChange={(event) => onCheckedChange(event.target.checked)} /> }));
vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, children }: { value: string; onValueChange: (value: string) => void; children: ReactNode }) => <select value={value} onChange={(event) => onValueChange(event.target.value)}>{children}</select>,
  SelectTrigger: ({ children }: { children: ReactNode }) => <>{children}</>, SelectValue: () => null,
  SelectContent: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectGroup: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectItem: ({ value, children }: { value: string; children: ReactNode }) => <option value={value}>{children}</option>,
}));

const t: TFunction = (key, values) => Object.entries(values ?? {}).reduce<string>((text, [name, value]) => text.replace(`{${name}}`, String(value)), enUS[key as keyof typeof enUS] ?? key);
let renderer: ReactTestRenderer | undefined;
let current: SettingsState;
let update: AppUpdateState;
const text = () => JSON.stringify(renderer!.toJSON());
const button = (label: string) => renderer!.root.findAllByType("button").find((node) => node.props["aria-label"] === label || node.children.includes(label))!;
const panel = (name: string) => renderer!.root.findAllByProps({ role: "tabpanel" }).find((node) => node.props.id.endsWith(`-${name}`))!;
function switchSection(value: string) { act(() => renderer!.root.findByType(Tabs).props.onValueChange(value)); }
async function mount() { await act(async () => { renderer = create(<SettingsPage />); }); }

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  vi.stubGlobal("requestAnimationFrame", (callback: () => void) => setTimeout(callback, 0));
  vi.stubGlobal("cancelAnimationFrame", clearTimeout);
  update = { status: "available", currentVersion: "1.2.3", availableVersion: "1.2.4", progress: 0 };
});
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; vi.unstubAllGlobals(); });

describe("settings presentation interactions", () => {
  it("preserves API drafts and the mounted ASR setup across tabs, then saves all drafts from the header", async () => {
    await mount();
    expect(panel("models").props.hidden).toBe(false);
    expect(panel("environment").props.hidden).toBe(true);
    const url = renderer!.root.findAllByType("input").find((node) => node.props["aria-label"] === "Base URL")!;
    act(() => url.props.onChange({ target: { value: "https://translation.example" } }));
    const key = renderer!.root.findAllByType("input").find((node) => node.props.type === "password")!;
    act(() => key.props.onChange({ target: { value: "test-only-key" } }));
    switchSection("environment");
    expect(panel("models").props.hidden).toBe(true);
    expect(panel("environment").props.hidden).toBe(false);
    act(() => button(t("common.save")).props.onClick());
    expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ base_url: "https://translation.example" }), "test-only-key");
    switchSection("updates"); switchSection("models");
    expect(renderer!.root.findAllByType("input").find((node) => node.props.type === "password")!.props.value).toBe("test-only-key");
    expect(current.base_url).toBe("https://translation.example");
    expect(mocks.asrMount).toHaveBeenCalledTimes(1);
    expect(mocks.asrUnmount).not.toHaveBeenCalled();
    expect(mocks.install).not.toHaveBeenCalled();
  });

  it("preserves CLI probe errors across tab switches and keeps advanced options connected", async () => {
    await mount();
    const provider = renderer!.root.findAllByType("select").find((node) => node.props.value === "api")!;
    act(() => provider.props.onChange({ target: { value: "cli" } }));
    vi.mocked(api.checkTranslationCli).mockResolvedValue({ available: false, path: null, version: null, error: "Probe failed safely" });
    await act(async () => { await button(t("settings.cliCheck")).props.onClick(); });
    switchSection("updates"); switchSection("models");
    expect(text()).toContain("Probe failed safely");
    expect(api.checkTranslationCli).toHaveBeenCalledTimes(1);
    const advanced = renderer!.root.findAllByType("details").find((node) => node.findAllByType("summary").some((summary) => summary.children.includes(t("settings.advanced"))))!;
    expect(advanced.props.open).toBeUndefined();
    const shard = advanced.findAllByType("input").find((node) => node.props.max === 1000)!;
    act(() => shard.props.onChange({ target: { value: "24" } }));
    expect(current.translation_shard_size).toBe(24);
  });

  it("retains explicit environment, update, help, and back actions", async () => {
    await mount(); switchSection("environment");
    expect(text()).toContain(t("env.requiredMissing"));
    expect(renderer!.root.findAllByType("summary").some((node) => node.children.includes(t("env.details")))).toBe(true);
    expect(text()).toContain("/luma/tools/ffmpeg");
    expect(text()).toContain(t("guide.stepRunTitle"));
    act(() => button(t("common.refresh")).props.onClick());
    act(() => button(t("common.openDir")).props.onClick());
    act(() => button(t("download.installDependencies")).props.onClick());
    expect(mocks.refresh).toHaveBeenCalledTimes(1);
    expect(mocks.open).toHaveBeenCalledWith("/luma/tools");
    expect(mocks.install).toHaveBeenCalledTimes(1);
    switchSection("updates");
    expect(panel("updates").props.hidden).toBe(false);
    expect(button(t("update.install")).props.disabled).toBe(false);
    act(() => button(t("update.check")).props.onClick());
    act(() => button(t("update.install")).props.onClick());
    expect(mocks.checkUpdate).toHaveBeenCalledTimes(1);
    expect(mocks.installUpdate).toHaveBeenCalledTimes(1);
    act(() => button(t("common.backToQueue")).props.onClick());
    expect(mocks.navigate).toHaveBeenCalledWith("/tasks");
  });
});
