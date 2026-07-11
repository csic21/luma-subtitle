import { type SetStateAction, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  canRunOperation,
  errorText,
  hasTauriRuntime,
  isTranslateStage,
  operationLabel,
  operationRequirementIssues,
  operationRequirementSummary,
  taskBusy,
} from "@/lib/app-utils";
import {
  applyCurrentSettingsToTask,
  cancelTask as cancelTaskCommand,
  checkEnvironment,
  getTask,
  getTaskLogs,
  loadSettings,
  openPath,
  runTaskOperation,
  selectWhisperModel,
  subtitlePreview as loadSubtitlePreview,
  updateTaskSettings,
} from "@/lib/tauri-api";
import {
  appendRealtimeLog,
  mergeTaskLogs,
  normalizeTaskSettings,
  shouldReplaceTaskSettingsDraft,
  taskSettingsEqual,
  taskSettingsUpdatePayload,
} from "@/lib/task-data";
import type {
  EnvironmentState,
  JobEvent,
  SettingsState,
  SubtitlePreview,
  TaskOperation,
  TaskRecord,
  TaskSettingsSnapshot,
  TFunction,
} from "@/types";

import { useAppResume } from "./use-app-resume";

type SubtitleView = "translated" | "source";

type FlowStep = {
  label: string;
  detail: string;
  state: string;
};

