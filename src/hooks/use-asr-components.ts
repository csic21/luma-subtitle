import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { errorText, hasTauriRuntime } from "@/lib/app-utils";
import {
  asrComponentCatalog, asrComponentStatus, cancelAsrComponent,
  installAsrComponent, removeAsrComponent, repairAsrComponent,
} from "@/lib/tauri-api";
import { isTerminalComponentPhase, type AsrComponentAction, type AsrComponentCatalog, type AsrComponentProgress, type AsrComponentStatus, type AsrInstallConsent, type AsrRecordedConsent } from "@/lib/asr-components";
import type { TFunction } from "@/types";

export type AsrComponentResult =
  | { status: "success"; component: AsrComponentStatus }
  | { status: "cancelled" | "busy" }
  | { status: "error"; error: string };
export type AsrComponentCancelResult =
  | { status: "accepted" | "too_late" | "idle" }
  | { status: "error"; error: string };
export type AsrComponentsRefreshResult =
  | { status: "success"; catalog: AsrComponentCatalog; components: AsrComponentStatus[]; consents: AsrRecordedConsent[]; operation?: AsrComponentProgress }
  | { status: "cancelled" | "busy" }
  | { status: "error"; error: string };

type State = {
  epoch: number;
  loading: boolean;
  verified: boolean;
  catalog?: AsrComponentCatalog;
  components: AsrComponentStatus[];
  consents: AsrRecordedConsent[];
  operation?: AsrComponentProgress;
  cancelling?: boolean;
  error?: string;
  notice?: string;
};
type ActiveRequest = { id: string; componentId: string; own: boolean; cancelling: boolean };
const actions = { install: installAsrComponent, repair: repairAsrComponent, remove: removeAsrComponent };

