import { useEffect, useId, useMemo, useRef, useState } from "react";
import { AlertCircle, Download, Loader2, RefreshCw, Trash2, X } from "lucide-react";
import { AsrComponentTermsDialog } from "@/components/app/asr-component-terms-dialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Progress } from "@/components/ui/progress";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useAsrComponents } from "@/hooks/use-asr-components";
import { asrConfigsEqual } from "@/lib/asr-config";
import { componentDownloadBytes, componentProgressPercent, componentSources, hasMatchingRuntimeConsent, isTerminalComponentPhase, managedAsrConfig, runtimeAcknowledgement, runtimeConfirmationKey, type AsrComponentAction, type AsrComponentStatus, type AsrInstallConsent, type AsrRecordedConsent, type AsrModelComponent, type AsrRuntimeComponent } from "@/lib/asr-components";
import type { AsrConfig, TFunction } from "@/types";

export function componentStorageLabel(bytes: number) {
  if (bytes < 1_000_000) return `${Math.ceil(bytes / 1_000)} KB`;
  if (bytes < 1_000_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
}

function ComponentDetails({ component, status, consents, disabled, perform, t }: {
  component: AsrRuntimeComponent | AsrModelComponent;
  status?: AsrComponentStatus;
  consents: AsrRecordedConsent[];
  disabled: boolean;
  perform: (action: AsrComponentAction, id: string, consent?: AsrInstallConsent) => Promise<void>;
  t: TFunction;
}) {
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [pendingTerms, setPendingTerms] = useState<{ action: "install" | "repair"; key: string; runtime: AsrRuntimeComponent }>();
  const runtime = "platform" in component ? component : undefined;
  const confirmationKey = useMemo(() => runtime ? runtimeConfirmationKey(runtime) : "", [runtime]);
  const latest = useRef({ mounted: true, key: confirmationKey, disabled });
  latest.current.key = confirmationKey;
  latest.current.disabled = disabled;
  useEffect(() => { latest.current.mounted = true; return () => { latest.current.mounted = false; }; }, []);
  useEffect(() => { setPendingTerms(undefined); }, [confirmationKey]);
  const installed = status?.state === "installed";
  const cachedBytes = status?.cached_bytes ?? 0;
  const cacheOnly = status?.state === "not_installed" && cachedBytes > 0;
  const removable = !!status && (status.state !== "not_installed" || cachedBytes > 0);
  const acknowledgement = runtime?.recipe ? runtimeAcknowledgement(runtime) : null;
  const reuseConsent = !!runtime && hasMatchingRuntimeConsent(runtime, consents);
  const action = installed || status?.state === "damaged" ? "repair" : "install";
  const needsTerms = !!runtime?.recipe?.terms.length && !(action === "repair" && reuseConsent);
  const unavailable = !!component.unavailable_reason || ("files" in component ? component.files.length === 0 : component.recipe ? !acknowledgement : !component.archive);
  const sources = componentSources(component);
  const start = () => {
    if (disabled || unavailable || !latest.current.mounted) return;
    setConfirmRemove(false);
    if (runtime?.recipe) {
      if (!acknowledgement) return;
      if (needsTerms) setPendingTerms({ action, key: confirmationKey, runtime });
      else void perform(action, component.id, acknowledgement);
    } else void perform(action, component.id);
  };
  return (
    <FieldGroup>
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={status?.state === "damaged" ? "destructive" : "secondary"}>
          {t(`asr.managed.state.${status?.state ?? "not_installed"}`)}
        </Badge>
        <span>{component.backend}{" · "}{component.version}</span>
      </div>
      {"platform" in component && <FieldDescription>{component.platform}{component.min_os_version ? ` · ${t("asr.managed.minimumOs", { version: component.min_os_version })}` : ""}</FieldDescription>}
      <FieldDescription>{t(runtime?.recipe ? "asr.terms.bytes" : "asr.managed.bytes", { download: runtime?.recipe ? componentDownloadBytes(component).toLocaleString() : componentStorageLabel(componentDownloadBytes(component)), installed: runtime?.recipe ? component.installed_bytes.toLocaleString() : componentStorageLabel(component.installed_bytes) })}</FieldDescription>
      {installed && status && <FieldDescription>{t("asr.managed.actualStorage", { bytes: status.installed_bytes.toLocaleString() })}</FieldDescription>}
      {cachedBytes > 0 && <FieldDescription>{t("asr.managed.cachedStorage", { bytes: cachedBytes.toLocaleString() })}</FieldDescription>}
      <FieldDescription>{t("asr.managed.license")} <a href={component.license_url} target="_blank" rel="noreferrer">{component.license}</a></FieldDescription>
      <details>
        <summary className="cursor-pointer">{t("asr.managed.sources")}</summary>
        <div className="flex max-h-40 flex-col gap-1 overflow-auto break-all">
          {sources.map((url) => <FieldDescription key={url}><a href={url} target="_blank" rel="noreferrer">{url}</a></FieldDescription>)}
          {sources.length === 0 && <FieldDescription>{t("asr.managed.unpublished")}</FieldDescription>}
        </div>
      </details>
      {unavailable && <FieldDescription role="status">{t("asr.managed.unavailable")} {component.unavailable_reason}</FieldDescription>}
      {runtime?.recipe && action === "repair" && reuseConsent && <FieldDescription>{t("asr.terms.reuse")}</FieldDescription>}
      {status?.error && <FieldDescription role="alert">{status.error}</FieldDescription>}
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="secondary" disabled={disabled || unavailable} onClick={start}>
          {installed || status?.state === "damaged" ? <RefreshCw data-icon="inline-start" /> : <Download data-icon="inline-start" />}
          {t(needsTerms ? action === "repair" ? "asr.terms.reviewRepair" : "asr.terms.reviewInstall" : action === "repair" ? "asr.managed.repair" : "asr.managed.install")}
        </Button>
        {removable && <Button type="button" variant="outline" disabled={disabled} onClick={() => setConfirmRemove(true)}><Trash2 data-icon="inline-start" />{t(cacheOnly ? "asr.managed.clearCache" : "asr.managed.remove")}</Button>}
      </div>
      {pendingTerms?.key === confirmationKey && <AsrComponentTermsDialog key={pendingTerms.key} runtime={pendingTerms.runtime} disabled={disabled} t={t} onCancel={() => setPendingTerms(undefined)} onAccept={(consent) => {
        if (!latest.current.mounted || latest.current.disabled || latest.current.key !== pendingTerms.key) return;
        setPendingTerms(undefined);
        void perform(pendingTerms.action, component.id, consent);
      }} />}
      {confirmRemove && <Alert>
        <AlertCircle /><AlertTitle>{t(cacheOnly ? "asr.managed.clearCacheTitle" : "asr.managed.removeTitle")}</AlertTitle>
        <AlertDescription className="flex flex-col gap-2">
          <p>{t(cacheOnly ? "asr.managed.clearCacheHint" : "asr.managed.removeHint")}</p>
          <div className="flex flex-wrap gap-2">
            <Button type="button" variant="destructive" disabled={disabled} onClick={() => { setConfirmRemove(false); void perform("remove", component.id); }}>{t(cacheOnly ? "asr.managed.confirmClearCache" : "asr.managed.confirmRemove")}</Button>
            <Button type="button" variant="outline" onClick={() => setConfirmRemove(false)}>{t("asr.managed.keep")}</Button>
          </div>
        </AlertDescription>
      </Alert>}
    </FieldGroup>
  );
}

export function ManagedAsrSetup({ config, onChange, disabled, t }: {
  config: AsrConfig;
  onChange: (config: AsrConfig) => void;
  disabled: boolean;
  t: TFunction;
}) {
  const id = useId();
  const manager = useAsrComponents(config.engine, t);
  const [runtimeId, setRuntimeId] = useState("");
  const [modelId, setModelId] = useState("");
  const [alignerId, setAlignerId] = useState("");
  const [applied, setApplied] = useState(false);
  const runtimes = manager.catalog?.runtimes.filter((runtime) => runtime.platform === manager.catalog?.platform && runtime.engine === config.engine) ?? [];
  const runtime = runtimes.find((item) => item.id === runtimeId) ?? runtimes.find((item) => !item.unavailable_reason && (item.archive || item.recipe)) ?? runtimes[0];
  const models = manager.catalog?.models.filter((model) => model.engine === config.engine && model.backend === runtime?.backend && model.role === "model") ?? [];
  const aligners = manager.catalog?.models.filter((model) => model.engine === config.engine && model.backend === runtime?.backend && model.role === "aligner") ?? [];
  const model = models.find((item) => item.id === modelId) ?? models[0];
  const aligner = aligners.find((item) => item.id === alignerId) ?? aligners[0];
  const selection = manager.verified ? managedAsrConfig(config, runtime, model, aligner, manager.components) : null;
  const inUse = !!selection && asrConfigsEqual(selection, config);
  const locked = disabled || manager.loading || manager.busy;
  const groups = [
    { key: "runtime", label: "asr.managed.runtime", values: runtimes, value: runtime, select: (value: string) => { setRuntimeId(value); setModelId(""); setAlignerId(""); setApplied(false); } },
    { key: "model", label: "asr.managed.model", values: models, value: model, select: (value: string) => { setModelId(value); setApplied(false); } },
    ...(config.engine === "qwen3-asr" ? [{ key: "aligner", label: "asr.managed.aligner", values: aligners, value: aligner, select: (value: string) => { setAlignerId(value); setApplied(false); } }] : []),
  ];
  return <FieldGroup>
    <FieldDescription>{t("asr.managed.intro")}</FieldDescription>
    <FieldDescription>{t("asr.managed.platformHint")}</FieldDescription>
    {manager.loading && <p role="status">{t("asr.managed.loading")}</p>}
    {!manager.loading && manager.catalog && runtimes.length === 0 && <Alert><AlertCircle /><AlertTitle>{t("asr.managed.unsupported")}</AlertTitle><AlertDescription>{t("asr.managed.unsupportedHint")}</AlertDescription></Alert>}
    {runtime && groups.filter((group) => group.values.length === 0).map((group) => <FieldDescription key={group.key} role="alert">{t("asr.managed.missingCompatible", { component: t(group.label) })}</FieldDescription>)}
    {groups.filter((group) => group.values.length > 0).map((group) => <Field key={group.key} data-disabled={locked}>
      <FieldLabel htmlFor={`${id}-${group.key}`}>{t(group.label)}</FieldLabel>
      <Select value={group.value?.id ?? ""} disabled={locked} onValueChange={group.select}>
        <SelectTrigger id={`${id}-${group.key}`} className="w-full"><SelectValue /></SelectTrigger>
        <SelectContent><SelectGroup>{group.values.map((item) => <SelectItem key={item.id} value={item.id}>{item.label}</SelectItem>)}</SelectGroup></SelectContent>
      </Select>
      {group.value && <ComponentDetails key={group.value.id} component={group.value} status={manager.components.find((item) => item.id === group.value!.id)} consents={manager.consents} disabled={locked} perform={manager.perform} t={t} />}
    </Field>)}
    {manager.operation && <Alert>
      <Loader2 className="spin" /><AlertTitle>{t(`asr.managed.phase.${manager.operation.phase}`)}</AlertTitle>
      <AlertDescription className="flex flex-col gap-2" aria-live="polite">
        <p>{manager.operation.component_id}</p>
        {manager.operation.message && <p>{manager.operation.message}</p>}
        {manager.operation.phase === "downloading" && <>
          <Progress value={componentProgressPercent(manager.operation)} aria-label={t("asr.managed.downloadProgress")} />
          <p>{componentStorageLabel(manager.operation.downloaded_bytes)} / {componentStorageLabel(manager.operation.total_bytes)}</p>
        </>}
        <Button type="button" variant="outline" disabled={manager.cancelling || isTerminalComponentPhase(manager.operation.phase)} onClick={() => void manager.cancel()}><X data-icon="inline-start" />{t(manager.cancelling ? "asr.managed.cancelling" : "asr.managed.cancel")}</Button>
      </AlertDescription>
    </Alert>}
    {runtimes.length > 0 && <>
      <FieldDescription>{t("asr.managed.spaceHint")}</FieldDescription>
      <Button type="button" disabled={locked || !selection || inUse} onClick={() => { if (selection && !locked) { onChange(selection); setApplied(true); } }}>{t(inUse ? "asr.managed.selected" : "asr.managed.use")}</Button>
      <FieldDescription>{t(applied && inUse ? "asr.managed.saveHint" : "asr.managed.selectHint")}</FieldDescription>
    </>}
    <div aria-live="polite">
      {manager.error && <Alert variant="destructive"><AlertCircle /><AlertTitle>{t("asr.managed.failed")}</AlertTitle><AlertDescription>{manager.error} {t("asr.managed.retryHint")}</AlertDescription></Alert>}
      {manager.notice && <p role="status">{manager.notice}</p>}
    </div>
    <Button type="button" variant="outline" disabled={locked} onClick={() => void manager.refresh()}><RefreshCw data-icon="inline-start" />{t("asr.managed.refresh")}</Button>
  </FieldGroup>;
}
