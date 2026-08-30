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
import { saveSourceSubtitles } from "@/lib/tauri-api";
import type { SubtitlePreview, TaskRecord, TFunction } from "@/types";

export function SubtitleEditorDialog({
  taskId, preview, busy, t, onClose, onSaved, onReturnFocus,
}: {
  taskId: string;
  preview: SubtitlePreview;
  busy: boolean;
  t: TFunction;
  onClose: () => void;
  onSaved: (task: TaskRecord) => Promise<void>;
  onReturnFocus: () => void;
}) {
  // The preview is captured when editing starts. Incoming task updates must not
  // overwrite a draft; the backend checks this snapshot before saving it.
  const [edits, setEdits] = useState<Record<number, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [discardOpen, setDiscardOpen] = useState(false);
  const savingRef = useRef(false);
  const originals = useMemo(
    () => new Map(preview.source_segments.map((segment) => [segment.id, segment.text])),
    [preview],
  );
  const editedCount = Object.keys(edits).length;
  const invalid = Object.values(edits).some((text) => !text.trim());
  const blocker = useBlocker(editedCount > 0 || saving);

  useEffect(() => {
    if (blocker.state !== "blocked") return;
    if (savingRef.current) blocker.reset();
    else setDiscardOpen(true);
  }, [blocker]);

  const changeText = useCallback((id: number, text: string) => {
    setEdits((current) => {
      const next = { ...current };
      if (text === originals.get(id)) delete next[id];
      else next[id] = text;
      return next;
    });
    setError("");
  }, [originals]);

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

  async function save() {
    if (savingRef.current || busy || invalid || !editedCount) return;
    savingRef.current = true;
    setSaving(true);
    setError("");
    try {
      const updated = await saveSourceSubtitles(
        taskId,
        preview.source_srt,
        preview.source_segments.map(({ id, text }) => ({ id, text: edits[id] ?? text })),
      );
      await onSaved(updated);
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
          className="max-h-[94dvh] sm:max-w-3xl"
          showCloseButton={false}
          onCloseAutoFocus={(event) => { event.preventDefault(); onReturnFocus(); }}
        >
          <DialogHeader>
            <DialogTitle>{t("subtitle.editSource")}</DialogTitle>
            <DialogDescription>{t("subtitle.editDescription")}</DialogDescription>
          </DialogHeader>
          {(error || busy) && (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertDescription>{error || t("subtitle.editBusy")}</AlertDescription>
            </Alert>
          )}
          <SubtitleSourceEditor
            segments={preview.source_segments}
            edits={edits}
            onTextChange={changeText}
            disabled={saving || busy}
            t={t}
          />
          <DialogFooter>
            <span className="mr-auto self-center text-sm text-muted-foreground" role="status">
              {invalid ? t("subtitle.emptyText") : t("subtitle.editedCount", { count: editedCount })}
            </span>
            <Button variant="outline" onClick={requestClose} disabled={saving}>{t("common.cancel")}</Button>
            <Button onClick={() => void save()} disabled={saving || busy || invalid || !editedCount}>
              {saving ? <Loader2 data-icon="inline-start" className="animate-spin" /> : <Save data-icon="inline-start" />}
              {t(saving ? "subtitle.saving" : "subtitle.saveSource")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={discardOpen} onOpenChange={(open) => {
        setDiscardOpen(open);
        if (!open && blocker.state === "blocked") blocker.reset();
      }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("subtitle.discardTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("subtitle.discardDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("subtitle.keepEditing")}</AlertDialogCancel>
            <AlertDialogAction onClick={(event) => {
              event.preventDefault();
              if (blocker.state === "blocked") blocker.proceed();
              onClose();
            }}>{t("subtitle.discard")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
