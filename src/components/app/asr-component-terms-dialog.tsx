import { useEffect, useId, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { componentDownloadBytes, componentSources, runtimeAcknowledgement, type AsrInstallConsent, type AsrRuntimeComponent } from "@/lib/asr-components";
import type { TFunction } from "@/types";

// Mounted for one immutable runtime plan. Closing always discards assent; no
// preferences, settings, or acceptance receipts are written by this dialog.
export function AsrComponentTermsDialog({ runtime, disabled, onCancel, onAccept, t }: {
  runtime: AsrRuntimeComponent;
  disabled: boolean;
  onCancel: () => void;
  onAccept: (consent: AsrInstallConsent) => void;
  t: TFunction;
}) {
  const id = useId();
  const [accepted, setAccepted] = useState(false);
  const live = useRef(true);
  const submitted = useRef(false);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const consent = runtimeAcknowledgement(runtime);
  const close = () => {
    live.current = false;
    setAccepted(false);
    onCancel();
  };
  return <Dialog open onOpenChange={(open) => { if (!open) close(); }}>
    <DialogContent className="flex max-h-[90vh] flex-col sm:max-w-2xl" showCloseButton={false}>
      <DialogHeader className="shrink-0">
        <DialogTitle>{t("asr.terms.title")}</DialogTitle>
        <DialogDescription>{t("asr.terms.intro")}</DialogDescription>
      </DialogHeader>
      <div className="flex min-h-0 max-h-[55vh] flex-col gap-4 overflow-y-auto break-words">
        <FieldGroup>
          <FieldDescription>{runtime.label}{" · "}{runtime.id}{" · "}{runtime.version}</FieldDescription>
          <FieldDescription>{t("asr.terms.bytes", { download: componentDownloadBytes(runtime).toLocaleString(), installed: runtime.installed_bytes.toLocaleString() })}</FieldDescription>
          <FieldDescription className="break-all">{t("asr.terms.planHash")} {runtime.plan_sha256}</FieldDescription>
          <FieldDescription>{t("asr.terms.sources")} {[...new Set(componentSources(runtime).map((url) => new URL(url).hostname))].join(", ")}</FieldDescription>
          <details>
            <summary className="cursor-pointer">{t("asr.managed.sources")}</summary>
            {componentSources(runtime).map((url) => <FieldDescription key={url} className="break-all"><a href={url} target="_blank" rel="noreferrer">{url}</a></FieldDescription>)}
          </details>
          <FieldDescription>{t("asr.terms.scope")}</FieldDescription>
          {runtime.recipe?.terms.map((term) => <Field key={`${term.id}-${term.version}-${term.sha256}`}>
            <FieldLabel>{term.id}{" · "}{term.version}</FieldLabel>
            <FieldDescription className="break-all">{t("asr.terms.textHash")} {term.sha256}</FieldDescription>
            {term.raw_sha256 && <FieldDescription className="break-all">{t("asr.terms.sourceHash")} {term.raw_sha256}{term.source_encoding ? ` · ${term.source_encoding}` : ""}</FieldDescription>}
            <FieldDescription className="break-all"><a href={term.url} target="_blank" rel="noreferrer">{term.url}</a></FieldDescription>
            <pre className="whitespace-pre-wrap break-words rounded-md border p-3 text-sm">{term.text}</pre>
          </Field>)}
          {!consent && <FieldDescription role="alert">{t("asr.terms.invalid")}</FieldDescription>}
        </FieldGroup>
      </div>
      <Field orientation="horizontal" className="shrink-0" data-disabled={disabled}>
        <Checkbox id={`${id}-assent`} checked={accepted} disabled={disabled || !consent} onCheckedChange={(value) => setAccepted(value === true)} />
        <FieldLabel htmlFor={`${id}-assent`}>{t("asr.terms.acceptLabel")}</FieldLabel>
      </Field>
      <DialogFooter className="shrink-0">
        <Button type="button" variant="outline" onClick={close}>{t("asr.terms.decline")}</Button>
        <Button type="button" disabled={disabled || !accepted || !consent} onClick={() => {
          if (!live.current || submitted.current || disabled || !accepted || !consent) return;
          submitted.current = true;
          onAccept(consent);
        }}>{t("asr.terms.acceptInstall")}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
