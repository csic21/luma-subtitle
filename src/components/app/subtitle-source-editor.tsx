import { memo, useDeferredValue, useEffect, useId, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import type { SourceSubtitleSegment, TFunction } from "@/types";

const SEGMENTS_PER_PAGE = 50;

type SubtitleSourceEditorProps = {
  segments: SourceSubtitleSegment[];
  edits: Record<number, string>;
  onTextChange: (id: number, text: string) => void;
  disabled: boolean;
  readOnly?: boolean;
  referenceSegments?: SourceSubtitleSegment[];
  referenceLabel?: string;
  t: TFunction;
};

type SubtitleSourceRowProps = {
  id: number;
  startMs: number;
  endMs: number;
  text: string;
  onTextChange: (id: number, text: string) => void;
  disabled: boolean;
  readOnly?: boolean;
  referenceText?: string;
  referenceLabel?: string;
  focused?: boolean;
  focusRequest?: number;
  t: TFunction;
};

function formatSrtTimestamp(totalMilliseconds: number) {
  const milliseconds = Math.max(0, Math.trunc(totalMilliseconds));
  const hours = Math.floor(milliseconds / 3_600_000);
  const minutes = Math.floor((milliseconds % 3_600_000) / 60_000);
  const seconds = Math.floor((milliseconds % 60_000) / 1_000);
  const remainder = milliseconds % 1_000;

  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")},${String(remainder).padStart(3, "0")}`;
}

const SubtitleSourceRow = memo(function SubtitleSourceRow({
  id,
  startMs,
  endMs,
  text,
  onTextChange,
  disabled,
  readOnly,
  referenceText,
  referenceLabel,
  focused,
  focusRequest,
  t,
}: SubtitleSourceRowProps) {
  const fieldId = useId();
  const timingId = `${fieldId}-timing`;
  const errorId = `${fieldId}-error`;
  const empty = text.trim().length === 0;
  const rowRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (focused) {
      rowRef.current?.scrollIntoView({ block: "center" });
      rowRef.current?.focus({ preventScroll: true });
    }
  }, [focused, focusRequest]);

  return (
    <Field data-disabled={disabled || undefined} data-invalid={empty || undefined}>
      <FieldLabel htmlFor={fieldId}>{t("subtitle.entry", { id })}</FieldLabel>
      <FieldDescription id={timingId}>
        <code>
          {formatSrtTimestamp(startMs)} --&gt; {formatSrtTimestamp(endMs)}
        </code>
      </FieldDescription>
      <div className={referenceText !== undefined ? "grid gap-3 sm:grid-cols-2" : undefined}>
      {referenceText !== undefined && <p className="whitespace-pre-wrap text-sm text-muted-foreground"><strong>{referenceLabel}: </strong>{referenceText}</p>}
      <Textarea
        ref={rowRef}
        id={fieldId}
        value={text}
        disabled={disabled}
        readOnly={readOnly}
        aria-invalid={empty || undefined}
        aria-describedby={empty ? `${timingId} ${errorId}` : timingId}
        onChange={(event) => onTextChange(id, event.target.value)}
        rows={2}
        className="min-h-20 max-h-64 resize-y"
      />
      </div>
      {empty ? <FieldError id={errorId}>{t("subtitle.emptyText")}</FieldError> : null}
    </Field>
  );
});

