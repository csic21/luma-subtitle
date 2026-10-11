import { useEffect, useRef, useState } from "react";
import { errorText } from "@/lib/app-utils";
import { matchesAsrSetupConsent, replanAsrSetup, type AsrSetupPlan } from "@/lib/asr-setup-plan";
import type { AsrComponentStatus, AsrInstallConsent } from "@/lib/asr-components";
import type { useAsrComponents } from "@/hooks/use-asr-components";

type Controller = Pick<ReturnType<typeof useAsrComponents>, "refresh" | "perform" | "cancel">;
export type AsrSetupQueueResult =
  | { status: "success"; components: AsrComponentStatus[]; plan: AsrSetupPlan }
  | { status: "cancelled" | "busy" | "stale" }
  | { status: "error"; error: string };
export type AsrSetupQueueStatus = "idle" | "running" | "success" | "cancelled" | "error" | "stale";
type QueueState = {
  generation: number;
  status: AsrSetupQueueStatus;
  plan?: AsrSetupPlan;
  currentIndex: number;
  completedIds: string[];
  cancelling: boolean;
  finishingCurrent: boolean;
  error?: string;
};
type Run = { generation: number; cancelled: boolean; inStep: boolean; cancelPending: boolean; controller: Controller };
const initialState = (generation: number): QueueState => ({ generation, status: "idle", currentIndex: 0, completedIds: [], cancelling: false, finishingCurrent: false, error: undefined });

// The queue is deliberately independent of React state for its lock and stop
// latch: two clicks in one render and cancellation at an activation boundary
// must not start a second request. Backend commands still own all file changes.
export function useAsrSetupQueue(controller: Controller, selectionKey: string) {
  const scope = useRef({ key: selectionKey, generation: 0, mounted: true });
  if (scope.current.key !== selectionKey) {
    scope.current.key = selectionKey;
    scope.current.generation += 1;
  }
  const generation = scope.current.generation;
  const active = useRef<Run>();
  const latestController = useRef(controller);
  latestController.current = controller;
  const [state, setState] = useState<QueueState>(() => initialState(generation));
  const live = (run: Run) => scope.current.mounted && scope.current.generation === run.generation && active.current === run;
  const publish = (run: Run, patch: Partial<QueueState>) => {
    if (live(run)) setState((previous) => ({ ...previous, ...patch, generation: run.generation }));
  };
  const stop = async (run: Run) => {
    run.cancelled = true;
    publish(run, { cancelling: true, finishingCurrent: false, error: undefined });
    // Even a rejected cancellation leaves the no-next latch set. A failed IPC
    // is retryable while the current component is still running.
    if (!run.inStep || run.cancelPending) return;
    run.cancelPending = true;
    try {
      const result = await run.controller.cancel();
      if (result.status === "too_late") publish(run, { finishingCurrent: true });
      if (result.status === "error") {
        run.cancelPending = false;
        publish(run, { cancelling: false, error: result.error });
      }
    } catch (error) {
      run.cancelPending = false;
      publish(run, { cancelling: false, error: errorText(error) });
    }
  };

  useEffect(() => {
    scope.current.mounted = true;
    setState(initialState(scope.current.generation));
    return () => {
      scope.current.mounted = false;
      scope.current.generation += 1;
      if (active.current) void stop(active.current);
    };
    // The generation also changes for A → B → A, even if metadata returns to A.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectionKey]);

  const start = async (plan: AsrSetupPlan, acknowledged: Readonly<Record<string, AsrInstallConsent>> = {}, expectedGeneration = generation): Promise<AsrSetupQueueResult> => {
    if (!scope.current.mounted || scope.current.generation !== generation || expectedGeneration !== generation || plan.key !== scope.current.key) return { status: "stale" };
    if (active.current) return { status: "busy" };
    const run: Run = { generation, cancelled: false, inStep: false, cancelPending: false, controller: latestController.current };
    active.current = run;
    publish(run, { ...initialState(generation), status: "running", plan });
    const interrupted = (): AsrSetupQueueResult | undefined => !live(run) ? { status: "stale" }
      : run.cancelled ? { status: "cancelled" } : undefined;
    const finish = (result: AsrSetupQueueResult) => {
      publish(run, { status: result.status === "busy" ? "error" : result.status, cancelling: false, finishingCurrent: false,
        ...(result.status === "error" ? { error: result.error } : result.status === "busy" ? { error: "asr.managed.setupBusy" } : {}) });
      return result;
    };
    try {
      // Copy explicit acceptance before the first await. No frontend acceptance
      // cache is treated as a recorded backend receipt.
      const accepted = JSON.parse(JSON.stringify(acknowledged)) as Record<string, AsrInstallConsent>;
      // Every start, including Retry, hashes inventory again and replans. Never
      // reuse a partially completed list or the preview's installed statuses.
      const inventory = await run.controller.refresh();
      let stopped = interrupted();
      if (stopped) return finish(stopped);
      if (inventory.status !== "success") return finish(inventory);
      if (inventory.operation) return finish({ status: "busy" });
      const fresh = replanAsrSetup(plan, inventory);
      if (!fresh) return finish({ status: "stale" });
      // Refresh may remove finished work, but cannot silently expand what the
      // user reviewed (including a lost backend consent receipt).
      if (fresh.steps.some((step) => {
        const reviewed = plan.steps.find((item) => item.component.id === step.component.id);
        return !reviewed || reviewed.action !== step.action || (!reviewed.consentRequired && step.consentRequired);
      })) return finish({ status: "stale" });
      publish(run, { plan: fresh });
      for (const step of fresh.steps) {
        if (step.consentRequired && (!step.consent || !matchesAsrSetupConsent(step.consent, accepted[step.component.id]))) {
          return finish({ status: "error", error: "asr.managed.setupConsentRequired" });
        }
      }
      const completedIds: string[] = [];
      for (let index = 0; index < fresh.steps.length; index += 1) {
        stopped = interrupted();
        if (stopped) return finish(stopped);
        const step = fresh.steps[index];
        publish(run, { currentIndex: index });
        run.inStep = true;
        // Even receipt-authorized repairs must forward the exact signed plan
        // and complete terms: the backend validates them on every request.
        const result = await run.controller.perform(step.action, step.component.id, step.consentRequired ? accepted[step.component.id] : step.consent);
        run.inStep = false;
        if (result.status === "success") {
          completedIds.push(step.component.id);
          publish(run, { completedIds: [...completedIds], currentIndex: index + 1 });
        }
        stopped = interrupted();
        if (stopped) return finish(stopped);
        if (result.status !== "success") return finish(result);
      }
      const verified = await run.controller.refresh();
      stopped = interrupted();
      if (stopped) return finish(stopped);
      if (verified.status !== "success") return finish(verified);
      if (verified.operation) return finish({ status: "busy" });
      const complete = replanAsrSetup(fresh, verified);
      if (!complete) return finish({ status: "stale" });
      if (complete.steps.length) return finish({ status: "error", error: "asr.managed.setupVerificationFailed" });
      return finish({ status: "success", components: verified.components, plan: complete });
    } catch (error) {
      return finish(interrupted() ?? { status: "error", error: errorText(error) });
    } finally {
      run.inStep = false;
      if (active.current === run) active.current = undefined;
    }
  };
  const cancel = async () => {
    const run = active.current;
    if (!run || run.cancelPending || (run.cancelled && !run.inStep)) return;
    await stop(run);
  };
  const visible = state.generation === generation ? state : initialState(generation);
  return { ...visible, busy: visible.status === "running", start, cancel };
}
