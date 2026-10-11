import { useEffect, useId, useMemo, useRef, useState } from "react";
import { AlertCircle, CheckCircle2, Download, Loader2, RefreshCw, Trash2, X } from "lucide-react";
import { AsrSetupDialog } from "@/components/app/asr-setup-dialog";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { useAsrSetupQueue } from "@/hooks/use-asr-setup-queue";
import { asrSetupSelectionKey, createAsrSetupPlan, isAsrSetupComponentReady, type AsrSetupPlan } from "@/lib/asr-setup-plan";
import { componentStorageLabel } from "@/lib/component-storage";
export { componentStorageLabel } from "@/lib/component-storage";
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

function ComponentDetails({ component, status, consents, disabled, perform, t }: {
  component: AsrRuntimeComponent | AsrModelComponent;
  status?: AsrComponentStatus;
  consents: AsrRecordedConsent[];
  disabled: boolean;
  perform: (action: AsrComponentAction, id: string, consent?: AsrInstallConsent) => Promise<unknown>;
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
      {runtime?.recipe?.windows_crt && <FieldDescription>{t("asr.managed.windowsTrustNetwork")}</FieldDescription>}
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
  const candidate = manager.verified ? managedAsrConfig(config, runtime, model, aligner, manager.components) : null;
  const selection = candidate && [runtime, model, ...(config.engine === "qwen3-asr" ? [aligner] : [])].every((item) => item && isAsrSetupComponentReady(item, manager.components)) ? candidate : null;
  const inUse = !!selection && asrConfigsEqual(selection, config);
  const selectionKey = useMemo(() => asrSetupSelectionKey({ engine: config.engine, runtime, model, aligner }), [config.engine, runtime, model, aligner]);
  const queue = useAsrSetupQueue(manager, selectionKey);
  const plan = useMemo(() => createAsrSetupPlan({ engine: config.engine, runtime, model, aligner, components: manager.components, consents: manager.consents, verified: manager.verified }), [config.engine, runtime, model, aligner, manager.components, manager.consents, manager.verified]);
  const [pendingPlan, setPendingPlan] = useState<{ plan: AsrSetupPlan; generation: number; queueGeneration: number }>();
  const scope = useRef({ key: selectionKey, generation: 0, mounted: true, locked: false });
  if (scope.current.key !== selectionKey) {
    scope.current.key = selectionKey;
    scope.current.generation += 1;
  }
  const generation = scope.current.generation;
  const locked = disabled || manager.loading || manager.busy || queue.busy;
  scope.current.locked = locked;
  useEffect(() => { scope.current.mounted = true; return () => { scope.current.mounted = false; scope.current.generation += 1; }; }, []);
  useEffect(() => { setPendingPlan(undefined); setApplied(false); }, [selectionKey]);
  const queuePlan = queue.plan;
  const activeStep = queuePlan?.steps[queue.currentIndex];
  const completedBytes = queuePlan?.steps.filter((step) => queue.completedIds.includes(step.component.id)).reduce((sum, step) => sum + step.downloadBytes, 0) ?? 0;
  const currentBytes = activeStep && manager.operation?.component_id === activeStep.component.id
    ? Math.min(activeStep.downloadBytes, Math.max(0, manager.operation.downloaded_bytes)) : 0;
  const downloadedBytes = Math.min(queuePlan?.downloadBytes ?? 0, completedBytes + currentBytes);
  const queuePercent = queuePlan?.downloadBytes ? Math.floor(downloadedBytes / queuePlan.downloadBytes * 100) : null;
  const stopped = queue.status === "cancelled" || queue.status === "error" || queue.status === "stale";
  const selectedComponents = [runtime, model, ...(config.engine === "qwen3-asr" ? [aligner] : [])].filter((item): item is AsrRuntimeComponent | AsrModelComponent => !!item);
  const labels = ["asr.setup.runtime", "asr.setup.model", "asr.setup.aligner"];
  const ready = (component: AsrRuntimeComponent | AsrModelComponent) => manager.verified && isAsrSetupComponentReady(component, manager.components);
  const apply = () => {
    if (scope.current.mounted && scope.current.generation === generation && selection && !scope.current.locked) { onChange(selection); setApplied(true); }
  };
  const groups = [
    { key: "runtime", label: "asr.managed.runtime", values: runtimes, value: runtime, select: (value: string) => { setRuntimeId(value); setModelId(""); setAlignerId(""); setApplied(false); } },
    { key: "model", label: "asr.managed.model", values: models, value: model, select: (value: string) => { setModelId(value); setApplied(false); } },
    ...(config.engine === "qwen3-asr" ? [{ key: "aligner", label: "asr.managed.aligner", values: aligners, value: aligner, select: (value: string) => { setAlignerId(value); setApplied(false); } }] : []),
  ];
  return <FieldGroup>
    {manager.loading && !queue.busy && <p role="status">{t("asr.managed.loading")}</p>}
    {!manager.loading && manager.catalog && runtimes.length === 0 && <Alert><AlertCircle /><AlertTitle>{t("asr.managed.unsupported")}</AlertTitle><AlertDescription>{t("asr.managed.unsupportedHint")}</AlertDescription></Alert>}
    {runtime && <Card size="sm" data-testid="asr-setup-card">
      <CardHeader>
        <CardTitle>{t(config.engine === "qwen3-asr" ? "asr.setup.qwenTitle" : "asr.setup.whisperTitle")}</CardTitle>
        <CardDescription className="flex flex-wrap items-center gap-2">
          <span>{runtime.platform === "windows-x64" ? "Windows" : "Apple Silicon"}{" · "}{runtime.device === "metal" ? "Metal" : "CPU"}</span>
          <Badge variant="secondary">{t("asr.setup.experimental")}</Badge>
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <ul className="flex flex-col gap-2" aria-label={t("asr.setup.components")}>
          {selectedComponents.map((component, index) => <li key={component.id} className="flex flex-wrap items-center justify-between gap-2">
            <span>{t(labels[index])}{index === 1 && <span className="ml-2 text-muted-foreground">{component.label.split(" (")[0]}</span>}</span>
            <span className="flex items-center gap-1 text-muted-foreground">{ready(component) ? <><CheckCircle2 className="size-4" aria-hidden="true" />{t("asr.setup.ready")}</> : componentStorageLabel(componentDownloadBytes(component))}</span>
          </li>)}
        </ul>
        {groups.filter((group) => group.values.length === 0).map((group) => <FieldDescription key={group.key} role="alert">{t("asr.managed.missingCompatible", { component: t(group.label) })}</FieldDescription>)}
        {queue.busy ? <div className="flex flex-col gap-2" role="status" aria-live="polite">
          <span>{queue.cancelling ? t("asr.setup.stopping") : queuePlan?.steps.length ? t("asr.setup.step", { current: Math.min(queue.currentIndex + 1, queuePlan.steps.length), total: queuePlan.steps.length }) : t("asr.managed.phase.verifying")}{" · "}{t(manager.operation ? `asr.managed.phase.${manager.operation.phase}` : "asr.managed.phase.verifying")}</span>
          {activeStep && <FieldDescription>{t("platform" in activeStep.component ? "asr.setup.runtime" : activeStep.component.role === "aligner" ? "asr.setup.aligner" : "asr.setup.model")}</FieldDescription>}
          <Progress value={queuePercent} aria-label={t("asr.setup.overallProgress")} />
          <FieldDescription>{componentStorageLabel(downloadedBytes)} / {componentStorageLabel(queuePlan?.downloadBytes ?? 0)}</FieldDescription>
          {queue.error && <FieldDescription role="alert">{t(queue.error).split("\n")[0].slice(0, 240)}</FieldDescription>}
          <FieldDescription>{t(queue.cancelling && (queue.finishingCurrent || manager.operation?.phase === "activating") ? "asr.setup.finishingCurrent" : "asr.setup.leaveHint")}</FieldDescription>
        </div> : <>
          <p role="status">{selection ? t("asr.setup.allReady") : plan ? t("asr.setup.missing", { count: plan.steps.length, size: componentStorageLabel(plan.downloadBytes) }) : t("asr.setup.unavailable")}</p>
          {stopped && <FieldDescription role={queue.status === "error" ? "alert" : "status"}>{t(`asr.setup.${queue.status}`)}{queue.error && <span className="block break-words">{t(queue.error).split("\n")[0].slice(0, 240)}</span>}</FieldDescription>}
          {config.engine === "qwen3-asr" && <FieldDescription>{t("asr.setup.qwenHint")}</FieldDescription>}
        </>}
      </CardContent>
      <CardFooter className="flex flex-col items-stretch gap-2">
        {queue.busy ? <Button type="button" variant="outline" disabled={queue.cancelling} onClick={() => void queue.cancel()}><X data-icon="inline-start" />{t(queue.cancelling ? "asr.managed.cancelling" : "asr.managed.cancel")}</Button>
          : selection ? <Button type="button" disabled={locked || inUse} onClick={apply}><CheckCircle2 data-icon="inline-start" />{t(inUse ? "asr.managed.selected" : "asr.managed.use")}</Button>
          : <Button type="button" disabled={locked || !plan || plan.steps.length === 0} onClick={() => {
            if (!scope.current.mounted || scope.current.generation !== generation || scope.current.locked || !plan || plan.steps.length === 0) return;
            setPendingPlan({ plan, generation: scope.current.generation, queueGeneration: queue.generation });
          }}><Download data-icon="inline-start" />{t(stopped ? "asr.setup.retry" : "asr.setup.install")}</Button>}
        {applied && inUse && <FieldDescription role="status">{t("asr.managed.saveHint")}</FieldDescription>}
      </CardFooter>
    </Card>}
    {pendingPlan && pendingPlan.generation === scope.current.generation && <AsrSetupDialog key={`${pendingPlan.generation}-${pendingPlan.plan.key}`} plan={pendingPlan.plan} disabled={locked} t={t} onCancel={() => setPendingPlan(undefined)} onAccept={(consents) => {
      if (!scope.current.mounted || scope.current.locked || scope.current.generation !== pendingPlan.generation) return;
      setPendingPlan(undefined);
      void queue.start(pendingPlan.plan, consents, pendingPlan.queueGeneration);
    }} />}
    <details>
      <summary className="cursor-pointer">{t("asr.setup.advanced")}</summary>
      <FieldGroup className="mt-4">
        {groups.filter((group) => group.values.length > 0).map((group) => <Field key={group.key} data-disabled={locked}>
          <FieldLabel htmlFor={`${id}-${group.key}`}>{t(group.label)}</FieldLabel>
          <Select value={group.value?.id ?? ""} disabled={locked} onValueChange={group.select}>
            <SelectTrigger id={`${id}-${group.key}`} className="w-full"><SelectValue /></SelectTrigger>
            <SelectContent><SelectGroup>{group.values.map((item) => <SelectItem key={item.id} value={item.id}>{item.label}</SelectItem>)}</SelectGroup></SelectContent>
          </Select>
          {group.value && <ComponentDetails key={group.value.id} component={group.value} status={manager.components.find((item) => item.id === group.value!.id)} consents={manager.consents} disabled={locked} perform={manager.perform} t={t} />}
        </Field>)}
        <FieldDescription>{t("asr.setup.diskHint")}</FieldDescription>
        {queue.error && <FieldDescription role="alert">{t(queue.error)}</FieldDescription>}
        <Button type="button" variant="outline" disabled={locked} onClick={() => void manager.refresh()}><RefreshCw data-icon="inline-start" />{t("asr.managed.refresh")}</Button>
      </FieldGroup>
    </details>
    {manager.operation && !queue.busy && <Alert>
      <Loader2 className="spin" /><AlertTitle>{t(`asr.managed.phase.${manager.operation.phase}`)}</AlertTitle>
      <AlertDescription className="flex flex-col gap-2" aria-live="polite">
        {manager.operation.phase === "downloading" && <>
          <Progress value={componentProgressPercent(manager.operation)} aria-label={t("asr.managed.downloadProgress")} />
          <p>{componentStorageLabel(manager.operation.downloaded_bytes)} / {componentStorageLabel(manager.operation.total_bytes)}</p>
        </>}
        <Button type="button" variant="outline" disabled={manager.cancelling || isTerminalComponentPhase(manager.operation.phase)} onClick={() => void manager.cancel()}><X data-icon="inline-start" />{t(manager.cancelling ? "asr.managed.cancelling" : "asr.managed.cancel")}</Button>
      </AlertDescription>
    </Alert>}
    <div aria-live="polite">
      {manager.error && !queue.error && <Alert variant="destructive"><AlertCircle /><AlertTitle>{t("asr.managed.failed")}</AlertTitle><AlertDescription>{manager.error}</AlertDescription></Alert>}
      {manager.notice && !queuePlan && <p role="status">{manager.notice}</p>}
    </div>
  </FieldGroup>;
}
