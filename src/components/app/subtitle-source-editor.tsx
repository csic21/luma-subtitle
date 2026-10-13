import { memo, useCallback, useDeferredValue, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { AlertCircle, ChevronLeft, ChevronRight, Search } from "lucide-react";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Field, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
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
  referencePosition?: "before" | "after";
  contentLabel?: string;
  toolbarStart?: ReactNode;
  toolbarEnd?: ReactNode;
  initialCueId?: number | null;
  onActiveCueChange?: (id: number | null) => void;
  t: TFunction;
};

type SubtitleSourceRowProps = {
  segment: SourceSubtitleSegment;
  text: string;
  onTextChange: (id: number, text: string) => void;
  onActivate: (id: number) => void;
  disabled: boolean;
  readOnly?: boolean;
  hasReference: boolean;
  reference?: SourceSubtitleSegment;
  referencePosition: "before" | "after";
  contentLabel: string;
  duplicateCue: boolean;
  duplicateReference: boolean;
  active: boolean;
  focusRequest: number;
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

function groupById(segments: SourceSubtitleSegment[]) {
  const groups = new Map<number, SourceSubtitleSegment[]>();
  for (const segment of segments) {
    const group = groups.get(segment.id);
    if (group) group.push(segment);
    else groups.set(segment.id, [segment]);
  }
  return groups;
}

const SubtitleSourceRow = memo(function SubtitleSourceRow({
  segment, text, onTextChange, onActivate, disabled, readOnly, hasReference,
  reference, referencePosition, contentLabel, duplicateCue, duplicateReference, active, focusRequest, t,
}: SubtitleSourceRowProps) {
  const { id, start_ms: startMs, end_ms: endMs } = segment;
  const fieldId = useId();
  const timingId = `${fieldId}-timing`;
  const errorId = `${fieldId}-error`;
  const warningId = `${fieldId}-warning`;
  const empty = text.trim().length === 0;
  const ambiguous = duplicateCue || duplicateReference;
  const mismatch = !!reference && (reference.start_ms !== startMs || reference.end_ms !== endMs);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const textRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (!focusRequest) return;
    const element = readOnly ? textRef.current : textareaRef.current;
    element?.scrollIntoView({ block: "center" });
    element?.focus({ preventScroll: true });
  }, [focusRequest, readOnly]);

  const referenceCell = hasReference ? (
    <div className="subtitle-cue-reference" data-missing={!reference || undefined}>
      <p className="whitespace-pre-wrap">
        {ambiguous ? t("subtitle.ambiguousReference") : reference?.text ?? t("subtitle.missingReference")}
      </p>
      {mismatch ? (
        <p className="subtitle-reference-timing" id={warningId}>
          <span>{t("subtitle.timingMismatch")}</span>
          <span>{t("subtitle.referenceTiming")}</span>
          <code>{formatSrtTimestamp(reference.start_ms)} → {formatSrtTimestamp(reference.end_ms)}</code>
        </p>
      ) : null}
    </div>
  ) : null;
  const contentCell = (
    <div className="subtitle-cue-text">
      <FieldLabel htmlFor={fieldId} className="sr-only">{contentLabel} · {t("subtitle.entry", { id })}</FieldLabel>
      {readOnly ? (
        <p
          ref={textRef}
          id={fieldId}
          className="subtitle-cue-content whitespace-pre-wrap"
          tabIndex={disabled || ambiguous ? -1 : 0}
          aria-describedby={timingId}
          onFocus={() => { if (!disabled && !ambiguous) onActivate(id); }}
        >{text}</p>
      ) : (
        <Textarea
          ref={textareaRef}
          id={fieldId}
          value={text}
          disabled={disabled || ambiguous}
          aria-invalid={empty || undefined}
          aria-describedby={[timingId, empty ? errorId : "", mismatch ? warningId : ""].filter(Boolean).join(" ")}
          onFocus={() => onActivate(id)}
          onChange={(event) => { if (!disabled && !ambiguous) onTextChange(id, event.target.value); }}
          rows={2}
          className="min-h-16 max-h-64 resize-y"
        />
      )}
      {ambiguous ? <p className="subtitle-cue-warning">{t("subtitle.ambiguousCue", { id })}</p> : null}
      {!readOnly && empty ? <FieldError id={errorId}>{t("subtitle.emptyText")}</FieldError> : null}
    </div>
  );

  return (
    <Field
      className="subtitle-cue-row"
      data-cue-id={id}
      data-active={active || undefined}
      data-disabled={disabled || ambiguous || undefined}
      data-invalid={!readOnly && empty || undefined}
      onClick={() => { if (!disabled && !ambiguous) onActivate(id); }}
    >
      <div className="subtitle-cue-time" id={timingId}>
        <span aria-label={t("subtitle.entry", { id })}>{String(id).padStart(2, "0")}</span>
        <code>{formatSrtTimestamp(startMs)}</code>
        <code>{formatSrtTimestamp(endMs)}</code>
      </div>
      {referencePosition === "before" ? referenceCell : null}
      {contentCell}
      {referencePosition === "after" ? referenceCell : null}
    </Field>
  );
});

