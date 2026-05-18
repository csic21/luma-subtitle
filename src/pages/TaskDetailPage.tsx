import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft } from "lucide-react";
import { useNavigate, useParams } from "react-router-dom";

import { NoticeAlert, StatusBadge } from "@/components/app/shared";
import { TaskConfigCard } from "@/components/app/task-detail-config";
import {
  SubtitlePreviewCard,
  TaskFlowStrip,
  TaskLogsCard,
  TaskProgressCard,
  TaskSummaryCard,
} from "@/components/app/task-detail";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useI18n } from "@/i18n";
import { errorText, hasTauriRuntime, isTranslateStage, operationLabel, taskBusy } from "@/lib/app-utils";
import { appendRealtimeLog, normalizeTaskSettings, taskSettingsUpdatePayload } from "@/lib/task-data";
import type { JobEvent, SubtitlePreview, TaskRecord, TaskSettingsSnapshot, TaskOperation } from "@/types";

export function TaskDetailPage() {
  const { taskId = "" } = useParams();
  const navigate = useNavigate();
  const { locale, t } = useI18n();
  const [task, setTask] = useState<TaskRecord | null>(null);
  const [settingsDraft, setSettingsDraft] = useState<TaskSettingsSnapshot | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [subtitlePreview, setSubtitlePreview] = useState<SubtitlePreview | null>(null);
  const [subtitleView, setSubtitleView] = useState<"translated" | "source">("source");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!taskId) return;
    if (!hasTauriRuntime()) {
      setNotice(t("notice.requireTauriDetails"));
      return;
    }
    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenJob: (() => void) | undefined;
    const refreshOnResume = () => {
      if (!disposed) void refreshTask({ preview: false });
    };
    const refreshOnVisible = () => {
      if (document.visibilityState === "visible") refreshOnResume();
    };
    window.addEventListener("focus", refreshOnResume);
    document.addEventListener("visibilitychange", refreshOnVisible);

    const taskListener = listen<TaskRecord>("task-updated", (event) => {
      if (event.payload.id !== taskId) return;
      setTask(event.payload);
      setSettingsDraft(normalizeTaskSettings(event.payload.settings));
      void refreshLogs();
      if (event.payload.source_srt_path) void refreshPreview(event.payload);
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenTask = fn;
    });
    const jobListener = listen<JobEvent>("job-event", (event) => {
      if (event.payload.job_id !== taskId) return;
      setLogs((current) => appendRealtimeLog(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenJob = fn;
    });
    void Promise.all([taskListener, jobListener]).then(() => {
      if (!disposed) void refreshTask();
    });
    return () => {
      disposed = true;
      window.removeEventListener("focus", refreshOnResume);
      document.removeEventListener("visibilitychange", refreshOnVisible);
      unlistenTask?.();
      unlistenJob?.();
    };
  }, [taskId, t]);

  async function refreshTask(options: { preview?: boolean } = {}) {
    try {
      const loaded = await invoke<TaskRecord>("get_task", { taskId });
      setTask(loaded);
      setSettingsDraft(normalizeTaskSettings(loaded.settings));
      await refreshLogs();
      if ((options.preview ?? true) && loaded.source_srt_path) await refreshPreview(loaded);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshLogs() {
    try {
      setLogs(await invoke<string[]>("get_task_logs", { taskId }));
    } catch {
      setLogs([]);
    }
  }

  async function refreshPreview(currentTask = task) {
    if (!currentTask?.source_srt_path) return;
    try {
      const preview = await invoke<SubtitlePreview>("subtitle_preview", { jobId: taskId });
      setSubtitlePreview(preview);
      setSubtitleView(preview.translated_srt?.trim() ? "translated" : "source");
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function runOperation(operation: "transcribe" | "translate" | "export") {
    try {
      await invoke("run_task_operation", { taskId, operation });
      await refreshTask();
      setNotice(t("notice.addedToQueue", { operation: operationLabel(operation, t) }));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function cancelTask() {
    try {
      await invoke("cancel_task", { taskId });
      await refreshTask();
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function applyCurrentSettingsToTask() {
    try {
      const updated = await invoke<TaskRecord>("apply_current_settings_to_task", { taskId });
      setTask(updated);
      setSettingsDraft(normalizeTaskSettings(updated.settings));
      await refreshLogs();
      setNotice(t("notice.settingsApplied"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function saveTaskSettings() {
    if (!settingsDraft) return;
    try {
      const updated = await invoke<TaskRecord>("update_task_settings", {
        taskId,
        settings: taskSettingsUpdatePayload(settingsDraft),
      });
      setTask(updated);
      setSettingsDraft(normalizeTaskSettings(updated.settings));
      await refreshLogs();
      setNotice(t("notice.taskSettingsSaved"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function pickTaskWhisperModel() {
    try {
      const picked = await invoke<string | null>("select_whisper_model");
      if (picked) setSettingsDraft((current) => (current ? { ...current, whisper_model_path: picked } : current));
    } catch (error) {
      setNotice(t("error.pickWhisperModel", { error: errorText(error) }));
    }
  }

  async function openOutputDir() {
    const target = task?.exported_output_dir || task?.output_dir || task?.settings.output_dir;
    if (!target) return;
    try {
      await invoke("open_path", { path: target });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  const flowSteps = useMemo(() => {
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
  const taskSettingsDirty = Boolean(
    task &&
      settingsDraft &&
      JSON.stringify(normalizeTaskSettings(task.settings)) !== JSON.stringify(normalizeTaskSettings(settingsDraft)),
  );

  if (!task) {
    return (
      <>
        <NoticeAlert message={notice} />
        <Card className="loading-panel">
          <CardContent>{t("task.loading")}</CardContent>
        </Card>
      </>
    );
  }
  const taskConfig = settingsDraft ?? normalizeTaskSettings(task.settings);

  return (
    <>
      <section className="page-heading">
        <Button variant="secondary" onClick={() => navigate("/tasks")}>
          <ArrowLeft data-icon="inline-start" />
          {t("common.backToQueue")}
        </Button>
        <div>
          <h1>{task.file_name}</h1>
          <p>{task.message}</p>
        </div>
        <StatusBadge status={task.status} />
      </section>

      <NoticeAlert message={notice} />

      <TaskFlowStrip flowSteps={flowSteps} progressLabel={t("common.progress")} />

      <section className="workspace">
        <div className="left-column">
          <TaskSummaryCard
            locale={locale}
            task={task}
            t={t}
            onCancelTask={cancelTask}
            onRunOperation={runOperation}
          />
          <TaskConfigCard
            task={task}
            taskConfig={taskConfig}
            taskSettingsDirty={taskSettingsDirty}
            t={t}
            onApplyCurrentSettings={applyCurrentSettingsToTask}
            onPickWhisperModel={pickTaskWhisperModel}
            onSaveTaskSettings={saveTaskSettings}
            setSettingsDraft={setSettingsDraft}
          />
        </div>

        <div className="right-column">
          <TaskProgressCard task={task} t={t} onOpenOutputDir={openOutputDir} onRunOperation={runOperation} />
          <SubtitlePreviewCard
            activeSubtitleBody={activeSubtitleBody}
            activeSubtitleFileName={activeSubtitleFileName}
            hasTranslatedSubtitle={hasTranslatedSubtitle}
            subtitlePreview={subtitlePreview}
            subtitleView={subtitleView}
            t={t}
            onRefreshPreview={() => refreshPreview()}
            setSubtitleView={setSubtitleView}
          />
          <TaskLogsCard logs={logs} t={t} />
        </div>
      </section>
    </>
  );
}
