import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "react-router-dom";

import { NoticeAlert } from "@/components/app/shared";
import { TaskMetricGrid, TaskQueueTable, TaskToolbar } from "@/components/app/tasks";
import { defaultSettings } from "@/config";
import { useI18n } from "@/i18n";
import { canRunOperation, errorText, hasTauriRuntime, operationLabel, taskBusy } from "@/lib/app-utils";
import { applyJobEventToTasks, taskCreatePayload, upsertTask } from "@/lib/task-data";
import type { JobEvent, QueueSettings, SettingsState, TaskRecord, TaskOperation } from "@/types";

export function TasksPage() {
  const navigate = useNavigate();
  const { locale, t } = useI18n();
  const [tasks, setTasks] = useState<TaskRecord[]>([]);
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [queueSettings, setQueueSettings] = useState<QueueSettings>({ max_concurrency: 2 });
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [outputDir, setOutputDir] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!hasTauriRuntime()) {
      setNotice(t("notice.requireTauriQueue"));
      return;
    }
    void refreshTasks();
    void refreshSettings();
    void refreshQueueSettings();

    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenJob: (() => void) | undefined;
    let unlistenDeleted: (() => void) | undefined;
    listen<TaskRecord>("task-updated", (event) => {
      setTasks((current) => upsertTask(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenTask = fn;
    });
    listen<JobEvent>("job-event", (event) => {
      setTasks((current) => applyJobEventToTasks(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenJob = fn;
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
    });
    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenJob?.();
      unlistenDeleted?.();
    };
  }, [t]);

  useEffect(() => {
    if (!hasTauriRuntime() || !tasks.some(taskBusy)) return;
    const timer = window.setInterval(() => {
      void refreshTasks();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [tasks]);

  async function refreshTasks() {
    try {
      setTasks(await invoke<TaskRecord[]>("list_tasks"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshSettings() {
    try {
      const loaded = await invoke<SettingsState>("load_settings");
      setSettings({ ...defaultSettings, ...loaded });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshQueueSettings() {
    try {
      setQueueSettings(await invoke<QueueSettings>("load_queue_settings"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function saveConcurrency(maxConcurrency: number) {
    try {
      const saved = await invoke<QueueSettings>("save_queue_settings", { maxConcurrency });
      setQueueSettings(saved);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function pickOutputDir() {
    try {
      const picked = await invoke<string | null>("select_output_dir");
      if (picked) setOutputDir(picked);
    } catch (error) {
      setNotice(t("error.pickOutputDir", { error: errorText(error) }));
    }
  }

  async function createVideoTask() {
    try {
      const picked = await invoke<string | null>("select_video");
      if (!picked) return;
      const created = await invoke<TaskRecord>("create_video_task", {
        request: taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked),
      });
      setTasks((current) => upsertTask(current, created));
      setNotice(t("notice.createdTask", { fileName: created.file_name }));
    } catch (error) {
      setNotice(t("error.createTask", { error: errorText(error) }));
    }
  }

  async function createSrtTask() {
    try {
      const picked = await invoke<string | null>("select_srt");
      if (!picked) return;
      const created = await invoke<TaskRecord>("create_srt_task", {
        request: taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked, "srt"),
      });
      setTasks((current) => upsertTask(current, created));
      setNotice(t("notice.importedSrt", { fileName: created.file_name }));
    } catch (error) {
      setNotice(t("error.importSrt", { error: errorText(error) }));
    }
  }

  async function runOperation(taskId: string, operation: "transcribe" | "translate" | "export") {
    try {
      await invoke("run_task_operation", { taskId, operation });
      await refreshTasks();
      setNotice(t("notice.addedToQueue", { operation: operationLabel(operation, t) }));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function runSelected(operation: "transcribe" | "translate" | "export") {
    const taskIds = tasks
      .filter((task) => selectedIds.has(task.id) && canRunOperation(task, operation))
      .map((task) => task.id);
    if (taskIds.length === 0) {
      setNotice(t("notice.noRunnableSelected"));
      return;
    }
    try {
      await invoke("run_task_operations", { taskIds, operation });
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
  }

  async function cancelTask(taskId: string) {
    try {
      await invoke("cancel_task", { taskId });
      await refreshTasks();
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function cancelSelected() {
    const taskIds = tasks.filter((task) => selectedIds.has(task.id) && taskBusy(task)).map((task) => task.id);
    await Promise.all(taskIds.map((taskId) => cancelTask(taskId)));
  }

  async function deleteTask(taskId: string) {
    try {
      await invoke("delete_task", { taskId });
      setTasks((current) => current.filter((task) => task.id !== taskId));
      setSelectedIds((current) => {
        const next = new Set(current);
        next.delete(taskId);
        return next;
      });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  const allSelected = tasks.length > 0 && tasks.every((task) => selectedIds.has(task.id));
  const busyCount = tasks.filter(taskBusy).length;
  const doneCount = tasks.filter((task) => task.status === "completed" || task.status === "exported").length;
  const failedCount = tasks.filter((task) => task.status === "failed").length;

  function toggleTask(taskId: string) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  }

  function toggleAll() {
    setSelectedIds((current) => {
      if (tasks.length > 0 && tasks.every((task) => current.has(task.id))) return new Set();
      return new Set(tasks.map((task) => task.id));
    });
  }

  return (
    <>
      <NoticeAlert message={notice} />

      <TaskMetricGrid
        busyCount={busyCount}
        doneCount={doneCount}
        failedCount={failedCount}
        taskCount={tasks.length}
        t={t}
      />

      <TaskToolbar
        outputDir={outputDir}
        queueSettings={queueSettings}
        t={t}
        onCancelSelected={cancelSelected}
        onCreateSrtTask={createSrtTask}
        onCreateVideoTask={createVideoTask}
        onPickOutputDir={pickOutputDir}
        onRefreshTasks={refreshTasks}
        onRunSelected={runSelected}
        onSaveConcurrency={saveConcurrency}
      />

      <TaskQueueTable
        allSelected={allSelected}
        locale={locale}
        selectedIds={selectedIds}
        tasks={tasks}
        t={t}
        onCancelTask={cancelTask}
        onDeleteTask={deleteTask}
        onOpenTask={(taskId) => navigate(`/tasks/${taskId}`)}
        onRunOperation={runOperation}
        onToggleAll={toggleAll}
        onToggleTask={toggleTask}
      />
    </>
  );
}