export function SubtitleSourceEditor({
  segments,
  edits,
  onTextChange,
  disabled,
  readOnly,
  referenceSegments,
  referenceLabel,
  t,
}: SubtitleSourceEditorProps) {
  const [page, setPage] = useState(0);
  const [query, setQuery] = useState("");
  const [jump, setJump] = useState("");
  const [jumpError, setJumpError] = useState(false);
  const [focusedId, setFocusedId] = useState<number | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const searchId = useId();
  const jumpId = useId();
  const deferredQuery = useDeferredValue(query);
  const reference = useMemo(() => new Map(referenceSegments?.map((segment) => [segment.id, segment.text])), [referenceSegments]);
  const filteredSegments = useMemo(() => {
    const needle = deferredQuery.trim().toLocaleLowerCase();
    return needle ? segments.filter((segment) => `${segment.id}\n${segment.text}\n${edits[segment.id] ?? ""}\n${reference.get(segment.id) ?? ""}`.toLocaleLowerCase().includes(needle)) : segments;
  }, [deferredQuery, segments, edits, reference]);
  const pageCount = Math.max(1, Math.ceil(filteredSegments.length / SEGMENTS_PER_PAGE));
  const currentPage = Math.min(page, pageCount - 1);
  const pageStart = currentPage * SEGMENTS_PER_PAGE;
  const visibleSegments = filteredSegments.slice(pageStart, pageStart + SEGMENTS_PER_PAGE);
  function jumpToCue() {
    const index = segments.findIndex((segment) => segment.id === Number(jump));
    if (!jump.trim() || index < 0) { setJumpError(true); return; }
    setJumpError(false);
    setQuery("");
    setPage(Math.floor(index / SEGMENTS_PER_PAGE));
    setFocusedId(segments[index].id);
    setFocusRequest((current) => current + 1);
  }

  return (
    <div className="flex min-h-0 flex-col gap-3">
      <FieldGroup className="grid gap-3 sm:grid-cols-2">
        <Field>
          <FieldLabel htmlFor={searchId}>{t("subtitle.search")}</FieldLabel>
          <Input id={searchId} type="search" value={query} disabled={disabled} onChange={(event) => { setQuery(event.target.value); setPage(0); setFocusedId(null); }} />
        </Field>
        <Field data-invalid={jumpError || undefined}>
          <FieldLabel htmlFor={jumpId}>{t("subtitle.jumpLabel")}</FieldLabel>
          <div className="flex gap-2">
            <Input id={jumpId} inputMode="numeric" value={jump} disabled={disabled} aria-invalid={jumpError || undefined} aria-describedby={jumpError ? `${jumpId}-error` : undefined} onChange={(event) => { setJump(event.target.value); setJumpError(false); }} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); jumpToCue(); } }} />
            <Button type="button" variant="outline" disabled={disabled} onClick={jumpToCue}>{t("subtitle.jump")}</Button>
          </div>
          {jumpError && <FieldError id={`${jumpId}-error`}>{t("subtitle.cueNotFound")}</FieldError>}
        </Field>
      </FieldGroup>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm text-muted-foreground" aria-live="polite">
          {t("subtitle.page", { page: currentPage + 1, pages: pageCount })}
          {query.trim() ? ` · ${t("subtitle.matches", { count: filteredSegments.length })}` : ""}
        </span>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || currentPage === 0}
            onClick={() => { setPage(Math.max(0, currentPage - 1)); setFocusedId(null); }}
          >
            <ChevronLeft data-icon="inline-start" />
            {t("subtitle.previousPage")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || currentPage >= pageCount - 1}
            onClick={() => { setPage(currentPage + 1); setFocusedId(null); }}
          >
            {t("subtitle.nextPage")}
            <ChevronRight data-icon="inline-end" />
          </Button>
        </div>
      </div>

      <ScrollArea key={currentPage} className="h-[min(50dvh,36rem)] rounded-md border">
        <FieldGroup className="p-3">
          {visibleSegments.length === 0 && <p role="status">{t("subtitle.noMatches")}</p>}
          {visibleSegments.map((segment) => (
            <SubtitleSourceRow
              key={segment.id}
              id={segment.id}
              startMs={segment.start_ms}
              endMs={segment.end_ms}
              text={edits[segment.id] ?? segment.text}
              onTextChange={onTextChange}
              disabled={disabled}
              readOnly={readOnly}
              referenceText={reference.get(segment.id)}
              referenceLabel={referenceLabel}
              focused={focusedId === segment.id}
              focusRequest={focusRequest}
              t={t}
            />
          ))}
        </FieldGroup>
      </ScrollArea>
    </div>
  );
}
