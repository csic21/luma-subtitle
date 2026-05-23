import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { defaultSettings } from "@/config";
import {
  canRunOperation,
  errorText,
  hasTauriRuntime,
  operationLabel,
  operationRequirementIssues,
  operationRequirementSummary,
  taskBusy,
} from "@/lib/app-utils";
import {
  cancelTask as cancelTaskCommand,
  checkEnvironment,
  createAudioTask as createAudioTaskCommand,
  createSrtTask as createSrtTaskCommand,
  createVideoTask as createVideoTaskCommand,
  deleteTask as deleteTaskCommand,
  listTasks,
  loadQueueSettings,
  loadSettings,
  runTaskOperation,
  runTaskOperations,
  saveQueueSettings as saveQueueSettingsCommand,
  selectOutputDir,
  selectAudio,
  selectSrt,
  selectVideo,
} from "@/lib/tauri-api";
import { taskCreatePayload, upsertTask } from "@/lib/task-data";
import type { EnvironmentState, QueueSettings, SettingsState, TaskOperation, TaskRecord, TFunction } from "@/types";

import { useAppResume } from "./use-app-resume";

const defaultQueueSettings: QueueSettings = {
  max_concurrency: 2,
  auto_start_next: false,
};

function removeTask(tasks: TaskRecord[], taskId: string) {
  const index = tasks.findIndex((task) => task.id === taskId);
  if (index === -1) return tasks;
  return [...tasks.slice(0, index), ...tasks.slice(index + 1)];
}

function removeSelectedId(selectedIds: Set<string>, taskId: string) {
  if (!selectedIds.has(taskId)) return selectedIds;
  const next = new Set(selectedIds);
  next.delete(taskId);
  return next;
}

