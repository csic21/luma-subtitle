import { useEffect, useRef, useState } from "react";
import { asrConfigKey, normalizeAsrConfig } from "@/lib/asr-config";
import { errorText, hasTauriRuntime } from "@/lib/app-utils";
import { checkAsrBackend, releaseAsrBackend } from "@/lib/tauri-api";
import type { AsrBackendStatus, AsrConfig, TFunction } from "@/types";

type CheckState = {
  epoch: number;
  busy?: "check" | "release";
  result?: AsrBackendStatus;
  error?: string;
  notice?: string;
};

export function useAsrBackendCheck(config: AsrConfig, t: TFunction) {
  const key = asrConfigKey(config);
  const scope = useRef({ key, epoch: 0, mounted: true });
  // Invalidate immediately on every configuration transition, including A → B → A.
  if (scope.current.key !== key) {
    scope.current.key = key;
    scope.current.epoch += 1;
  }
  const [state, setState] = useState<CheckState>({ epoch: -1 });
  useEffect(() => {
    scope.current.mounted = true;
    return () => {
      scope.current.mounted = false;
      scope.current.epoch += 1;
    };
  }, []);

  const run = async (action: "check" | "release") => {
    const epoch = ++scope.current.epoch;
    const publish = (next: Omit<CheckState, "epoch">) => {
      if (scope.current.mounted && scope.current.epoch === epoch && scope.current.key === key) {
        setState({ epoch, ...next });
      }
    };
    if (!hasTauriRuntime()) {
      publish({ error: t("notice.requireTauriConfig") });
      return;
    }
    publish({ busy: action });
    try {
      if (action === "check") {
        publish({ result: await checkAsrBackend(normalizeAsrConfig(config)) });
      } else {
        const released = await releaseAsrBackend();
        publish({ notice: t(released ? "asr.released" : "asr.nothingToRelease") });
      }
    } catch (error) {
      publish({ error: errorText(error) });
    }
  };

  const current = state.epoch === scope.current.epoch ? state : undefined;
  return {
    checking: current?.busy === "check",
    releasing: current?.busy === "release",
    result: current?.result,
    error: current?.error,
    notice: current?.notice,
    check: () => run("check"),
    release: () => run("release"),
  };
}
