import { useEffect, useState } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskDetailPage } from "./TaskDetailPage";
import { defaultSettings } from "@/config";
import type { TaskRecord, TFunction } from "@/types";

const mocks = vi.hoisted(() => ({ run: vi.fn(), cancel: vi.fn(), navigate: vi.fn(), configMount: vi.fn(), configUnmount: vi.fn() }));
vi.mock("react-router-dom", () => ({ useNavigate: () => mocks.navigate, useParams: () => ({ taskId: "one" }) }));
vi.mock("@/i18n", () => ({ useI18n: () => ({ t }) }));
vi.mock("@/hooks/use-task-detail-state", () => ({ useTaskDetailState: () => state }));
vi.mock("@/components/app/shared", () => ({ NoticeAlert: ({ message }: { message: string }) => message ? <div role="alert">{message}</div> : null, StatusBadge: ({ status }: { status: string }) => <span>{status}</span>, SectionTitle: ({ title }: { title: string }) => <span>{title}</span> }));
vi.mock("@/components/app/task-detail-config", () => ({ TaskConfigCard: () => { const [draft, setDraft] = useState(""); useEffect(() => { mocks.configMount(); return () => mocks.configUnmount(); }, []); return <input aria-label="Model draft" value={draft} onChange={(event) => setDraft(event.target.value)} />; } }));
const t: TFunction = (key, values) => key + (values ? `:${Object.values(values).join(",")}` : "");
const task: TaskRecord = { id: "one", source_type: "video", file_name: "Film.mp4", video_path: "/media/Film.mp4", status: "completed", stage: "completed", progress: 100, message: "Finished", created_at: 1, updated_at: 1, source_srt_path: "/output/source.srt", translated_srt_path: "/output/translated.srt", settings: { ...defaultSettings, whisper_model_path: "/model.bin" } };
let renderer: ReactTestRenderer;
let state: ReturnType<typeof makeState>;
function makeState() { return { task, operationTask: task, taskConfig: task.settings, commandPending: false, taskSettingsDirty: false, notice: "", operationContext: { environmentReady: true, hasApiCredential: true, llamaReady: true }, flowSteps: [], logs: [], subtitlePreview: null, subtitleView: "parallel", hasTranslatedSubtitle: false, runOperation: mocks.run, cancelTask: mocks.cancel, refreshPreview: vi.fn(), setSubtitleView: vi.fn(), sourceSubtitlesSaved: vi.fn(), openOutputDir: vi.fn() }; }
function mount() { act(() => { renderer = create(<TaskDetailPage />); }); }
function rerender() { act(() => renderer.update(<TaskDetailPage />)); }
function button(label: string) { return renderer.root.findAllByType("button").find((node) => node.children.includes(label))!; }
beforeEach(() => { vi.clearAllMocks(); state = makeState(); });
afterEach(() => { act(() => renderer?.unmount()); });
describe("task workbench behavior", () => {
  it("keeps a queued notice from hiding a later task failure and retains a distinct save error", () => {
    state.notice = "Added to queue"; mount();
    state = { ...state, task: { ...task, status: "failed", error: "Engine stopped" }, operationTask: { ...task, status: "failed", error: "Engine stopped" } }; rerender();
    expect(renderer.root.findByProps({ role: "alert" }).children.join("")).toBe("Engine stopped\nAdded to queue");
    state.notice = "Saving failed"; rerender();
    expect(renderer.root.findByProps({ role: "alert" }).children.join("")).toBe("Engine stopped\nSaving failed");
    state.notice = "Engine stopped"; rerender();
    expect(renderer.root.findByProps({ role: "alert" }).children.join("")).toBe("Engine stopped");
  });
  it("keeps the inspector mounted with its edited draft across open/close", () => {
    mount();
    expect(renderer.root.findByType("aside").props.hidden).toBe(true);
    act(() => button("workbench.configure").props.onClick());
    act(() => renderer.root.findByProps({ "aria-label": "Model draft" }).props.onChange({ target: { value: "my-model" } }));
    act(() => renderer.root.findByProps({ "aria-label": "common.close" }).props.onClick());
    expect(renderer.root.findByType("aside").props.hidden).toBe(true);
    act(() => button("workbench.configure").props.onClick());
    expect(renderer.root.findByProps({ "aria-label": "Model draft" }).props.value).toBe("my-model");
    expect(mocks.configMount).toHaveBeenCalledTimes(1); expect(mocks.configUnmount).not.toHaveBeenCalled();
  });
  it("shows direct cancel while running and routes the existing cancel handler", () => {
    state.task = { ...task, status: "running", stage: "transcribing", progress: 30 }; state.operationTask = state.task; mount();
    expect(button("common.cancel").props.disabled).toBe(false);
    act(() => button("common.cancel").props.onClick()); expect(mocks.cancel).toHaveBeenCalledOnce();
    expect(renderer.root.findAllByType("details").some((node) => node.props.className === "operation-menu")).toBe(false);
  });
  it("gates export through existing prerequisites and forwards allowed operations", () => {
    state.task = { ...task, source_srt_path: null, translated_srt_path: null }; state.operationTask = state.task; mount();
    expect(button("common.exportSubtitles").props.disabled).toBe(true);
    expect(button("common.exportSubtitles").props.title).toBeTruthy();
    act(() => button("common.transcribe").props.onClick()); expect(mocks.run).toHaveBeenCalledWith("transcribe");
    state = { ...state, task, operationTask: task, taskSettingsDirty: true }; rerender();
    const saveExport = button("settings.saveAndRun:common.export");
    act(() => saveExport.props.onClick()); expect(mocks.run).toHaveBeenCalledWith("export");
  });
  it("preserves explicit keyboard access to logs and long status", () => {
    mount(); act(() => button("tabs.logs").props.onClick());
    expect(renderer.root.findByProps({ className: "workbench-logs" }).props.hidden).toBe(false);
    const logs = renderer.root.findAll((node) => node.props["data-slot"] === "scroll-area-viewport").find((node) => node.props["aria-label"] === "tabs.logs");
    expect(logs?.props.tabIndex).toBe(0);
    expect(renderer.root.findByProps({ className: "workbench-message" }).findByType("p").props.tabIndex).toBe(0);
  });
});