export function useTasksPageState(t: TFunction) {
  const [tasks, setTasks] = useState<TaskRecord[]>([]);
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [env, setEnv] = useState<EnvironmentState | null>(null);
  const [queueSettings, setQueueSettings] = useState<QueueSettings>(defaultQueueSettings);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [outputDir, setOutputDir] = useState("");
  const [notice, setNotice] = useState("");
  const tasksRef = useRef(tasks);
  const selectedIdsRef = useRef(selectedIds);
  tasksRef.current = tasks;
  selectedIdsRef.current = selectedIds;

  const tauriReady = hasTauriRuntime();
  const operationContext = useMemo(
    () => ({
      environmentReady: Boolean(env?.ffmpeg_path && env?.whisper_path),
      hasApiCredential: settings.has_api_key,
    }),
    [env?.ffmpeg_path, env?.whisper_path, settings.has_api_key],
  );

  const taskCounts = useMemo(() => {
    let busyCount = 0;
    let doneCount = 0;
    let failedCount = 0;

    for (const task of tasks) {
      if (taskBusy(task)) busyCount += 1;
      if (task.status === "completed" || task.status === "exported") doneCount += 1;
      if (task.status === "failed") failedCount += 1;
    }

    return { busyCount, doneCount, failedCount };
  }, [tasks]);

  const allSelected = useMemo(
    () => tasks.length > 0 && tasks.every((task) => selectedIds.has(task.id)),
    [selectedIds, tasks],
  );

  const selectedRunnableOperations = useMemo(
    () => ({
      transcribe: tasks.some((task) => selectedIds.has(task.id) && canRunOperation(task, "transcribe", operationContext)),
      translate: tasks.some((task) => selectedIds.has(task.id) && canRunOperation(task, "translate", operationContext)),
      export: tasks.some((task) => selectedIds.has(task.id) && canRunOperation(task, "export", operationContext)),
    }),
    [operationContext, selectedIds, tasks],
  );

  const refreshTasks = useCallback(async () => {
    try {
      setTasks(await listTasks());
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const refreshSettings = useCallback(async () => {
    try {
      const loaded = await loadSettings();
      setSettings({ ...defaultSettings, ...loaded });
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const refreshEnvironment = useCallback(async () => {
    try {
      setEnv(await checkEnvironment());
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const refreshQueueSettings = useCallback(async () => {
    try {
      setQueueSettings(await loadQueueSettings());
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  useEffect(() => {
    if (!tauriReady) {
      setNotice(t("notice.requireTauriQueue"));
      return;
    }

    void refreshTasks();
    void refreshSettings();
    void refreshQueueSettings();
    const environmentTimer = window.setTimeout(() => {
      void refreshEnvironment();
    }, 350);

    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenDeleted: (() => void) | undefined;

    listen<TaskRecord>("task-updated", (event) => {
      setTasks((current) => upsertTask(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenTask = fn;
    }).catch((error) => {
      if (!disposed) setNotice(errorText(error));
    });

    listen<string>("task-deleted", (event) => {
      setTasks((current) => removeTask(current, event.payload));
      setSelectedIds((current) => removeSelectedId(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenDeleted = fn;
    }).catch((error) => {
      if (!disposed) setNotice(errorText(error));
    });

    return () => {
      disposed = true;
      window.clearTimeout(environmentTimer);
      unlistenTask?.();
      unlistenDeleted?.();
    };
  }, [refreshEnvironment, refreshQueueSettings, refreshSettings, refreshTasks, t, tauriReady]);

  const refreshTasksOnResume = useCallback(() => {
    void refreshTasks();
    void refreshSettings();
  }, [refreshSettings, refreshTasks]);

  useAppResume(refreshTasksOnResume, tauriReady);

  const saveQueueSettings = useCallback(async (nextSettings: QueueSettings) => {
    try {
      const saved = await saveQueueSettingsCommand(
        nextSettings.max_concurrency,
        nextSettings.auto_start_next,
      );
      setQueueSettings(saved);
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const saveConcurrency = useCallback(
    async (maxConcurrency: number) => {
      await saveQueueSettings({ ...queueSettings, max_concurrency: maxConcurrency });
    },
    [queueSettings, saveQueueSettings],
  );

  const saveAutoStartNext = useCallback(
    async (autoStartNext: boolean) => {
      await saveQueueSettings({ ...queueSettings, auto_start_next: autoStartNext });
    },
    [queueSettings, saveQueueSettings],
  );

  const pickOutputDir = useCallback(async () => {
    try {
      const picked = await selectOutputDir();
      if (picked) setOutputDir(picked);
    } catch (error) {
      setNotice(t("error.pickOutputDir", { error: errorText(error) }));
    }
  }, [t]);

  const createVideoTask = useCallback(async () => {
    try {
      const picked = await selectVideo();
      if (!picked) return;
      const created = await createVideoTaskCommand(
        taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked),
      );
      setTasks((current) => upsertTask(current, created));
      setNotice(t("notice.createdTask", { fileName: created.file_name }));
    } catch (error) {
      setNotice(t("error.createTask", { error: errorText(error) }));
    }
  }, [outputDir, settings, t]);

  const createAudioTask = useCallback(async () => {
    try {
      const picked = await selectAudio();
      if (!picked) return;
      const created = await createAudioTaskCommand(
        taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked, "audio"),
      );
      setTasks((current) => upsertTask(current, created));
      setNotice(t("notice.createdTask", { fileName: created.file_name }));
    } catch (error) {
      setNotice(t("error.createTask", { error: errorText(error) }));
    }
  }, [outputDir, settings, t]);

  const createSrtTask = useCallback(async () => {
    try {
      const picked = await selectSrt();
      if (!picked) return;
      const created = await createSrtTaskCommand(
        taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked, "srt"),
      );
      setTasks((current) => upsertTask(current, created));
      setNotice(t("notice.importedSrt", { fileName: created.file_name }));
    } catch (error) {
      setNotice(t("error.importSrt", { error: errorText(error) }));
    }
  }, [outputDir, settings, t]);

  const runOperation = useCallback(
    async (taskId: string, operation: TaskOperation) => {
      const task = tasksRef.current.find((current) => current.id === taskId);
      if (task && !canRunOperation(task, operation, operationContext)) {
        setNotice(operationRequirementSummary(operationRequirementIssues(task, operation, operationContext), t));
        return;
      }
      try {
        await runTaskOperation(taskId, operation);
        await refreshTasks();
        setNotice(t("notice.addedToQueue", { operation: operationLabel(operation, t) }));
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [operationContext, refreshTasks, t],
  );

  const runSelected = useCallback(
    async (operation: TaskOperation) => {
      const currentTasks = tasksRef.current;
      const currentSelectedIds = selectedIdsRef.current;
      const taskIds: string[] = [];
      for (const task of currentTasks) {
        if (currentSelectedIds.has(task.id) && canRunOperation(task, operation, operationContext)) taskIds.push(task.id);
      }
      if (taskIds.length === 0) {
        const issues = new Set<ReturnType<typeof operationRequirementIssues>[number]>();
        for (const task of currentTasks) {
          if (!currentSelectedIds.has(task.id)) continue;
          for (const issue of operationRequirementIssues(task, operation, operationContext)) issues.add(issue);
        }
        const requirements = operationRequirementSummary([...issues], t);
        setNotice(requirements ? t("notice.noRunnableSelectedWithRequirements", { requirements }) : t("notice.noRunnableSelected"));
        return;
      }
      try {
        await runTaskOperations(taskIds, operation);
        await refreshTasks();
        setNotice(
          t("notice.selectedAddedToQueue", {
            count: taskIds.length,
            operation: operationLabel(operation, t),
          }),
        );
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [operationContext, refreshTasks, t],
  );

  const cancelTask = useCallback(
    async (taskId: string) => {
      try {
        await cancelTaskCommand(taskId);
        await refreshTasks();
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [refreshTasks],
  );

  const cancelSelected = useCallback(async () => {
    const currentTasks = tasksRef.current;
    const currentSelectedIds = selectedIdsRef.current;
    const taskIds: string[] = [];
    for (const task of currentTasks) {
      if (currentSelectedIds.has(task.id) && taskBusy(task)) taskIds.push(task.id);
    }
    await Promise.all(taskIds.map((taskId) => cancelTask(taskId)));
  }, [cancelTask]);

  const deleteTask = useCallback(async (taskId: string) => {
    try {
      await deleteTaskCommand(taskId);
      setTasks((current) => removeTask(current, taskId));
      setSelectedIds((current) => removeSelectedId(current, taskId));
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const toggleTask = useCallback((taskId: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelectedIds((current) => {
      const currentTasks = tasksRef.current;
      if (currentTasks.length > 0 && currentTasks.every((task) => current.has(task.id))) return new Set();
      return new Set(currentTasks.map((task) => task.id));
    });
  }, []);

  return {
    allSelected,
    busyCount: taskCounts.busyCount,
    cancelSelected,
    cancelTask,
    createAudioTask,
    createSrtTask,
    createVideoTask,
    deleteTask,
    doneCount: taskCounts.doneCount,
    failedCount: taskCounts.failedCount,
    notice,
    operationContext,
    outputDir,
    pickOutputDir,
    queueSettings,
    refreshTasks,
    runOperation,
    runSelected,
    saveAutoStartNext,
    saveConcurrency,
    selectedIds,
    selectedRunnableOperations,
    tasks,
    toggleAll,
    toggleTask,
  };
}