export function useTaskDetailState(taskId: string, t: TFunction) {
  const [task, setTask] = useState<TaskRecord | null>(null);
  const [settingsDraft, setSettingsDraftState] = useState<TaskSettingsSnapshot | null>(null);
  const [globalSettings, setGlobalSettings] = useState<Pick<SettingsState, "has_api_key"> | null>(null);
  const [env, setEnv] = useState<EnvironmentState | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [subtitlePreview, setSubtitlePreview] = useState<SubtitlePreview | null>(null);
  const [subtitleView, setSubtitleView] = useState<SubtitleView>("source");
  const [notice, setNotice] = useState("");
  const taskRef = useRef<TaskRecord | null>(null);
  const settingsDraftRef = useRef<TaskSettingsSnapshot | null>(null);
  taskRef.current = task;
  settingsDraftRef.current = settingsDraft;

  const setSettingsDraft = useCallback((next: SetStateAction<TaskSettingsSnapshot | null>) => {
    const current = settingsDraftRef.current;
    const resolved = typeof next === "function" ? next(current) : next;
    settingsDraftRef.current = resolved;
    setSettingsDraftState(resolved);
  }, []);

  const syncTask = useCallback(
    (nextTask: TaskRecord, forceSettingsDraft = false) => {
      const previousTask = taskRef.current;
      taskRef.current = nextTask;
      setTask(nextTask);
      if (shouldReplaceTaskSettingsDraft(previousTask, settingsDraftRef.current, nextTask, forceSettingsDraft)) {
        setSettingsDraft(normalizeTaskSettings(nextTask.settings));
      }
    },
    [setSettingsDraft],
  );

  const tauriReady = hasTauriRuntime();
  const operationContext = useMemo(
    () => ({
      environmentReady: Boolean(env?.ffmpeg_path && env?.whisper_path),
      hasApiCredential: Boolean(globalSettings?.has_api_key),
    }),
    [env?.ffmpeg_path, env?.whisper_path, globalSettings?.has_api_key],
  );

  const refreshLogs = useCallback(async () => {
    try {
      setLogs(await getTaskLogs(taskId));
    } catch {
      setLogs([]);
    }
  }, [taskId]);

  const refreshPreview = useCallback(
    async (currentTask = taskRef.current) => {
      if (!currentTask?.source_srt_path) return;
      try {
        const preview = await loadSubtitlePreview(taskId);
        setSubtitlePreview(preview);
        setSubtitleView(preview.translated_srt?.trim() ? "translated" : "source");
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [taskId],
  );

  const refreshTask = useCallback(
    async (options: { preview?: boolean } = {}) => {
      try {
        const [loaded, loadedLogs] = await Promise.all([getTask(taskId), getTaskLogs(taskId).catch(() => [])]);
        syncTask(loaded);
        setLogs((current) => mergeTaskLogs(loadedLogs, current));
        if ((options.preview ?? true) && loaded.source_srt_path) await refreshPreview(loaded);
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [refreshPreview, syncTask, taskId],
  );

  const refreshRunPrerequisites = useCallback(async () => {
    try {
      const [loadedSettings, loadedEnv] = await Promise.all([loadSettings(), checkEnvironment()]);
      setGlobalSettings({ has_api_key: loadedSettings.has_api_key });
      setEnv(loadedEnv);
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  useEffect(() => {
    if (!taskId) return;
    if (!tauriReady) {
      setNotice(t("notice.requireTauriDetails"));
      return;
    }

    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenJob: (() => void) | undefined;

    void refreshTask();
    void refreshRunPrerequisites();

    listen<TaskRecord>("task-updated", (event) => {
      if (event.payload.id !== taskId) return;
      const previousTask = taskRef.current;
      const subtitlePathsChanged =
        previousTask?.source_srt_path !== event.payload.source_srt_path ||
        previousTask?.translated_srt_path !== event.payload.translated_srt_path;
      syncTask(event.payload);
      if (subtitlePathsChanged && event.payload.source_srt_path) void refreshPreview(event.payload);
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenTask = fn;
    }).catch((error) => {
      if (!disposed) setNotice(errorText(error));
    });

    listen<JobEvent>("job-event", (event) => {
      if (event.payload.job_id !== taskId) return;
      setLogs((current) => appendRealtimeLog(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenJob = fn;
    }).catch((error) => {
      if (!disposed) setNotice(errorText(error));
    });

    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenJob?.();
    };
  }, [refreshPreview, refreshRunPrerequisites, refreshTask, syncTask, t, taskId, tauriReady]);

  const refreshTaskOnResume = useCallback(() => {
    void refreshTask({ preview: false });
  }, [refreshTask]);

  useAppResume(refreshTaskOnResume, Boolean(taskId && tauriReady));

  const runOperation = useCallback(
    async (operation: TaskOperation) => {
      const currentTask = taskRef.current;
      if (currentTask && !canRunOperation(currentTask, operation, operationContext)) {
        setNotice(operationRequirementSummary(operationRequirementIssues(currentTask, operation, operationContext), t));
        return;
      }
      try {
        await runTaskOperation(taskId, operation);
        await refreshTask();
        setNotice(t("notice.addedToQueue", { operation: operationLabel(operation, t) }));
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [operationContext, refreshTask, t, taskId],
  );

  const cancelTask = useCallback(async () => {
    try {
      await cancelTaskCommand(taskId);
      await refreshTask();
    } catch (error) {
      setNotice(errorText(error));
    }
  }, [refreshTask, taskId]);

  const applyCurrentSettings = useCallback(async () => {
    try {
      const updated = await applyCurrentSettingsToTask(taskId);
      syncTask(updated, true);
      await refreshLogs();
      setNotice(t("notice.settingsApplied"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }, [refreshLogs, syncTask, t, taskId]);

  const saveTaskSettings = useCallback(async () => {
    if (!settingsDraft) return;
    try {
      const updated = await updateTaskSettings(taskId, taskSettingsUpdatePayload(settingsDraft));
      syncTask(updated, true);
      await refreshLogs();
      setNotice(t("notice.taskSettingsSaved"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }, [refreshLogs, settingsDraft, syncTask, t, taskId]);

  const pickTaskWhisperModel = useCallback(async () => {
    try {
      const picked = await selectWhisperModel();
      if (picked) setSettingsDraft((current) => (current ? { ...current, whisper_model_path: picked } : current));
    } catch (error) {
      setNotice(t("error.pickWhisperModel", { error: errorText(error) }));
    }
  }, [t]);

  const openOutputDir = useCallback(async () => {
    const target = task?.exported_output_dir || task?.output_dir || task?.settings.output_dir;
    if (!target) return;
    try {
      await openPath(target);
    } catch (error) {
      setNotice(errorText(error));
    }
  }, [task]);

  const flowSteps = useMemo<FlowStep[]>(() => {
    if (!task) return [];
    return [
      {
        label: t("flow.material"),
        detail: task.file_name,
        state: "done",
      },
      {
        label: t("common.transcribe"),
        detail: task.source_srt_path ? t("flow.sourceReady") : task.message,
        state: task.source_srt_path ? "done" : taskBusy(task) ? "active" : "idle",
      },
      {
        label: t("common.translate"),
        detail: task.translated_srt_path ? t("flow.translationReady") : t("flow.waitingTranslation"),
        state: task.translated_srt_path ? "done" : isTranslateStage(task.stage) ? "active" : "idle",
      },
      {
        label: t("common.export"),
        detail: task.exported_output_dir ? t("flow.exported") : t("flow.waitingExport"),
        state: task.exported_output_dir ? "done" : task.stage === "exporting" ? "active" : "idle",
      },
    ];
  }, [task, t]);

  const hasTranslatedSubtitle = Boolean(subtitlePreview?.translated_srt?.trim());
  const activeSubtitleBody =
    subtitleView === "translated" && hasTranslatedSubtitle
      ? subtitlePreview?.translated_srt
      : subtitlePreview?.source_srt;
  const activeSubtitleFileName =
    subtitleView === "translated" && hasTranslatedSubtitle
      ? subtitlePreview?.translated_file_name
      : subtitlePreview?.source_file_name;
  const taskSettingsDirty = useMemo(() => {
    if (!task || !settingsDraft) return false;
    return !taskSettingsEqual(task.settings, settingsDraft);
  }, [settingsDraft, task?.settings]);
  const taskConfig = useMemo(
    () => (task ? settingsDraft ?? normalizeTaskSettings(task.settings) : null),
    [settingsDraft, task?.settings],
  );

  return {
    activeSubtitleBody,
    activeSubtitleFileName,
    applyCurrentSettings,
    cancelTask,
    flowSteps,
    hasTranslatedSubtitle,
    logs,
    notice,
    openOutputDir,
    operationContext,
    pickTaskWhisperModel,
    refreshPreview,
    runOperation,
    saveTaskSettings,
    setSettingsDraft,
    setSubtitleView,
    subtitlePreview,
    subtitleView,
    task,
    taskConfig,
    taskSettingsDirty,
  };
}
