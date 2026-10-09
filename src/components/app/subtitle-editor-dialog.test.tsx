import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SubtitleEditorDialog } from "./subtitle-editor-dialog";
import { saveSourceSubtitles, saveTranslatedSubtitles } from "@/lib/tauri-api";
import type { SubtitlePreview, TaskRecord, TFunction } from "@/types";

vi.mock("react-router-dom", () => ({ useBlocker: () => ({ state: "unblocked" }) }));
vi.mock("@/lib/tauri-api", () => ({ saveSourceSubtitles: vi.fn(), saveTranslatedSubtitles: vi.fn() }));
vi.mock("@/components/ui/scroll-area", () => ({ ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
vi.mock("@/components/ui/dialog", () => {
  const Container = ({ children }: { children: React.ReactNode }) => <div>{children}</div>;
  return { Dialog: Container, DialogContent: Container, DialogDescription: Container, DialogFooter: Container, DialogHeader: Container, DialogTitle: Container };
});
vi.mock("@/components/ui/alert-dialog", () => {
  const Container = ({ children }: { children: React.ReactNode }) => <div>{children}</div>;
  const Action = ({ children, onClick }: { children: React.ReactNode; onClick: () => void }) => <button onClick={onClick}>{children}</button>;
  return { AlertDialog: ({ children, open }: { children: React.ReactNode; open: boolean }) => open ? <div>{children}</div> : null, AlertDialogAction: Action, AlertDialogCancel: Action, AlertDialogContent: Container, AlertDialogDescription: Container, AlertDialogFooter: Container, AlertDialogHeader: Container, AlertDialogTitle: Container };
});
const t: TFunction = (key) => key;
const preview: SubtitlePreview = { source_srt: "original source", source_file_name: "source.srt", source_segments: [{ id: 4, start_ms: 0, end_ms: 1000, text: "Source" }], translated_srt: "original translation", translated_segments: [{ id: 4, start_ms: 0, end_ms: 1000, text: "Translation" }] };
let renderer: ReactTestRenderer;
const onClose = vi.fn(); const onSaved = vi.fn(async () => {});
function button(label: string) { return renderer.root.findAllByType("button").find((node) => node.children.includes(label))!; }
function mount(kind: "source" | "translated" = "translated") { act(() => { renderer = create(<SubtitleEditorDialog taskId="task-1" preview={preview} kind={kind} busy={false} t={t} onClose={onClose} onSaved={onSaved} onReturnFocus={() => {}} />); }); }
function change(value: string) { act(() => renderer.root.findByType("textarea").props.onChange({ target: { value } })); }
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn() }); });
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
    expect(saveTranslatedSubtitles).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
    expect(button("subtitle.saving").props.disabled).toBe(true);
    await act(async () => { resolve({ id: "task-1" } as TaskRecord); });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
