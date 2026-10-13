import type { ComponentProps } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SubtitleSourceEditor } from "./subtitle-source-editor";
import type { SourceSubtitleSegment, TFunction } from "@/types";

vi.mock("@/components/ui/scroll-area", () => ({ ScrollArea: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div> }));
const t: TFunction = (key, values) => `${key}${values ? ` ${JSON.stringify(values)}` : ""}`;
const segments = Array.from({ length: 101 }, (_, index) => ({ id: index * 2 + 1, start_ms: index * 1000, end_ms: (index + 1) * 1000, text: `Cue ${index + 1}` }));
const consecutive = Array.from({ length: 125 }, (_, index) => ({ id: index + 1, start_ms: index * 1000, end_ms: (index + 1) * 1000, text: `Source ${index + 1}` }));
type Props = ComponentProps<typeof SubtitleSourceEditor>;
let renderer: ReactTestRenderer;
let currentProps: Props;
const focused: Array<string | undefined> = [];
function mount(props: Partial<Props> = {}) {
  currentProps = { segments, referenceSegments: [{ ...segments[100], text: "Unique reference" }], referenceLabel: "Source", edits: {}, onTextChange: () => {}, disabled: false, t, ...props };
  act(() => { renderer = create(<SubtitleSourceEditor {...currentProps} />, { createNodeMock: (element) => ({ focus: () => focused.push(element.props.value ?? element.props.children), scrollIntoView: vi.fn() }) }); });
}
function update(props: Partial<Props>) {
  currentProps = { ...currentProps, ...props };
  act(() => renderer.update(<SubtitleSourceEditor {...currentProps} />));
}
function button(label: string) { return renderer.root.findAllByType("button").find((node) => node.children.includes(label))!; }
function rows() { return renderer.root.findAllByType("div").filter((node) => node.props["data-cue-id"] !== undefined); }
function cell(row: ReactTestInstance, name: string) { return row.findAllByType("div").find((node) => node.props.className === name)!; }
function search(query: string) { act(() => renderer.root.findByProps({ type: "search" }).props.onChange({ target: { value: query } })); }
function jump(id: number) {
  act(() => renderer.root.findByProps({ inputMode: "numeric" }).props.onChange({ target: { value: String(id) } }));
  act(() => button("subtitle.jump").props.onClick());
}
function page() { return renderer.root.findByProps({ className: "subtitle-page-status" }).children.join(""); }
afterEach(() => { act(() => renderer?.unmount()); focused.length = 0; vi.restoreAllMocks(); });