export function SubtitleSourceEditor({
  segments, edits, onTextChange, disabled, readOnly, referenceSegments, referenceLabel,
  referencePosition = "before", contentLabel, toolbarStart, toolbarEnd, initialCueId = null, onActiveCueChange, t,
}: SubtitleSourceEditorProps) {
  const segmentGroups = useMemo(() => groupById(segments), [segments]);
  const referenceGroups = useMemo(() => groupById(referenceSegments ?? []), [referenceSegments]);
  const entries = useMemo(() => {
    const occurrences = new Map<number, number>();
    return segments.map((segment) => {
      const occurrence = occurrences.get(segment.id) ?? 0;
      occurrences.set(segment.id, occurrence + 1);
      return { segment, key: `${segment.id}:${occurrence}` };
    });
  }, [segments]);
  const isUnambiguous = useCallback((id: number) => segmentGroups.get(id)?.length === 1 && (referenceGroups.get(id)?.length ?? 0) <= 1, [segmentGroups, referenceGroups]);
  const initialIndex = initialCueId !== null && isUnambiguous(initialCueId) ? segments.findIndex(({ id }) => id === initialCueId) : -1;
  const [page, setPage] = useState(() => initialIndex < 0 ? 0 : Math.floor(initialIndex / SEGMENTS_PER_PAGE));
  const [query, setQuery] = useState("");
  const [jump, setJump] = useState("");
  const [jumpError, setJumpError] = useState(false);
  const [activeId, setActiveId] = useState<number | null>(() => initialIndex < 0 ? null : initialCueId);
  const [focusTarget, setFocusTarget] = useState(() => ({ id: initialIndex < 0 ? null : initialCueId, request: 1 }));
  const activeChangeRef = useRef(onActiveCueChange);
  activeChangeRef.current = onActiveCueChange;
  const searchId = useId();
  const jumpId = useId();
  const deferredQuery = useDeferredValue(query);
  const searching = query.trim().length > 0;
  const filteredEntries = useMemo(() => {
    // Clearing a search (including a jump) must restore the full list before
    // page clamping or focus reconciliation can discard the destination.
    const needle = searching ? deferredQuery.trim().toLocaleLowerCase() : "";
    return needle ? entries.filter(({ segment }) => `${segment.id}\n${segment.text}\n${edits[segment.id] ?? ""}\n${referenceGroups.get(segment.id)?.map(({ text }) => text).join("\n") ?? ""}`.toLocaleLowerCase().includes(needle)) : entries;
  }, [searching, deferredQuery, entries, edits, referenceGroups]);
  const pageCount = Math.max(1, Math.ceil(filteredEntries.length / SEGMENTS_PER_PAGE));
  const currentPage = Math.min(page, pageCount - 1);
  const visibleEntries = filteredEntries.slice(currentPage * SEGMENTS_PER_PAGE, (currentPage + 1) * SEGMENTS_PER_PAGE);
  const validActiveId = activeId !== null && isUnambiguous(activeId) && visibleEntries.some(({ segment }) => segment.id === activeId) ? activeId : null;
  const hasDuplicates = [...segmentGroups.values(), ...referenceGroups.values()].some((group) => group.length > 1);
  const hasReference = referenceSegments !== undefined;
  const resolvedContentLabel = contentLabel ?? t("subtitle.content");
  const jumpAmbiguous = !!jump.trim() && segmentGroups.has(Number(jump)) && !isUnambiguous(Number(jump));

  useEffect(() => { activeChangeRef.current?.(validActiveId); }, [validActiveId]);
  useEffect(() => {
    if (activeId !== null && validActiveId === null) {
      setActiveId(null);
      setFocusTarget((current) => ({ ...current, id: null }));
    }
  }, [activeId, validActiveId]);
  // Reconcile a shrunken list, so growing it later cannot revive a stale page.
  useEffect(() => { if (page !== currentPage) setPage(currentPage); }, [page, currentPage]);

  const activateCue = useCallback((id: number) => setActiveId(id), []);
  function clearSelection() {
    setActiveId(null);
    setFocusTarget((current) => ({ ...current, id: null }));
  }
  function changePage(nextPage: number) {
    setPage(nextPage);
    clearSelection();
  }
  function jumpToCue() {
    if (disabled) return;
    const id = Number(jump);
    const index = segments.findIndex((segment) => segment.id === id);
    if (!jump.trim() || !Number.isInteger(id) || index < 0 || !isUnambiguous(id)) {
      setJumpError(true);
      return;
    }
    setJumpError(false);
    setQuery("");
    setPage(Math.floor(index / SEGMENTS_PER_PAGE));
    setActiveId(id);
    setFocusTarget((current) => ({ id, request: current.request + 1 }));
  }

  return (
    <div className="subtitle-source-editor" data-has-reference={hasReference} data-read-only={readOnly || undefined} data-reference-position={referencePosition}>
      <div className="subtitle-editor-toolbar">
        {toolbarStart}
        <FieldGroup className="subtitle-editor-search">
          <Field>
            <FieldLabel htmlFor={searchId} className="sr-only">{t("subtitle.search")}</FieldLabel>
            <div className="subtitle-search-control">
              <Search aria-hidden="true" />
              <Input id={searchId} type="search" value={query} placeholder={t("subtitle.search")} disabled={disabled} onChange={(event) => { setQuery(event.target.value); setPage(0); clearSelection(); }} />
            </div>
          </Field>
        </FieldGroup>
        {toolbarEnd}
      </div>
      {hasDuplicates ? <Alert className="subtitle-editor-warning"><AlertCircle /><AlertDescription>{t("subtitle.duplicateIds")}</AlertDescription></Alert> : null}
      <div className="subtitle-cue-head" aria-hidden="true">
        <span className="subtitle-cue-time">{t("subtitle.timecode")}</span>
        {hasReference && referencePosition === "before" ? <span className="subtitle-cue-reference">{referenceLabel ?? t("subtitle.reference")}</span> : null}
        <span className="subtitle-cue-text">{resolvedContentLabel}</span>
        {hasReference && referencePosition === "after" ? <span className="subtitle-cue-reference">{referenceLabel ?? t("subtitle.reference")}</span> : null}
      </div>
      <ScrollArea key={currentPage} className="subtitle-cue-list">
        <FieldGroup className="subtitle-cue-rows">
          {visibleEntries.length === 0 ? <p className="subtitle-cue-empty" role="status">{t("subtitle.noMatches")}</p> : null}
          {visibleEntries.map(({ segment, key }) => {
            const duplicateCue = (segmentGroups.get(segment.id)?.length ?? 0) > 1;
            const references = referenceGroups.get(segment.id);
            const duplicateReference = (references?.length ?? 0) > 1;
            return <SubtitleSourceRow
              key={key}
              segment={segment}
              text={duplicateCue ? segment.text : edits[segment.id] ?? segment.text}
              onTextChange={onTextChange}
              onActivate={activateCue}
              disabled={disabled}
              readOnly={readOnly}
              hasReference={hasReference}
              reference={!duplicateCue && !duplicateReference ? references?.[0] : undefined}
              referencePosition={referencePosition}
              contentLabel={resolvedContentLabel}
              duplicateCue={duplicateCue}
              duplicateReference={duplicateReference}
              active={validActiveId === segment.id}
              focusRequest={focusTarget.id === segment.id && isUnambiguous(segment.id) ? focusTarget.request : 0}
              t={t}
            />;
          })}
        </FieldGroup>
      </ScrollArea>
      <div className="subtitle-editor-footer">
        <FieldGroup className="subtitle-editor-jump">
          <Field data-invalid={jumpError || jumpAmbiguous || undefined}>
            <FieldLabel htmlFor={jumpId} className="sr-only">{t("subtitle.jumpLabel")}</FieldLabel>
            <div className="subtitle-jump-control">
              <Input id={jumpId} inputMode="numeric" value={jump} placeholder="#" disabled={disabled} aria-invalid={jumpError || jumpAmbiguous || undefined} aria-describedby={jumpError || jumpAmbiguous ? `${jumpId}-error` : undefined} onChange={(event) => { setJump(event.target.value); setJumpError(false); }} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); jumpToCue(); } }} />
              <Button type="button" variant="ghost" size="sm" disabled={disabled || jumpAmbiguous} onClick={jumpToCue}>{t("subtitle.jump")}</Button>
            </div>
            {jumpError || jumpAmbiguous ? <FieldError id={`${jumpId}-error`}>{t(jumpAmbiguous ? "subtitle.ambiguousCue" : "subtitle.cueNotFound", { id: Number(jump) })}</FieldError> : null}
          </Field>
        </FieldGroup>
        <span className="subtitle-page-status" aria-live="polite">
          {t("subtitle.page", { page: currentPage + 1, pages: pageCount })}
          {query.trim() ? ` · ${t("subtitle.matches", { count: filteredEntries.length })}` : ""}
        </span>
        <div className="subtitle-page-actions">
          <Button type="button" variant="ghost" size="sm" disabled={disabled || currentPage === 0} onClick={() => changePage(Math.max(0, currentPage - 1))}>
            <ChevronLeft data-icon="inline-start" />{t("subtitle.previousPage")}
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={disabled || currentPage >= pageCount - 1} onClick={() => changePage(currentPage + 1)}>
            {t("subtitle.nextPage")}<ChevronRight data-icon="inline-end" />
          </Button>
        </div>
      </div>
    </div>
  );
}
