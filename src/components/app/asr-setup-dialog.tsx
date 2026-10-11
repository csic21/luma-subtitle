import { useEffect, useId, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { componentSources, runtimeAcknowledgement, type AsrInstallConsent } from "@/lib/asr-components";
import type { AsrSetupPlan } from "@/lib/asr-setup-plan";
import { componentStorageLabel } from "@/lib/component-storage";
import type { TFunction } from "@/types";

// This dialog belongs to one frozen plan. Closing it discards all assent.
// Models disclose their licenses; only runtime terms produce consent receipts.
export function AsrSetupDialog({ plan, disabled, onCancel, onAccept, t }: {
  plan: AsrSetupPlan;
  disabled: boolean;
  onCancel: () => void;
  onAccept: (consents: Record<string, AsrInstallConsent>) => void;
  t: TFunction;
}) {
  const id = useId();
  const [accepted, setAccepted] = useState(false);
  const live = useRef(true);
  const returnFocus = useRef(typeof document === "undefined" ? null : document.activeElement);
  const submitted = useRef(false);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const required = plan.steps.filter((step) => step.consentRequired);
  const consents: Record<string, AsrInstallConsent> = {};
  for (const step of required) {
    if ("platform" in step.component) {
      const consent = runtimeAcknowledgement(step.component);
      if (consent) consents[step.component.id] = consent;
    }
  }
  const valid = required.every((step) => !!consents[step.component.id]);
  const close = () => { live.current = false; setAccepted(false); onCancel(); };
  return <Dialog open onOpenChange={(open) => { if (!open) close(); }}>
    <DialogContent className="flex max-h-[90vh] flex-col sm:max-w-xl" showCloseButton={false} onCloseAutoFocus={(event) => {
      if (returnFocus.current instanceof HTMLElement && returnFocus.current.isConnected) { event.preventDefault(); returnFocus.current.focus(); }
    }}>
      <DialogHeader className="shrink-0">
        <DialogTitle>{t("asr.setup.confirmTitle")}</DialogTitle>
        <DialogDescription>{t("asr.setup.confirmDescription", { count: plan.steps.length, size: componentStorageLabel(plan.downloadBytes) })}</DialogDescription>
      </DialogHeader>
      <div className="flex min-h-0 flex-col gap-4 overflow-y-auto break-words">
        <ul className="flex flex-col gap-3" aria-label={t("asr.setup.downloadList")}>
          {plan.steps.map((step) => <li key={step.component.id} className="flex flex-col gap-1">
            <span>{step.component.label}</span>
            <FieldDescription>{componentStorageLabel(step.downloadBytes)}{" · "}<a href={step.component.license_url} target="_blank" rel="noreferrer">{step.component.license}</a></FieldDescription>
          </li>)}
        </ul>
        <FieldDescription>{t("asr.setup.diskHint")}</FieldDescription>
        {plan.steps.some((step) => "platform" in step.component && step.component.recipe?.windows_crt) && <FieldDescription>{t("asr.setup.windowsTrust")}</FieldDescription>}
        {required.map((step) => "platform" in step.component && step.component.recipe?.terms.map((term) => <details key={`${step.component.id}-${term.id}`} className="rounded-md border p-3">
          <summary className="cursor-pointer">{term.id}{" · "}{term.version}</summary>
          <FieldDescription className="mt-2 break-all"><a href={term.url} target="_blank" rel="noreferrer">{term.url}</a></FieldDescription>
          <pre className="mt-2 whitespace-pre-wrap break-words text-sm">{term.text}</pre>
        </details>))}
        <details>
          <summary className="cursor-pointer">{t("asr.setup.technicalDetails")}</summary>
          <div className="mt-3 flex flex-col gap-3">
            {plan.steps.map((step) => <div key={step.component.id} className="flex flex-col gap-1 break-all">
              <span>{step.component.id}{" · "}{step.component.version}</span>
              <FieldDescription>{t("asr.terms.bytes", { download: step.downloadBytes.toLocaleString(), installed: step.component.installed_bytes.toLocaleString() })}</FieldDescription>
              {componentSources(step.component).map((url) => <FieldDescription key={url}><a href={url} target="_blank" rel="noreferrer">{url}</a></FieldDescription>)}
              {"platform" in step.component && <>
                <FieldDescription>{t("asr.terms.planHash")} {step.component.plan_sha256}</FieldDescription>
                {step.component.recipe?.terms.map((term) => <FieldDescription key={term.id}>{term.id}{" · "}{t("asr.terms.textHash")} {term.sha256}{term.raw_sha256 ? ` · ${t("asr.terms.sourceHash")} ${term.raw_sha256}` : ""}</FieldDescription>)}
              </>}
            </div>)}
          </div>
        </details>
        {!valid && <FieldDescription role="alert">{t("asr.terms.invalid")}</FieldDescription>}
      </div>
      {required.length > 0 && <Field orientation="horizontal" className="shrink-0" data-disabled={disabled}>
        <Checkbox id={`${id}-assent`} checked={accepted} disabled={disabled || !valid} onCheckedChange={(value) => setAccepted(value === true)} />
        <FieldLabel htmlFor={`${id}-assent`}>{t("asr.setup.acceptTerms")}</FieldLabel>
      </Field>}
      <DialogFooter className="shrink-0">
        <Button type="button" variant="outline" onClick={close}>{t("asr.setup.back")}</Button>
        <Button type="button" disabled={disabled || !valid || (required.length > 0 && !accepted)} onClick={() => {
          if (!live.current || submitted.current || disabled || !valid || (required.length > 0 && !accepted)) return;
          submitted.current = true;
          onAccept(consents);
        }}>{t("asr.setup.start")}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