describe("subtitle navigation", () => {
  it("bounds rendered cues and finds reference text across every page", () => {
    mount(); expect(renderer.root.findAllByType("textarea")).toHaveLength(50);
    search("unique reference");
    expect(renderer.root.findAllByType("textarea")).toHaveLength(1);
    expect(renderer.root.findByType("textarea").props.value).toBe("Cue 101");
  });

  it("jumps by actual cue ID rather than array position, clearing the search", () => {
    mount({ edits: { 201: "Saved draft" } });
    search("no match");
    expect(renderer.root.findAllByType("textarea")).toHaveLength(0);
    jump(201);
    expect(renderer.root.findByProps({ type: "search" }).props.value).toBe("");
    expect(renderer.root.findByType("textarea").props.value).toBe("Saved draft");
    expect(focused[focused.length - 1]).toBe("Saved draft");
  });

  it("reports an unknown cue without changing the current page", () => {
    mount(); jump(2);
    expect(renderer.root.findByProps({ inputMode: "numeric" }).props["aria-invalid"]).toBe(true);
    expect(renderer.root.findAllByType("textarea")).toHaveLength(50);
  });

  it("keeps matching original text visible while editing, and keeps edits on other pages", () => {
    mount({ edits: { 1: "Replacement", 201: "Draft on last page" } });
    search("Cue 1");
    expect(renderer.root.findAllByType("textarea").some((field) => field.props.value === "Replacement")).toBe(true);
    search("Draft on last page");
    expect(renderer.root.findByType("textarea").props.value).toBe("Draft on last page");
  });

  it("opens the real initial ID on source page 3 and focuses its textarea", () => {
    const onActiveCueChange = vi.fn();
    mount({ segments: consecutive, referenceSegments: undefined, initialCueId: 101, onActiveCueChange });
    expect(page()).toContain('"page":3');
    expect(rows()[0].props["data-cue-id"]).toBe(101);
    expect(focused).toEqual(["Source 101"]);
    expect(onActiveCueChange).toHaveBeenLastCalledWith(101);
  });

  it("opens the same ID on target page 2 when a middle cue is missing", () => {
    const target = consecutive.filter(({ id }) => id !== 2).map((segment) => ({ ...segment, text: `Target ${segment.id}` }));
    mount({ segments: target, referenceSegments: consecutive, initialCueId: 101 });
    expect(page()).toContain('"page":2');
    expect(rows()[49].props["data-cue-id"]).toBe(101);
    expect(focused).toEqual(["Target 101"]);
  });

  it("clears both selection and parent active cue on search and page changes", () => {
    const onActiveCueChange = vi.fn();
    mount({ onActiveCueChange });
    act(() => rows()[1].props.onClick());
    expect(onActiveCueChange).toHaveBeenLastCalledWith(3);
    search("Cue");
    expect(onActiveCueChange).toHaveBeenLastCalledWith(null);
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
    act(() => rows()[2].props.onClick());
    expect(onActiveCueChange).toHaveBeenLastCalledWith(5);
    act(() => button("subtitle.nextPage").props.onClick());
    expect(onActiveCueChange).toHaveBeenLastCalledWith(null);
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
  });

  it("reconciles a selected ID removed by a refreshed preview without reviving it", () => {
    const onActiveCueChange = vi.fn();
    mount({ segments: consecutive, referenceSegments: undefined, initialCueId: 101, onActiveCueChange });
    update({ segments: consecutive.filter(({ id }) => id !== 101) });
    expect(onActiveCueChange).toHaveBeenLastCalledWith(null);
    update({ segments: consecutive });
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
    expect(focused).toEqual(["Source 101"]);
  });

  it("does not reset selection or focus when surrounding layout props change", () => {
    mount({ initialCueId: 101 });
    const previousFocusCount = focused.length;
    update({ toolbarEnd: <button>Resized controls</button> });
    expect(rows().some((row) => row.props["data-cue-id"] === 101 && row.props["data-active"])).toBe(true);
    expect(focused).toHaveLength(previousFocusCount);
  });
});

