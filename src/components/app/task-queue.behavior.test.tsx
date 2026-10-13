import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import type { ReactNode } from "react";
import { defaultSettings } from "@/config";
import type { TaskRecord, TFunction } from "@/types";
import { TaskMetricGrid, TaskQueueTable, TaskRowMenu, TaskToolbar } from "./tasks";

vi.mock("@/i18n", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: ReactNode }) => children,
  TooltipTrigger: ({ children }: { children: ReactNode }) => children,
  TooltipContent: () => null,
}));
// Expose closed menu contents to inspect application permissions and handlers.
// Browser QA covers the real portal, keyboard dismissal, and focus return.
vi.mock("radix-ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("radix-ui")>();
  const Children = ({ children }: { children: ReactNode }) => children;
  return { ...actual, DropdownMenu: {
    Root: Children, Trigger: Children, Portal: Children, Group: Children,
    Content: ({ children }: { children: ReactNode }) => <div role="menu">{children}</div>,
    Item: ({ children, onSelect, ...props }: { children: ReactNode; onSelect?: () => void }) => <button {...props} onClick={onSelect}>{children}</button>,
    Separator: () => <hr />,
  } };
});

const t: TFunction = (key, values) => values ? `${key}:${Object.values(values).join(",")}` : key;
const context = { environmentReady: true, hasApiCredential: true, llamaReady: true };
const task: TaskRecord = {
  id: "one", source_type: "video", file_name: "Interview.mp4", video_path: "/media/Interview.mp4",
  status: "idle", stage: "created", progress: 0, message: "Ready", created_at: 1, updated_at: 1,
  settings: { ...defaultSettings, whisper_model_path: "/models/whisper.bin" },
};
let renderer: ReactTestRenderer | undefined;
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; });
function button(label: string) {
  return renderer!.root.findAllByType("button").find((node) => node.findAll((child) => typeof child.type === "string" && child.children.includes(label)).length > 0 || node.children.includes(label))!;
}
const toolbarProps = () => ({
  canRunSelectedOperations: { transcribe: true, translate: false, resume_translate: false, export: true },
  outputDir: "/output", queueSettings: { max_concurrency: 2, auto_start_next: false }, selectedCount: 0,
  summary: <TaskMetricGrid busyCount={1} doneCount={2} failedCount={0} taskCount={3} t={t} />, t,
  onCancelSelected: vi.fn(), onCreateAudioTask: vi.fn(), onCreateSrtTask: vi.fn(), onCreateVideoTask: vi.fn(),
  onPickOutputDir: vi.fn(), onRefreshTasks: vi.fn(), onRunSelected: vi.fn(), onSaveAutoStartNext: vi.fn(), onSaveConcurrency: vi.fn(),
});

function queueProps() {
  return {
    allSelected: false, locale: "en-US" as const, operationContext: context,
    selectedIds: new Set<string>(), tasks: [task], t,
    onCancelTask: vi.fn(), onDeleteTask: vi.fn(), onOpenTask: vi.fn(), onRunOperation: vi.fn(),
    onToggleAll: vi.fn(), onToggleTask: vi.fn(),
  };
}

