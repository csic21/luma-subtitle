import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CircleStop, Database, Download, Eye, FileVideo, FolderOpen, Languages, Play, Plus, RefreshCw, Trash2, Upload } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { NoticeAlert, IconAction, SectionTitle, StatusBadge } from "@/components/app/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { defaultSettings } from "@/config";
import { useI18n } from "@/i18n";
import { canRunOperation, errorText, fileName, formattedTime, hasTauriRuntime, operationLabel, progressValue, stageText, taskBusy } from "@/lib/app-utils";
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

      <section className="metric-grid">
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>{t("metric.all")}</CardDescription>
            <CardTitle>{tasks.length}</CardTitle>
          </CardHeader>
        </Card>
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>{t("metric.running")}</CardDescription>
            <CardTitle>{busyCount}</CardTitle>
          </CardHeader>
        </Card>
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>{t("metric.done")}</CardDescription>
            <CardTitle>{doneCount}</CardTitle>
          </CardHeader>
        </Card>
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>{t("metric.failed")}</CardDescription>
            <CardTitle>{failedCount}</CardTitle>
          </CardHeader>
        </Card>
      </section>

      <Card className="toolbar-card">
        <CardContent className="toolbar-content">
          <div className="toolbar-main">
            <Button onClick={createVideoTask} title={t("task.videoNewTitle")}>
              <Plus data-icon="inline-start" />
              {t("task.videoNew")}
            </Button>
            <Button variant="secondary" onClick={createSrtTask} title={t("task.srtImportTitle")}>
              <Upload data-icon="inline-start" />
              {t("task.srtImport")}
            </Button>
            <Button variant="secondary" onClick={pickOutputDir} title={t("task.pickOutputDir")}>
              <FolderOpen data-icon="inline-start" />
              {t("task.inputDir")}
            </Button>
            <code className="path-chip">{outputDir || t("task.defaultOutput")}</code>
          </div>

          <Separator className="toolbar-separator" />

          <div className="toolbar-actions">
            <div className="concurrency-field">
              <Label htmlFor="max-concurrency">{t("task.concurrency")}</Label>
              <Input
                id="max-concurrency"
                type="number"
                min="1"
                max="4"
                value={queueSettings.max_concurrency}
                onChange={(event) => void saveConcurrency(Number.parseInt(event.target.value, 10) || 1)}
              />
            </div>
            <Button variant="secondary" onClick={() => runSelected("transcribe")}>
              <Play data-icon="inline-start" />
              {t("common.transcribe")}
            </Button>
            <Button variant="secondary" onClick={() => runSelected("translate")}>
              <Languages data-icon="inline-start" />
              {t("common.translate")}
            </Button>
            <Button variant="secondary" onClick={() => runSelected("export")}>
              <Download data-icon="inline-start" />
              {t("common.export")}
            </Button>
            <Button variant="destructive" onClick={cancelSelected}>
              <CircleStop data-icon="inline-start" />
              {t("common.cancel")}
            </Button>
            <IconAction label={t("common.refresh")} onClick={refreshTasks}>
              <RefreshCw />
            </IconAction>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <SectionTitle icon={<Database />} title={t("task.queue")} description={t("task.queueDescription")} />
          <CardAction>
            <Badge variant="secondary">{t("task.records", { count: tasks.length })}</Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="table-card-content">
          <Table className="task-table">
            <TableHeader>
              <TableRow>
                <TableHead className="select-cell">
                  <Checkbox checked={allSelected} onCheckedChange={toggleAll} aria-label={t("task.ariaSelectAll")} />
                </TableHead>
                <TableHead>{t("common.file")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("common.progress")}</TableHead>
                <TableHead>{t("common.targetLanguage")}</TableHead>
                <TableHead>{t("task.outputDir")}</TableHead>
                <TableHead>{t("common.updatedAt")}</TableHead>
                <TableHead>{t("task.tableAction")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tasks.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={8}>
                    <div className="empty-state">{t("task.empty")}</div>
                  </TableCell>
                </TableRow>
              ) : (
                tasks.map((task) => (
                  <TableRow key={task.id} data-state={taskBusy(task) ? "selected" : undefined}>
                    <TableCell className="select-cell">
                      <Checkbox
                        checked={selectedIds.has(task.id)}
                        onCheckedChange={() => toggleTask(task.id)}
                        aria-label={t("task.ariaSelect", { fileName: task.file_name })}
                      />
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        className="file-button"
                        onClick={() => navigate(`/tasks/${task.id}`)}
                      >
                        <FileVideo data-icon="inline-start" />
                        <span>{task.file_name}</span>
                      </Button>
                      <small>{task.source_type === "srt" ? "SRT" : fileName(task.video_path)}</small>
                    </TableCell>
                    <TableCell>
                      <StatusBadge status={task.status} />
                      <small>{stageText(task.stage, t)}</small>
                    </TableCell>
                    <TableCell>
                      <div className="table-progress-stack">
                        <Progress className="hotdog-progress" value={progressValue(task.progress)} />
                        <small>{task.message}</small>
                      </div>
                    </TableCell>
                    <TableCell>{task.settings.target_language}</TableCell>
                    <TableCell>
                      <code className="table-code">
                        {task.output_dir || task.settings.output_dir || t("task.sameAsSourceDir")}
                      </code>
                    </TableCell>
                    <TableCell>{formattedTime(task.updated_at, locale)}</TableCell>
                    <TableCell>
                      <div className="row-actions">
                        <IconAction
                          label={t("common.transcribe")}
                          onClick={() => runOperation(task.id, "transcribe")}
                          disabled={!canRunOperation(task, "transcribe")}
                        >
                          <Play />
                        </IconAction>
                        <IconAction
                          label={t("common.translate")}
                          onClick={() => runOperation(task.id, "translate")}
                          disabled={!canRunOperation(task, "translate")}
                        >
                          <Languages />
                        </IconAction>
                        <IconAction
                          label={t("common.export")}
                          onClick={() => runOperation(task.id, "export")}
                          disabled={!canRunOperation(task, "export")}
                        >
                          <Download />
                        </IconAction>
                        <IconAction label={t("common.cancel")} onClick={() => cancelTask(task.id)} disabled={!taskBusy(task)}>
                          <CircleStop />
                        </IconAction>
                        <IconAction label={t("common.details")} onClick={() => navigate(`/tasks/${task.id}`)}>
                          <Eye />
                        </IconAction>
                        <IconAction
                          label={t("common.delete")}
                          onClick={() => deleteTask(task.id)}
                          disabled={taskBusy(task)}
                          className="danger-action"
                        >
                          <Trash2 />
                        </IconAction>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </>
  );
}