describe("subtitle cue alignment", () => {
  const source = consecutive.slice(0, 3);
  const translation = (id: number): SourceSubtitleSegment => ({ ...source[id - 1], text: `Translation ${id}` });

  it("pairs reordered references by ID and retains the missing-middle column", () => {
    mount({ segments: source, referenceSegments: [translation(3), translation(1)], referencePosition: "after", readOnly: true, contentLabel: "Source", referenceLabel: "Translated" });
    expect(rows()).toHaveLength(3);
    const referenceTexts = rows().map((row) => cell(row, "subtitle-cue-reference").findByType("p").children.join(""));
    expect(referenceTexts).toEqual(["Translation 1", "subtitle.missingReference", "Translation 3"]);
    expect(rows().map((row) => cell(row, "subtitle-cue-text").findByType("p").children.join(""))).toEqual(["Source 1", "Source 2", "Source 3"]);
    expect(rows().map((row) => row.children.map((child) => typeof child === "object" ? child.props.className : null).filter(Boolean))).toEqual(Array(3).fill(["subtitle-cue-time", "subtitle-cue-text", "subtitle-cue-reference"]));
    expect(renderer.root.findAllByType("textarea")).toHaveLength(0);
  });

  it("retains three columns for an explicitly empty reference list", () => {
    mount({ segments: source, referenceSegments: [], readOnly: true });
    expect(rows().every((row) => cell(row, "subtitle-cue-reference").props["data-missing"])).toBe(true);
    expect(renderer.root.findByProps({ className: "subtitle-cue-head" }).children).toHaveLength(3);
  });

  it("shows mismatched reference timing explicitly without changing canonical timing", () => {
    mount({ segments: [source[0]], referenceSegments: [{ ...translation(1), start_ms: 4500, end_ms: 5900 }], readOnly: true });
    const times = cell(rows()[0], "subtitle-cue-time").findAllByType("code").map((node) => node.children.join(""));
    expect(times).toEqual(["00:00:00,000", "00:00:01,000"]);
    const reference = cell(rows()[0], "subtitle-cue-reference");
    expect(reference.findByType("code").children.join("")).toBe("00:00:04,500 → 00:00:05,900");
    expect(reference.findAllByType("span").some((node) => node.children.includes("subtitle.timingMismatch"))).toBe(true);
  });

  it("uses selectable plain paragraphs in read-only mode and updates the parent", () => {
    const onActiveCueChange = vi.fn();
    mount({ segments: source, referenceSegments: undefined, readOnly: true, onActiveCueChange });
    const paragraph = cell(rows()[1], "subtitle-cue-text").findByType("p");
    expect(paragraph.props.tabIndex).toBe(0);
    act(() => paragraph.props.onFocus());
    expect(onActiveCueChange).toHaveBeenLastCalledWith(2);
    expect(rows()[1].props["data-active"]).toBe(true);
  });

  it("preserves duplicate source texts, uses unique React keys, and prevents ambiguous edits and jump", () => {
    const warn = vi.spyOn(console, "error").mockImplementation(() => {});
    const onTextChange = vi.fn();
    mount({ segments: [source[0], { ...source[0], text: "Second duplicate" }], referenceSegments: [translation(1)], edits: { 1: "Must not overwrite either" }, onTextChange, initialCueId: 1 });
    const fields = renderer.root.findAllByType("textarea");
    expect(fields.map((field) => field.props.value)).toEqual(["Source 1", "Second duplicate"]);
    expect(fields.every((field) => field.props.disabled)).toBe(true);
    act(() => fields[0].props.onChange({ target: { value: "unsafe" } }));
    expect(onTextChange).not.toHaveBeenCalled();
    expect(rows().every((row) => cell(row, "subtitle-cue-reference").findByType("p").children.includes("subtitle.ambiguousReference"))).toBe(true);
    jump(1);
    expect(button("subtitle.jump").props.disabled).toBe(true);
    expect(focused).toHaveLength(0);
    expect(warn).not.toHaveBeenCalled();
  });

  it("does not pick the last duplicate reference and still searches both candidate texts", () => {
    mount({ segments: source, referenceSegments: [translation(1), { ...translation(1), text: "Second candidate" }] });
    expect(rows()[0].findByType("textarea").props.disabled).toBe(true);
    expect(cell(rows()[0], "subtitle-cue-reference").findByType("p").children).toEqual(["subtitle.ambiguousReference"]);
    search("Second candidate");
    expect(rows()).toHaveLength(1);
    expect(rows()[0].props["data-cue-id"]).toBe(1);
    jump(1);
    expect(button("subtitle.jump").props.disabled).toBe(true);
  });

  it("clears a selection when its ID becomes ambiguous in a refreshed preview", () => {
    const onActiveCueChange = vi.fn();
    mount({ segments: source, referenceSegments: undefined, initialCueId: 1, onActiveCueChange });
    update({ segments: [source[0], source[0], source[1]] });
    expect(onActiveCueChange).toHaveBeenLastCalledWith(null);
    expect(rows().every((row) => !row.props["data-active"])).toBe(true);
  });
});