export function useAsrComponents(engine: string, t: TFunction) {
  const scope = useRef({ engine, epoch: 0, mounted: true });
  if (scope.current.engine !== engine) {
    scope.current.engine = engine;
    scope.current.epoch += 1;
  }
  const epoch = scope.current.epoch;
  const active = useRef<ActiveRequest>();
  const listener = useRef<UnlistenFn>();
  const loadVersion = useRef(0);
  const lifecycleVersion = useRef(0);
  const lastProgress = useRef<AsrComponentProgress>();
  const loadingRef = useRef(false);
  const eventsDuringLoad = useRef(new Map<string, AsrComponentProgress>());
  const [state, setState] = useState<State>({ epoch, loading: true, verified: false, components: [], consents: [] });
  const current = () => scope.current.mounted && scope.current.epoch === epoch;
  const publish = (patch: Partial<State>) => {
    if (current()) setState((previous) => ({ ...previous, ...patch, epoch }));
  };

  const refresh = async (): Promise<AsrComponentsRefreshResult> => {
    if (!current()) return { status: "cancelled" };
    if (loadingRef.current || active.current?.own) return { status: "busy" };
    if (!hasTauriRuntime()) {
      publish({ loading: false, error: t("notice.requireTauriConfig") });
      return { status: "error", error: t("notice.requireTauriConfig") };
    }
    const version = ++loadVersion.current;
    const lifecycle = lifecycleVersion.current;
    loadingRef.current = true;
    publish({ loading: true, verified: false, error: undefined });
    try {
      if (!listener.current) {
        const unlisten = await listen<AsrComponentProgress>("asr-component-progress", ({ payload }) => {
          if (!current() || lifecycleVersion.current !== lifecycle) return;
          if (loadingRef.current) {
            if (eventsDuringLoad.current.size >= 20) eventsDuringLoad.current.clear();
            eventsDuringLoad.current.set(payload.request_id, payload);
          }
          if (active.current?.id !== payload.request_id || active.current.componentId !== payload.component_id) return;
          lastProgress.current = payload;
          publish({ operation: payload });
          if (!active.current.own && isTerminalComponentPhase(payload.phase)) {
            active.current = undefined;
            publish({ operation: undefined, cancelling: false, notice: t(`asr.managed.phase.${payload.phase}`) });
            void refresh();
          }
        });
        if (!current() || lifecycleVersion.current !== lifecycle) { unlisten(); return { status: "cancelled" }; }
        listener.current = unlisten;
      }
      const [catalog, initialSnapshot] = await Promise.all([asrComponentCatalog(), asrComponentStatus()]);
      if (!current() || loadVersion.current !== version) return { status: "cancelled" };
      // A terminal event can arrive while a status response is in flight. Reconcile
      // that response before adopting a recovered operation, or it could stay busy forever.
      let snapshot = initialSnapshot;
      // Hashing may outlast an external install/remove, even if operation is null
      // by the time the snapshot is returned. Re-read after every terminal change.
      while ([...eventsDuringLoad.current.values()].some((event) => isTerminalComponentPhase(event.phase))) {
        eventsDuringLoad.current.clear();
        snapshot = await asrComponentStatus();
        if (!current() || loadVersion.current !== version) return { status: "cancelled" };
      }
      const latestSeen = [...eventsDuringLoad.current.values()].pop();
      const recovered = snapshot.operation
        ? eventsDuringLoad.current.get(snapshot.operation.request_id) ?? snapshot.operation
        : latestSeen;
      const operation = recovered && !isTerminalComponentPhase(recovered.phase) ? recovered : undefined;
      eventsDuringLoad.current.clear();
      active.current = operation ? { id: operation.request_id, componentId: operation.component_id, own: false, cancelling: false } : undefined;
      publish({ catalog, components: snapshot.components, consents: snapshot.consents ?? [], operation, loading: false, verified: true, cancelling: false });
      return { status: "success", catalog, components: snapshot.components, consents: snapshot.consents ?? [], operation };
    } catch (error) {
      if (!current() || loadVersion.current !== version) return { status: "cancelled" };
      publish({ loading: false, error: errorText(error) });
      return { status: "error", error: errorText(error) };
    } finally {
      if (current() && loadVersion.current === version) loadingRef.current = false;
    }
  };

  useEffect(() => {
    scope.current.mounted = true;
    active.current = undefined;
    loadingRef.current = false;
    listener.current = undefined;
    lastProgress.current = undefined;
    eventsDuringLoad.current.clear();
    setState({ epoch, loading: true, verified: false, components: [], consents: [] });
    void refresh();
    return () => {
      scope.current.mounted = false;
      // A pending listener/command from this view must never publish into a later view.
      lifecycleVersion.current += 1;
      loadVersion.current += 1;
      listener.current?.();
      listener.current = undefined;
      if (active.current?.own) void cancelAsrComponent(active.current.id).catch(() => {});
      active.current = undefined;
    };
    // This effect owns the subscription for one engine generation, not a render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine]);

  const perform = async (action: AsrComponentAction, componentId: string, consent?: AsrInstallConsent): Promise<AsrComponentResult> => {
    if (!current()) return { status: "cancelled" };
    if (active.current || loadingRef.current || !listener.current) return { status: "busy" };
    const requestId = crypto.randomUUID();
    const lifecycle = lifecycleVersion.current;
    const ownsRequest = () => current() && lifecycleVersion.current === lifecycle && active.current?.id === requestId;
    active.current = { id: requestId, componentId, own: true, cancelling: false };
    lastProgress.current = undefined;
    publish({ error: undefined, notice: undefined, cancelling: false, operation: { request_id: requestId, component_id: componentId, phase: action === "remove" ? "removing" : "preparing", downloaded_bytes: 0, total_bytes: 0, message: "" } });
    try {
      const installed = await (consent ? actions[action](componentId, requestId, consent) : actions[action](componentId, requestId));
      if (!ownsRequest()) return { status: "cancelled" };
      setState((previous) => ({ ...previous, components: [...previous.components.filter((item) => item.id !== installed.id), installed] }));
      publish({ notice: t(action === "remove" ? "asr.managed.removed" : "asr.managed.installedNotice") });
      if (consent) {
        // Reuse only a receipt returned by the backend, never a frontend acceptance cache.
        publish({ loading: true });
        try {
          const snapshot = await asrComponentStatus();
          if (ownsRequest()) publish({ consents: snapshot.consents ?? [] });
        } catch {
          if (ownsRequest()) publish({ consents: [] });
        }
      }
      if (!ownsRequest()) return { status: "cancelled" };
      return { status: "success", component: installed };
    } catch (error) {
      if (!ownsRequest()) return { status: "cancelled" };
      const progress = lastProgress.current as AsrComponentProgress | undefined;
      if (progress?.request_id === requestId && progress.phase === "cancelled") {
        publish({ notice: t("asr.managed.phase.cancelled") });
      } else {
        publish({ error: errorText(error) });
      }
      // A failed repair may change integrity state. Re-read it before enabling use.
      publish({ loading: true });
      try {
        const snapshot = await asrComponentStatus();
        if (ownsRequest()) publish({ components: snapshot.components, consents: snapshot.consents ?? [], verified: true });
      } catch {
        if (ownsRequest()) publish({ verified: false });
      }
      if (!ownsRequest()) return { status: "cancelled" };
      return progress?.request_id === requestId && progress.phase === "cancelled"
        ? { status: "cancelled" } : { status: "error", error: errorText(error) };
    } finally {
      if (ownsRequest()) {
        active.current = undefined;
        publish({ operation: undefined, cancelling: false, loading: false });
      }
    }
  };

  const cancel = async (): Promise<AsrComponentCancelResult> => {
    const request = active.current;
    if (!current() || !request || request.cancelling || (lastProgress.current && isTerminalComponentPhase(lastProgress.current.phase))) return { status: "idle" };
    request.cancelling = true;
    publish({ cancelling: true, error: undefined });
    try {
      const accepted = await cancelAsrComponent(request.id);
      if (current() && active.current?.id === request.id && !accepted) {
        publish({ notice: t("asr.managed.cancelTooLate") });
      }
      return { status: accepted ? "accepted" : "too_late" };
    } catch (error) {
      if (current() && active.current?.id === request.id) {
        request.cancelling = false;
        publish({ cancelling: false, error: errorText(error) });
      }
      return { status: "error", error: errorText(error) };
    }
  };

  const visible: State = state.epoch === epoch ? state : { epoch, loading: true, verified: false, components: [], consents: [] };
  return { ...visible, busy: !!visible.operation, refresh, perform, cancel };
}
