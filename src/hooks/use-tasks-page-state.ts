import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { defaultSettings } from "@/config";
import { canRunOperation, errorText, hasTauriRuntime, operationLabel, taskBusy } from "@/lib/app-utils";
import {
  cancelTask as cancelTaskCommand,
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
  selectSrt,
  selectVideo,
} from "@/lib/tauri-api";
import { taskCreatePayload, upsertTask } from "@/lib/task-data";
import type { QueueSettings, SettingsState, TaskOperation, TaskRecord, TFunction } from "@/types";

import { useAppResume } from "./use-app-resume";

const defaultQueueSettings: QueueSettings = {
  max_concurrency: 2,
  auto_start_next: false,
};

export function useTasksPageState(t: TFunction) {
  const [tasks, setTasks] = useState<TaskRecord[]>([]);
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [queueSettings, setQueueSettings] = useState<QueueSettings>(defaultQueueSettings);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [outputDir, setOutputDir] = useState("");
  const [notice, setNotice] = useState("");

  const tauriReady = hasTauriRuntime();

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
      setTasks((current) => current.filter((task) => task.id !== event.payload));
      setSelectedIds((current) => {
        const next = new Set(current);
        next.delete(event.payload);
        return next;
      });
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
      unlistenTask?.();
      unlistenDeleted?.();
    };
  }, [refreshQueueSettings, refreshSettings, refreshTasks, t, tauriReady]);

  const refreshTasksOnResume = useCallback(() => {
    void refreshTasks();
  }, [refreshTasks]);

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
      try {
        await runTaskOperation(taskId, operation);
        await refreshTasks();
        setNotice(t("notice.addedToQueue", { operation: operationLabel(operation, t) }));
      } catch (error) {
        setNotice(errorText(error));
      }
    },
    [refreshTasks, t],
  );

  const runSelected = useCallback(
    async (operation: TaskOperation) => {
      const taskIds: string[] = [];
      for (const task of tasks) {
        if (selectedIds.has(task.id) && canRunOperation(task, operation)) taskIds.push(task.id);
      }
      if (taskIds.length === 0) {
        setNotice(t("notice.noRunnableSelected"));
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
    [refreshTasks, selectedIds, t, tasks],
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
    const taskIds: string[] = [];
    for (const task of tasks) {
      if (selectedIds.has(task.id) && taskBusy(task)) taskIds.push(task.id);
    }
    await Promise.all(taskIds.map((taskId) => cancelTask(taskId)));
  }, [cancelTask, selectedIds, tasks]);

  const deleteTask = useCallback(async (taskId: string) => {
    try {
      await deleteTaskCommand(taskId);
      setTasks((current) => current.filter((task) => task.id !== taskId));
      setSelectedIds((current) => {
        const next = new Set(current);
        next.delete(taskId);
        return next;
      });
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
      if (tasks.length > 0 && tasks.every((task) => current.has(task.id))) return new Set();
      return new Set(tasks.map((task) => task.id));
    });
  }, [tasks]);

  return {
    allSelected,
    busyCount: taskCounts.busyCount,
    cancelSelected,
    cancelTask,
    createSrtTask,
    createVideoTask,
    deleteTask,
    doneCount: taskCounts.doneCount,
    failedCount: taskCounts.failedCount,
    notice,
    outputDir,
    pickOutputDir,
    queueSettings,
    refreshTasks,
    runOperation,
    runSelected,
    saveAutoStartNext,
    saveConcurrency,
    selectedIds,
    tasks,
    toggleAll,
    toggleTask,
  };
}
