import { memo, useId, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
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
  t: TFunction;
};

type SubtitleSourceRowProps = {
  id: number;
  startMs: number;
  endMs: number;
  text: string;
  onTextChange: (id: number, text: string) => void;
  disabled: boolean;
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
  t,
}: SubtitleSourceRowProps) {
  const fieldId = useId();
  const timingId = `${fieldId}-timing`;
  const errorId = `${fieldId}-error`;
  const empty = text.trim().length === 0;

  return (
    <Field data-disabled={disabled || undefined} data-invalid={empty || undefined}>
      <FieldLabel htmlFor={fieldId}>{t("subtitle.entry", { id })}</FieldLabel>
      <FieldDescription id={timingId}>
        <code>
          {formatSrtTimestamp(startMs)} --&gt; {formatSrtTimestamp(endMs)}
        </code>
      </FieldDescription>
      <Textarea
        id={fieldId}
        value={text}
        disabled={disabled}
        aria-invalid={empty || undefined}
        aria-describedby={empty ? `${timingId} ${errorId}` : timingId}
        onChange={(event) => onTextChange(id, event.target.value)}
        rows={2}
        className="min-h-20 max-h-64 resize-y"
      />
      {empty ? <FieldError id={errorId}>{t("subtitle.emptyText")}</FieldError> : null}
    </Field>
  );
});

export function SubtitleSourceEditor({
  segments,
  edits,
  onTextChange,
  disabled,
  t,
}: SubtitleSourceEditorProps) {
  const [page, setPage] = useState(0);
  const pageCount = Math.max(1, Math.ceil(segments.length / SEGMENTS_PER_PAGE));
  const currentPage = Math.min(page, pageCount - 1);
  const pageStart = currentPage * SEGMENTS_PER_PAGE;
  const visibleSegments = segments.slice(pageStart, pageStart + SEGMENTS_PER_PAGE);

  return (
    <div className="flex min-h-0 flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm text-muted-foreground" aria-live="polite">
          {t("subtitle.page", { page: currentPage + 1, pages: pageCount })}
        </span>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || currentPage === 0}
            onClick={() => setPage(Math.max(0, currentPage - 1))}
          >
            <ChevronLeft data-icon="inline-start" />
            {t("subtitle.previousPage")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || currentPage >= pageCount - 1}
            onClick={() => setPage(currentPage + 1)}
          >
            {t("subtitle.nextPage")}
            <ChevronRight data-icon="inline-end" />
          </Button>
        </div>
      </div>

      <ScrollArea key={currentPage} className="h-[min(50dvh,36rem)] rounded-md border">
        <FieldGroup className="p-3">
          {visibleSegments.map((segment) => (
            <SubtitleSourceRow
              key={segment.id}
              id={segment.id}
              startMs={segment.start_ms}
              endMs={segment.end_ms}
              text={edits[segment.id] ?? segment.text}
              onTextChange={onTextChange}
              disabled={disabled}
              t={t}
            />
          ))}
        </FieldGroup>
      </ScrollArea>
    </div>
  );
}
