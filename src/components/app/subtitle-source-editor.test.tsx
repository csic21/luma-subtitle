import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SubtitleSourceEditor } from "./subtitle-source-editor";
import type { TFunction } from "@/types";

vi.mock("@/components/ui/scroll-area", () => ({ ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
const t: TFunction = (key, values) => `${key}${values ? ` ${JSON.stringify(values)}` : ""}`;
const segments = Array.from({ length: 101 }, (_, index) => ({ id: index * 2 + 1, start_ms: index * 1000, end_ms: (index + 1) * 1000, text: `Cue ${index + 1}` }));
let renderer: ReactTestRenderer;
function mount(edits: Record<number, string> = {}) { act(() => { renderer = create(<SubtitleSourceEditor segments={segments} referenceSegments={[{ ...segments[100], text: "Unique reference" }]} referenceLabel="Source" edits={edits} onTextChange={() => {}} disabled={false} t={t} />); }); }
afterEach(() => act(() => renderer?.unmount()));

describe("subtitle navigation", () => {
  it("bounds rendered cues and finds reference text across every page", () => {
    mount(); expect(renderer.root.findAllByType("textarea")).toHaveLength(50);
    act(() => renderer.root.findByProps({ type: "search" }).props.onChange({ target: { value: "unique reference" } }));
    expect(renderer.root.findAllByType("textarea")).toHaveLength(1);
    expect(renderer.root.findByType("textarea").props.value).toBe("Cue 101");
  });

  it("jumps by actual cue ID rather than array position, clearing the search", () => {
    mount({ 201: "Saved draft" });
    act(() => renderer.root.findByProps({ type: "search" }).props.onChange({ target: { value: "no match" } }));
    expect(renderer.root.findAllByType("textarea")).toHaveLength(0);
    act(() => renderer.root.findByProps({ inputMode: "numeric" }).props.onChange({ target: { value: "201" } }));
    const jump = renderer.root.findAllByType("button").find((button) => button.children.includes("subtitle.jump"))!;
    act(() => jump.props.onClick());
    expect(renderer.root.findByProps({ type: "search" }).props.value).toBe("");
    expect(renderer.root.findByType("textarea").props.value).toBe("Saved draft");
  });

  it("reports an unknown cue without changing the current page", () => {
    mount();
    act(() => renderer.root.findByProps({ inputMode: "numeric" }).props.onChange({ target: { value: "2" } }));
    act(() => renderer.root.findAllByType("button").find((button) => button.children.includes("subtitle.jump"))!.props.onClick());
    expect(renderer.root.findByProps({ inputMode: "numeric" }).props["aria-invalid"]).toBe(true);
    expect(renderer.root.findAllByType("textarea")).toHaveLength(50);
  });

  it("keeps matching original text visible while editing, and keeps edits on other pages", () => {
    mount({ 1: "Replacement", 201: "Draft on last page" });
    act(() => renderer.root.findByProps({ type: "search" }).props.onChange({ target: { value: "Cue 1" } }));
    expect(renderer.root.findAllByType("textarea").some((field) => field.props.value === "Replacement")).toBe(true);
    act(() => renderer.root.findByProps({ type: "search" }).props.onChange({ target: { value: "Draft on last page" } }));
    expect(renderer.root.findByType("textarea").props.value).toBe("Draft on last page");
  });
});
