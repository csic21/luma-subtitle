import type { ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SubtitleEditorDialog } from "./subtitle-editor-dialog";
import { SubtitleSourceEditor } from "./subtitle-source-editor";
import { saveSourceSubtitles, saveTranslatedSubtitles } from "@/lib/tauri-api";
import type { SubtitlePreview, TaskRecord, TFunction } from "@/types";

const router = vi.hoisted(() => ({
  current: { state: "unblocked" as "unblocked" | "blocked", reset: vi.fn(), proceed: vi.fn() },
  guard: vi.fn(),
}));
vi.mock("react-router-dom", () => ({ useBlocker: (guard: boolean) => { router.guard(guard); return router.current; } }));
vi.mock("@/lib/tauri-api", () => ({ saveSourceSubtitles: vi.fn(), saveTranslatedSubtitles: vi.fn() }));
vi.mock("@/components/ui/scroll-area", () => ({ ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
vi.mock("@/components/ui/dialog", () => {
  const container = (name: string) => ({ children, ...props }: { children: React.ReactNode }) => <div data-test-component={name} {...props}>{children}</div>;
  return { Dialog: container("dialog"), DialogContent: container("dialog-content"), DialogDescription: container("description"), DialogFooter: container("footer"), DialogHeader: container("header"), DialogTitle: container("title") };
});
vi.mock("@/components/ui/alert-dialog", () => {
  const container = (name: string) => ({ children, ...props }: { children: React.ReactNode }) => <div data-test-component={name} {...props}>{children}</div>;
  const Action = ({ children, onClick }: { children: React.ReactNode; onClick: () => void }) => <button onClick={onClick}>{children}</button>;
  return { AlertDialog: ({ children, open, ...props }: { children: React.ReactNode; open: boolean }) => open ? <div data-test-component="alert-dialog" {...props}>{children}</div> : null, AlertDialogAction: Action, AlertDialogCancel: Action, AlertDialogContent: container("alert-content"), AlertDialogDescription: container("alert-description"), AlertDialogFooter: container("alert-footer"), AlertDialogHeader: container("alert-header"), AlertDialogTitle: container("alert-title") };
});
const t: TFunction = (key) => key;
const preview: SubtitlePreview = { source_srt: "original source", source_file_name: "source.srt", source_segments: [{ id: 4, start_ms: 0, end_ms: 1000, text: "Source" }], translated_srt: "original translation", translated_segments: [{ id: 4, start_ms: 0, end_ms: 1000, text: "Translation" }] };
let renderer: ReactTestRenderer;
let props: ComponentProps<typeof SubtitleEditorDialog>;
const onClose = vi.fn(); const onSaved = vi.fn(async () => {}); const onReturnFocus = vi.fn();
function button(label: string) { return renderer.root.findAllByType("button").find((node) => node.children.includes(label))!; }
function component(name: string) { return renderer.root.findByProps({ "data-test-component": name }); }
function mount(kind: "source" | "translated" = "translated", overrides: Partial<typeof props> = {}) {
  props = { taskId: "task-1", preview, kind, busy: false, t, onClose, onSaved, onReturnFocus, ...overrides };
  act(() => { renderer = create(<SubtitleEditorDialog {...props} />); });
}
function update(overrides: Partial<typeof props> = {}) { props = { ...props, ...overrides }; act(() => renderer.update(<SubtitleEditorDialog {...props} />)); }
function change(value: string) { act(() => renderer.root.findByType("textarea").props.onChange({ target: { value } })); }
beforeEach(() => {
  vi.clearAllMocks();
  router.current = { state: "unblocked", reset: vi.fn(), proceed: vi.fn() };
  vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn() });
});
afterEach(() => { act(() => renderer?.unmount()); vi.unstubAllGlobals(); });

describe("subtitle editing", () => {
  it("saves translation corrections with both snapshots and never changes the source", async () => {
    mount(); change("Corrected");
    vi.mocked(saveTranslatedSubtitles).mockResolvedValue({ id: "task-1" } as TaskRecord);
    await act(async () => button("subtitle.saveTranslated").props.onClick());
    expect(saveTranslatedSubtitles).toHaveBeenCalledExactlyOnceWith("task-1", "original source", "original translation", [{ id: 4, text: "Corrected" }]);
    expect(saveSourceSubtitles).not.toHaveBeenCalled();
    expect(onSaved).toHaveBeenCalledWith({ id: "task-1" }, "translated");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("saves source corrections with the original source snapshot", async () => {
    mount("source"); change("Corrected source");
    vi.mocked(saveSourceSubtitles).mockResolvedValue({ id: "task-1" } as TaskRecord);
    await act(async () => button("subtitle.saveSource").props.onClick());
    expect(saveSourceSubtitles).toHaveBeenCalledExactlyOnceWith("task-1", "original source", [{ id: 4, text: "Corrected source" }]);
    expect(saveTranslatedSubtitles).not.toHaveBeenCalled();
  });

  it("retains edits and keeps the editor open on a conflicting snapshot", async () => {
    mount(); change("Keep my correction");
    vi.mocked(saveTranslatedSubtitles).mockRejectedValue(new Error("Subtitle changed"));
    await act(async () => button("subtitle.saveTranslated").props.onClick());
    expect(onClose).not.toHaveBeenCalled();
    expect(renderer.root.findByType("textarea").props.value).toBe("Keep my correction");
    expect(JSON.stringify(renderer.toJSON())).toContain("Subtitle changed");
  });

  it("blocks blank subtitle saves and asks before discarding a draft", () => {
    mount("source"); change("   ");
    expect(button("subtitle.saveSource").props.disabled).toBe(true);
    act(() => button("common.cancel").props.onClick());
    expect(onClose).not.toHaveBeenCalled();
    expect(button("subtitle.discard")).toBeDefined();
    act(() => button("subtitle.discard").props.onClick({ preventDefault: () => {} }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("ignores repeated Save and Cancel actions while persistence is pending", async () => {
    mount(); change("One save only");
    let resolve!: (task: TaskRecord) => void;
    vi.mocked(saveTranslatedSubtitles).mockReturnValue(new Promise((done) => { resolve = done; }));
    const save = button("subtitle.saveTranslated").props.onClick;
    act(() => { save(); save(); });
    act(() => button("common.cancel").props.onClick());
    act(() => component("dialog").props.onOpenChange(false));
    expect(saveTranslatedSubtitles).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
    expect(button("subtitle.saving").props.disabled).toBe(true);
    await act(async () => { resolve({ id: "task-1" } as TaskRecord); });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("returns Keep editing to the exact previously focused textarea, preserving the draft", () => {
    mount(); change("Still editing");
    const textarea = { tagName: "TEXTAREA", isConnected: true, focus: vi.fn() };
    act(() => component("dialog-content").props.onFocusCapture({ target: textarea }));
    // Clicking Cancel must not replace the saved textarea with the button.
    act(() => component("dialog-content").props.onFocusCapture({ target: { tagName: "BUTTON" } }));
    act(() => button("common.cancel").props.onClick());
    const closeAutoFocus = component("alert-content").props.onCloseAutoFocus;
    act(() => button("subtitle.keepEditing").props.onClick({ preventDefault: vi.fn() }));
    const preventDefault = vi.fn();
    act(() => closeAutoFocus({ preventDefault }));
    expect(preventDefault).toHaveBeenCalled();
    expect(textarea.focus).toHaveBeenCalledExactlyOnceWith({ preventScroll: true });
    expect(renderer.root.findByType("textarea").props.value).toBe("Still editing");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("guards Back navigation and resets the blocked route when keeping the draft", () => {
    mount(); change("Unsaved");
    expect(router.guard).toHaveBeenLastCalledWith(true);
    router.current = { ...router.current, state: "blocked" };
    update();
    act(() => button("subtitle.keepEditing").props.onClick({ preventDefault: vi.fn() }));
    expect(router.current.reset).toHaveBeenCalledTimes(1);
    expect(router.current.proceed).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("proceeds with blocked Back navigation only after explicit discard", () => {
    mount(); change("Unsaved");
    router.current = { ...router.current, state: "blocked" };
    update();
    act(() => button("subtitle.discard").props.onClick({ preventDefault: vi.fn() }));
    expect(router.current.proceed).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("keeps Back navigation blocked while saving rather than opening discard", async () => {
    mount(); change("Saving");
    let resolve!: (task: TaskRecord) => void;
    vi.mocked(saveTranslatedSubtitles).mockReturnValue(new Promise((done) => { resolve = done; }));
    act(() => button("subtitle.saveTranslated").props.onClick());
    router.current = { ...router.current, state: "blocked" };
    update();
    expect(router.current.reset).toHaveBeenCalledTimes(1);
    expect(renderer.root.findAllByProps({ "data-test-component": "alert-dialog" })).toHaveLength(0);
    await act(async () => { resolve({ id: "task-1" } as TaskRecord); });
  });

  it("installs and removes the dirty beforeunload guard when text is restored", () => {
    mount(); change("Unsaved");
    expect(window.addEventListener).toHaveBeenCalledWith("beforeunload", expect.any(Function));
    const handler = vi.mocked(window.addEventListener).mock.calls[0][1] as (event: BeforeUnloadEvent) => void;
    const event = { preventDefault: vi.fn(), returnValue: undefined as string | undefined };
    handler(event as unknown as BeforeUnloadEvent);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(event.returnValue).toBe("");
    change("Translation");
    expect(window.removeEventListener).toHaveBeenCalledWith("beforeunload", handler);
    expect(router.guard).toHaveBeenLastCalledWith(false);
  });

  it("pins both snapshots and draft content across incoming task updates", async () => {
    mount(); change("Local draft");
    update({ preview: { ...preview, source_srt: "new source snapshot", translated_srt: "new target snapshot", translated_segments: [{ ...preview.translated_segments![0], text: "New remote text" }] } });
    expect(renderer.root.findByType("textarea").props.value).toBe("Local draft");
    vi.mocked(saveTranslatedSubtitles).mockResolvedValue({ id: "task-1" } as TaskRecord);
    await act(async () => button("subtitle.saveTranslated").props.onClick());
    expect(saveTranslatedSubtitles).toHaveBeenCalledWith("task-1", "original source", "original translation", [{ id: 4, text: "Local draft" }]);
  });

  it("forwards the initial cue and preserves the draft while layout props rerender", () => {
    mount("translated", { initialCueId: 4 });
    expect(renderer.root.findByType(SubtitleSourceEditor).props.initialCueId).toBe(4);
    const preventDefault = vi.fn();
    component("dialog-content").props.onOpenAutoFocus({ preventDefault });
    expect(preventDefault).toHaveBeenCalled();
    change("Draft across resize");
    update({ t: (key) => key });
    expect(renderer.root.findByType("textarea").props.value).toBe("Draft across resize");
  });

  it("disables saving ambiguous duplicate IDs without overwriting either row", () => {
    mount("source", { preview: { ...preview, source_segments: [preview.source_segments[0], { ...preview.source_segments[0], text: "Duplicate" }] } });
    expect(renderer.root.findAllByType("textarea").map((field) => field.props.value)).toEqual(["Source", "Duplicate"]);
    expect(button("subtitle.saveSource").props.disabled).toBe(true);
    act(() => button("subtitle.saveSource").props.onClick());
    expect(saveSourceSubtitles).not.toHaveBeenCalled();
  });

  it("closes a clean editor immediately and returns focus to its opener", () => {
    mount();
    act(() => button("common.cancel").props.onClick());
    expect(onClose).toHaveBeenCalledTimes(1);
    component("dialog-content").props.onCloseAutoFocus({ preventDefault: vi.fn() });
    expect(onReturnFocus).toHaveBeenCalledTimes(1);
  });
});
