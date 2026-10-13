import { useState, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, describe, expect, it, vi } from "vitest";
import { defaultSettings } from "@/config";
import { Tabs } from "@/components/ui/tabs";
import type { SubtitlePreview, TaskRecord, TFunction } from "@/types";
import { SubtitlePreviewCard } from "./task-detail";
import { SubtitleSourceEditor } from "./subtitle-source-editor";
import type { SubtitleEditorDialog } from "./subtitle-editor-dialog";

vi.mock("@/components/ui/scroll-area", () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
// Exercise the real preview/editor state coupling. The modal's persistence,
// focus restoration and router guards are covered by its dedicated tests.
vi.mock("@/components/app/subtitle-editor-dialog", () => ({
  SubtitleEditorDialog: (props: ComponentProps<typeof SubtitleEditorDialog>) => (
    <div data-testid="editor-dialog" data-initial-cue-id={props.initialCueId} data-kind={props.kind}>
      <button onClick={props.onClose}>Close editor</button>
    </div>
  ),
}));

const t: TFunction = (key, values) => values ? `${key}:${Object.values(values).join(",")}` : key;
const source = Array.from({ length: 125 }, (_, index) => ({
  id: index + 1, start_ms: index * 1000, end_ms: (index + 1) * 1000, text: `Source ${index + 1}`,
}));
const preview: SubtitlePreview = {
  source_srt: "source snapshot", source_file_name: "source.srt", source_segments: source,
  translated_srt: "translated snapshot", translated_file_name: "translated.srt",
  translated_segments: source.filter(({ id }) => id !== 2).map((segment) => ({ ...segment, text: `Translation ${segment.id}` })),
};
const task: TaskRecord = {
  id: "task-1", source_type: "video", file_name: "Interview.mp4", video_path: "/media/Interview.mp4",
  status: "completed", stage: "completed", progress: 1, message: "Ready", created_at: 1, updated_at: 1,
  settings: defaultSettings, source_srt_path: "/task/source.srt", translated_srt_path: "/task/translated.srt",
};
type View = "source" | "translated" | "parallel";
function Harness({ subtitlePreview = preview, initialView = "parallel" }: { subtitlePreview?: SubtitlePreview; initialView?: View }) {
  const [subtitleView, setSubtitleView] = useState<View>(initialView);
  return <SubtitlePreviewCard task={task} hasTranslatedSubtitle subtitlePreview={subtitlePreview} subtitleView={subtitleView}
    setSubtitleView={setSubtitleView} t={t} onRefreshPreview={() => {}} onSourceSaved={async () => {}} />;
}
let renderer: ReactTestRenderer | undefined;
function mount(initialView: View = "parallel") {
  act(() => { renderer = create(<Harness initialView={initialView} />, {
    createNodeMock: (element) => element.type === "p" ? { focus: vi.fn(), scrollIntoView: vi.fn() } : null,
  }); });
}
function root() { return renderer!.root; }
function button(label: string) { return root().findAllByType("button").find((node) => node.children.includes(label))!; }
function rows() { return root().findAllByType("div").filter((node) => node.props["data-cue-id"] !== undefined); }
function jump(id: number) {
  act(() => root().findByProps({ inputMode: "numeric" }).props.onChange({ target: { value: String(id) } }));
  act(() => button("subtitle.jump").props.onClick());
}
async function openEditor(kind: "source" | "translated") {
  await act(async () => { button(kind === "source" ? "subtitle.editSource" : "subtitle.editTranslated").props.onClick(); });
  return root().findByProps({ "data-testid": "editor-dialog" });
}
function changeView(view: View) { act(() => root().findByType(Tabs).props.onValueChange(view)); }
afterEach(() => { act(() => renderer?.unmount()); renderer = undefined; });

describe("subtitle preview to edit behavior", () => {
  it.each(["source", "translated"] as const)("passes jumped actual ID101 into Edit %s with a missing-middle translation", async (kind) => {
    mount();
    expect(root().findByType(SubtitleSourceEditor).props.segments).toBe(source);
    jump(101);
    expect(rows()[0].props["data-cue-id"]).toBe(101);
    expect(rows()[0].props["data-active"]).toBe(true);
    const editor = await openEditor(kind);
    expect(editor.props["data-kind"]).toBe(kind);
    expect(editor.props["data-initial-cue-id"]).toBe(101);
  });

  it.each(["source", "translated"] as const)("passes manually selected ID101 into Edit %s", async (kind) => {
    mount("translated");
    act(() => button("subtitle.nextPage").props.onClick());
    const row = rows().find((node) => node.props["data-cue-id"] === 101)!;
    expect(row).toBeTruthy();
    act(() => row.props.onClick());
    expect((await openEditor(kind)).props["data-initial-cue-id"]).toBe(101);
  });

  it.each(["source", "translated"] as const)("clears the parent cue before Edit %s after searching", async (kind) => {
    mount(); jump(101);
    act(() => root().findByProps({ type: "search" }).props.onChange({ target: { value: "Source 101" } }));
    expect(rows()).toHaveLength(1);
    expect(rows()[0].props["data-active"]).toBeUndefined();
    expect((await openEditor(kind)).props["data-initial-cue-id"]).toBeNull();
  });

  it.each(["source", "translated"] as const)("clears the parent cue before Edit %s after changing pages", async (kind) => {
    mount(); jump(101);
    act(() => button("subtitle.previousPage").props.onClick());
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
    expect((await openEditor(kind)).props["data-initial-cue-id"]).toBeNull();
  });

  it.each(["source", "translated", "parallel"] as const)("clears parent and local selection when switching to %s view", async (view) => {
    mount(view === "parallel" ? "translated" : "parallel"); jump(101);
    changeView(view);
    expect(rows()[0].props["data-cue-id"]).toBe(1);
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
    expect((await openEditor("source")).props["data-initial-cue-id"]).toBeNull();
  });

  it("reconciles a removed selected cue after preview refresh before either editor opens", async () => {
    mount(); jump(101);
    const refreshed: SubtitlePreview = {
      ...preview,
      source_segments: source.filter(({ id }) => id !== 101),
      translated_segments: preview.translated_segments!.filter(({ id }) => id !== 101),
    };
    act(() => renderer!.update(<Harness subtitlePreview={refreshed} />));
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
    expect((await openEditor("source")).props["data-initial-cue-id"]).toBeNull();
    act(() => button("Close editor").props.onClick());
    expect((await openEditor("translated")).props["data-initial-cue-id"]).toBeNull();
  });

  it("preserves an unchanged selected cue across a content-only preview refresh", async () => {
    mount(); jump(101);
    const refreshed = { ...preview, source_segments: source.map((cue) => cue.id === 101 ? { ...cue, text: "Refreshed 101" } : cue) };
    act(() => renderer!.update(<Harness subtitlePreview={refreshed} />));
    expect(rows()[0].props["data-active"]).toBe(true);
    expect((await openEditor("translated")).props["data-initial-cue-id"]).toBe(101);
  });
});