describe("task queue workbench", () => {
  it("uses a compact semantic summary and only exposes batch actions for a selection", () => {
    const props = toolbarProps();
    act(() => { renderer = create(<TaskToolbar {...props} />); });
    expect(renderer!.root.findAllByType("dl")).toHaveLength(1);
    expect(renderer!.root.findAllByType("dd").map((node) => node.children.join(""))).toEqual(["3", "1", "2", "0"]);
    expect(renderer!.root.findAllByProps({ className: "queue-batch-actions" })).toHaveLength(0);
    act(() => { renderer!.update(<TaskToolbar {...props} selectedCount={2} />); });
    expect(renderer!.root.findByProps({ "aria-label": "task.selected:2" })).toBeTruthy();
    expect(button("common.translate").props.disabled).toBe(true);
    expect(button("common.transcribe").props.disabled).toBe(false);
    act(() => { button("common.transcribe").props.onClick(); button("common.export").props.onClick(); button("common.cancel").props.onClick(); });
    expect(props.onRunSelected.mock.calls).toEqual([["transcribe"], ["export"]]);
    expect(props.onCancelSelected).toHaveBeenCalledOnce();
    act(() => { renderer!.update(<TaskToolbar {...props} selectedCount={0} />); });
    expect(renderer!.root.findAllByProps({ className: "queue-batch-actions" })).toHaveLength(0);
  });

  it("keeps all imports, output selection, refresh, concurrency and auto-next wired", () => {
    const props = toolbarProps();
    act(() => { renderer = create(<TaskToolbar {...props} />); });
    act(() => {
      button("task.videoNew").props.onClick(); button("task.audioNew").props.onClick(); button("task.srtImport").props.onClick();
      button("task.inputDir").props.onClick();
      renderer!.root.findAllByType("button").find((node) => node.props["aria-label"] === "common.refresh")!.props.onClick();
      renderer!.root.findAllByType("input").find((node) => node.props.id === "max-concurrency")!.props.onChange({ target: { value: "4" } });
      renderer!.root.findAll((node) => node.props.id === "auto-start-next" && typeof node.props.onCheckedChange === "function")[0].props.onCheckedChange(true);
    });
    [props.onCreateVideoTask, props.onCreateAudioTask, props.onCreateSrtTask, props.onPickOutputDir, props.onRefreshTasks].forEach((handler) => expect(handler).toHaveBeenCalledOnce());
    expect(props.onSaveConcurrency).toHaveBeenCalledWith(4);
    expect(props.onSaveAutoStartNext).toHaveBeenCalledWith(true);
  });

  it("shows mixed select-all state and keeps selection independent of opening a task", () => {
    const props = queueProps();
    act(() => { renderer = create(<TaskQueueTable {...props} selectedIds={new Set(["one"])} tasks={[task, { ...task, id: "two" }]} />); });
    const checkboxes = renderer!.root.findAllByType("button").filter((node) => node.props.role === "checkbox");
    expect(checkboxes[0].props["aria-checked"]).toBe("mixed");
    expect(renderer!.root.findAllByType("tr").filter((node) => node.props["aria-selected"] === true)).toHaveLength(1);
    act(() => { renderer!.root.findAllByType("button").find((node) => node.props.title === task.file_name)!.props.onClick(); });
    expect(props.onOpenTask).toHaveBeenCalledWith("one");
    expect(props.onToggleTask).not.toHaveBeenCalled();
    act(() => { renderer!.root.findAll((node) => node.props["aria-label"] === "task.ariaSelectAll" && typeof node.props.onCheckedChange === "function")[0].props.onCheckedChange(true); });
    expect(props.onToggleAll).toHaveBeenCalledOnce();
    act(() => { renderer!.root.findAll((node) => node.props["aria-label"] === `task.ariaSelect:${task.file_name}` && typeof node.props.onCheckedChange === "function")[0].props.onCheckedChange(false); });
    expect(props.onToggleTask).toHaveBeenCalledWith("one");
  });

  it("keeps readiness explanations and blocks destructive actions while running", () => {
    const onRunOperation = vi.fn(); const onCancelTask = vi.fn(); const onDeleteTask = vi.fn();
    act(() => { renderer = create(<TaskRowMenu task={task} operationContext={{ ...context, environmentReady: false }} t={t} onRunOperation={onRunOperation} onCancelTask={onCancelTask} onDeleteTask={onDeleteTask} />); });
    expect(button("common.transcribe").props.disabled).toBe(true);
    expect(renderer!.root.findAllByType("small").some((node) => node.children.includes("requirement.missingEnvironment"))).toBe(true);
    expect(button("common.cancel").props.disabled).toBe(true);
    expect(button("common.delete").props.disabled).toBe(false);
    act(() => { button("common.delete").props.onClick(); });
    expect(onDeleteTask).toHaveBeenCalledWith("one");
    act(() => { renderer!.update(<TaskRowMenu task={{ ...task, status: "running" }} operationContext={context} t={t} onRunOperation={onRunOperation} onCancelTask={onCancelTask} onDeleteTask={onDeleteTask} />); });
    expect(button("common.delete").props.disabled).toBe(true);
    expect(button("common.transcribe").props.disabled).toBe(true);
    expect(button("common.cancel").props.disabled).toBe(false);
    act(() => { button("common.cancel").props.onClick(); });
    expect(onCancelTask).toHaveBeenCalledWith("one");
  });

  it("retains resumable translation and dispatches the precise operation and task", () => {
    const partial: TaskRecord = { ...task, status: "failed", source_srt_path: "/task/source.srt", translated_srt_path: "/task/translated.srt", translation_completed_count: 1, segment_count: 2 };
    const onRunOperation = vi.fn();
    act(() => { renderer = create(<TaskRowMenu task={partial} operationContext={context} t={t} onRunOperation={onRunOperation} onCancelTask={vi.fn()} onDeleteTask={vi.fn()} />); });
    expect(button("common.resumeTranslate")).toBeTruthy();
    expect(button("common.resumeTranslate").props.disabled).toBe(false);
    act(() => { button("common.resumeTranslate").props.onClick(); button("common.export").props.onClick(); button("common.transcribe").props.onClick(); });
    expect(onRunOperation.mock.calls).toEqual([["one", "resume_translate"], ["one", "export"], ["one", "transcribe"]]);
  });

  it("does not repeat a terminal status while preserving a distinct processing stage", () => {
    const translated: TFunction = (key) => key === "stage.completed" || key === "status.completed" ? "Completed" : key;
    const props = queueProps();
    act(() => { renderer = create(<TaskQueueTable {...props} t={translated} tasks={[{ ...task, status: "completed", stage: "completed" }]} />); });
    expect(renderer!.root.findByProps({ className: "queue-status-heading" }).findAllByType("span")).toHaveLength(1);
    act(() => { renderer!.update(<TaskQueueTable {...props} t={translated} tasks={[{ ...task, status: "running", stage: "transcribing" }]} />); });
    expect(renderer!.root.findByProps({ className: "queue-status-heading" }).findAllByType("span")).toHaveLength(2);
  });

  it("disables select-all for an empty queue and keeps the empty state readable", () => {
    act(() => { renderer = create(<TaskQueueTable {...queueProps()} tasks={[]} />); });
    expect(renderer!.root.findAllByType("button").find((node) => node.props.role === "checkbox")!.props.disabled).toBe(true);
    expect(renderer!.root.findByType("strong").children).toEqual(["task.empty"]);
  });

  it("preserves intrinsic sizing for the settings group and full-row selected tint", () => {
    const css = readFileSync(new URL("../../styles/queue.css", import.meta.url), "utf8");
    const settingsRule = css.match(/\.queue-settings-fields\[data-slot="field-group"\]\s*\{([^}]+)\}/)?.[1];
    expect(settingsRule).toContain("container-type: normal");
    expect(settingsRule).toContain("width: max-content");
    expect(settingsRule).toContain("flex: 0 0 auto");
    expect(css).toMatch(/\.queue-table tbody tr\[data-state="selected"\]\s*\{\s*background: var\(--selection\)/);
  });
});
