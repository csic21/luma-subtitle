import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, Loader2, Save } from "lucide-react";
import { useBlocker } from "react-router-dom";

import { SubtitleSourceEditor } from "@/components/app/subtitle-source-editor";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { errorText } from "@/lib/app-utils";
import { saveSourceSubtitles, saveTranslatedSubtitles } from "@/lib/tauri-api";
import type { SubtitlePreview, TaskRecord, TFunction } from "@/types";

export function SubtitleEditorDialog({
  taskId, preview, kind = "source", busy, t, onClose, onSaved, onReturnFocus, initialCueId,
}: {
  taskId: string;
  preview: SubtitlePreview;
  kind?: "source" | "translated";
  busy: boolean;
  t: TFunction;
  onClose: () => void;
  onSaved: (task: TaskRecord, kind: "source" | "translated") => Promise<void>;
  onReturnFocus: () => void;
  initialCueId?: number | null;
}) {
  // The preview is captured when editing starts. Incoming task updates must not
  // overwrite a draft; the backend checks this snapshot before saving it.
  const [snapshot] = useState(preview);
  const [edits, setEdits] = useState<Record<number, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [discardOpen, setDiscardOpen] = useState(false);
  const savingRef = useRef(false);
  const lastEditorFocusRef = useRef<HTMLTextAreaElement | null>(null);
  const discardingRef = useRef(false);
  const segments = kind === "source" ? snapshot.source_segments : snapshot.translated_segments ?? [];
  const referenceSegments = kind === "source" ? snapshot.translated_segments : snapshot.source_segments;
  const originals = useMemo(
    () => new Map(segments.map((segment) => [segment.id, segment.text])),
    [segments],
  );
  const editedCount = Object.keys(edits).length;
  const duplicateIds = originals.size !== segments.length;
  const invalid = duplicateIds || Object.values(edits).some((text) => !text.trim());
  const hasInitialCue = initialCueId != null && segments.filter(({ id }) => id === initialCueId).length === 1
    && (referenceSegments?.filter(({ id }) => id === initialCueId).length ?? 0) <= 1;
  const blocker = useBlocker(editedCount > 0 || saving);

  useEffect(() => {
    if (blocker.state !== "blocked") return;
    if (savingRef.current) blocker.reset();
    else setDiscardOpen(true);
  }, [blocker]);

  const changeText = useCallback((id: number, text: string) => {
    if (segments.filter((segment) => segment.id === id).length !== 1
      || (referenceSegments?.filter((segment) => segment.id === id).length ?? 0) > 1) return;
    setEdits((current) => {
      const next = { ...current };
      if (text === originals.get(id)) delete next[id];
      else next[id] = text;
      return next;
    });
    setError("");
  }, [originals, segments, referenceSegments]);

  useEffect(() => {
    if (!editedCount) return;
    const warnBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warnBeforeUnload);
    return () => window.removeEventListener("beforeunload", warnBeforeUnload);
  }, [editedCount > 0]);

  function requestClose() {
    if (savingRef.current) return;
    if (editedCount) setDiscardOpen(true);
    else onClose();
  }

  function keepEditing() {
    setDiscardOpen(false);
    if (blocker.state === "blocked") blocker.reset();
  }

  async function save() {
    if (savingRef.current || busy || invalid || !editedCount) return;
    savingRef.current = true;
    setSaving(true);
    setError("");
    try {
      const payload = segments.map(({ id, text }) => ({ id, text: edits[id] ?? text }));
      const updated = kind === "source"
        ? await saveSourceSubtitles(taskId, snapshot.source_srt, payload)
        : await saveTranslatedSubtitles(taskId, snapshot.source_srt, snapshot.translated_srt ?? "", payload);
      await onSaved(updated, kind);
      onClose();
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  }

  return (
    <>
      <Dialog open onOpenChange={(open) => { if (!open) requestClose(); }}>
        <DialogContent
          className="subtitle-editor-dialog"
          showCloseButton={false}
          onOpenAutoFocus={(event) => { if (hasInitialCue) event.preventDefault(); }}
          onFocusCapture={(event) => {
            const target = event.target as HTMLElement;
            if (target.tagName === "TEXTAREA") lastEditorFocusRef.current = target as HTMLTextAreaElement;
          }}
          onCloseAutoFocus={(event) => { event.preventDefault(); onReturnFocus(); }}
        >
          <DialogHeader>
            <DialogTitle>{t(kind === "source" ? "subtitle.editSource" : "subtitle.editTranslated")}</DialogTitle>
            <DialogDescription>{t(kind === "source" ? "subtitle.editDescription" : "subtitle.editTranslatedDescription")}</DialogDescription>
          </DialogHeader>
          {(error || busy) && (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertDescription>{error || t("subtitle.editBusy")}</AlertDescription>
            </Alert>
          )}
          <SubtitleSourceEditor
            segments={segments}
            referenceSegments={referenceSegments ?? undefined}
            referenceLabel={t(kind === "source" ? "common.translated" : "common.source")}
            contentLabel={t(kind === "source" ? "common.source" : "common.translated")}
            initialCueId={initialCueId}
            edits={edits}
            onTextChange={changeText}
            disabled={saving || busy}
            t={t}
          />
          <DialogFooter className="subtitle-dialog-footer">
            <span className="mr-auto self-center text-sm text-muted-foreground" role="status">
              {duplicateIds ? t("subtitle.duplicateIds") : invalid ? t("subtitle.emptyText") : t("subtitle.editedCount", { count: editedCount })}
            </span>
            <Button variant="outline" onClick={requestClose} disabled={saving}>{t("common.cancel")}</Button>
            <Button onClick={() => void save()} disabled={saving || busy || invalid || !editedCount}>
              {saving ? <Loader2 data-icon="inline-start" className="animate-spin" /> : <Save data-icon="inline-start" />}
              {t(saving ? "subtitle.saving" : kind === "source" ? "subtitle.saveSource" : "subtitle.saveTranslated")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={discardOpen} onOpenChange={(open) => {
        if (open) setDiscardOpen(true);
        else keepEditing();
      }}>
        <AlertDialogContent onCloseAutoFocus={(event) => {
          if (discardingRef.current) event.preventDefault();
          else if (lastEditorFocusRef.current?.isConnected) {
            event.preventDefault();
            lastEditorFocusRef.current.focus({ preventScroll: true });
          }
        }}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("subtitle.discardTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("subtitle.discardDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={(event) => {
              event.preventDefault();
              keepEditing();
            }}>{t("subtitle.keepEditing")}</AlertDialogCancel>
            <AlertDialogAction onClick={(event) => {
              event.preventDefault();
              if (savingRef.current) return;
              discardingRef.current = true;
              if (blocker.state === "blocked") blocker.proceed();
              onClose();
            }}>{t("subtitle.discard")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
