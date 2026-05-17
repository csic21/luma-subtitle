import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, ArrowLeft, Check, CheckCircle2, ChevronRight, CircleStop, Download, ExternalLink, FileVideo, FolderOpen, Languages, Loader2, Play, RefreshCw, Save, Settings, Subtitles, Terminal } from "lucide-react";
import { useNavigate, useParams } from "react-router-dom";

import { FieldBlock, IconAction, NoticeAlert, SectionTitle, StatusBadge } from "@/components/app/shared";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { defaultSettings, languageOptions, whisperLanguageOptions } from "@/config";
import { useI18n } from "@/i18n";
import { canRunOperation, errorText, formattedTime, hasTauriRuntime, isTranslateStage, operationLabel, progressLabel, progressValue, stageText, taskBusy } from "@/lib/app-utils";
import { appendRealtimeLog, applyJobEventToTask, normalizeTaskSettings, taskSettingsUpdatePayload } from "@/lib/task-data";
import { cn } from "@/lib/utils";
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
    void refreshTask();
    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenJob: (() => void) | undefined;
    listen<TaskRecord>("task-updated", (event) => {
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
    listen<JobEvent>("job-event", (event) => {
      if (event.payload.job_id !== taskId) return;
      setTask((current) => (current ? applyJobEventToTask(current, event.payload) : current));
      setLogs((current) => appendRealtimeLog(current, event.payload));
      if (event.payload.status !== "running" || event.payload.outputs) {
        void refreshTask();
      }
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenJob = fn;
    });
    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenJob?.();
    };
  }, [taskId, t]);

  useEffect(() => {
    if (!hasTauriRuntime() || !taskId || !task || !taskBusy(task)) return;
    const timer = window.setInterval(() => {
      void refreshTask({ preview: false });
    }, 1500);
    return () => window.clearInterval(timer);
  }, [taskId, task?.status]);

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

      <section className="flow-strip" aria-label={t("common.progress")}>
        {flowSteps.map((step, index) => (
          <div className={cn("flow-step", step.state)} key={step.label}>
            <span className="flow-index">{step.state === "done" ? <Check /> : index + 1}</span>
            <div>
              <strong>{step.label}</strong>
              <span>{step.detail}</span>
            </div>
            {index < flowSteps.length - 1 && <ChevronRight className="flow-arrow" />}
          </div>
        ))}
      </section>

      <section className="workspace">
        <div className="left-column">
          <Card>
            <CardHeader>
              <SectionTitle icon={<FileVideo />} title={t("tabs.task")} />
            </CardHeader>
            <CardContent className="stack-panel">
              <div className="detail-list">
                <span>{t("flow.material")}</span>
                <code>{task.video_path || task.srt_path || task.file_name}</code>
                <span>{t("task.outputDir")}</span>
                <code>{task.output_dir || task.settings.output_dir || t("task.sameAsSourceDir")}</code>
                <span>{t("common.updatedAt")}</span>
                <code>{formattedTime(task.updated_at, locale)}</code>
                <span>{t("task.segmentCount")}</span>
                <code>{task.segment_count ?? "—"}</code>
              </div>
              {task.error && (
                <Alert variant="destructive">
                  <AlertCircle />
                  <AlertTitle>{t("task.taskError")}</AlertTitle>
                  <AlertDescription>{task.error}</AlertDescription>
                </Alert>
              )}
              <div className="action-row">
                <Button
                  variant="secondary"
                  onClick={() => runOperation("transcribe")}
                  disabled={!canRunOperation(task, "transcribe")}
                >
                  <Play data-icon="inline-start" />
                  {t("common.transcribe")}
                </Button>
                <Button
                  variant="secondary"
                  onClick={() => runOperation("translate")}
                  disabled={!canRunOperation(task, "translate")}
                >
                  <Languages data-icon="inline-start" />
                  {t("common.translate")}
                </Button>
                <Button onClick={() => runOperation("export")} disabled={!canRunOperation(task, "export")}>
                  <Download data-icon="inline-start" />
                  {t("common.export")}
                </Button>
                <Button variant="destructive" onClick={cancelTask} disabled={!taskBusy(task)}>
                  <CircleStop data-icon="inline-start" />
                  {t("common.cancel")}
                </Button>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <SectionTitle icon={<Settings />} title={t("tabs.taskConfig")} />
              <CardAction>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={applyCurrentSettingsToTask}
                  disabled={taskBusy(task)}
                  title={t("settings.applyGlobalTitle")}
                >
                  <RefreshCw data-icon="inline-start" />
                  {t("settings.applyGlobal")}
                </Button>
              </CardAction>
            </CardHeader>
            <CardContent className="settings-form">
              <FieldBlock label={t("common.whisperModel")}>
                <div className="input-action">
                  <Input
                    value={taskConfig.whisper_model_path}
                    onChange={(event) =>
                      setSettingsDraft({ ...taskConfig, whisper_model_path: event.target.value })
                    }
                    disabled={taskBusy(task)}
                    placeholder={t("settings.notSet")}
                    title={taskConfig.whisper_model_path || t("settings.selectWhisper")}
                  />
                  <IconAction label={t("settings.selectWhisper")} onClick={pickTaskWhisperModel} disabled={taskBusy(task)}>
                    <FolderOpen />
                  </IconAction>
                </div>
              </FieldBlock>

              <div className="grid-two">
                <FieldBlock label={t("settings.sourceLanguage")}>
                  <Select
                    value={taskConfig.whisper_language || "auto"}
                    onValueChange={(value) => setSettingsDraft({ ...taskConfig, whisper_language: value })}
                    disabled={taskBusy(task)}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {whisperLanguageOptions.map((language) => (
                          <SelectItem key={language.value} value={language.value}>
                            {t(language.labelKey)}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </FieldBlock>
                <FieldBlock label={t("common.targetLanguage")}>
                  <Select
                    value={taskConfig.target_language}
                    onValueChange={(value) => setSettingsDraft({ ...taskConfig, target_language: value })}
                    disabled={taskBusy(task)}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {languageOptions.map((language) => (
                          <SelectItem key={language.value} value={language.value}>
                            {t(language.labelKey)}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </FieldBlock>
              </div>

              <div className="grid-two">
                <FieldBlock label="Base URL">
                  <Input
                    value={taskConfig.base_url}
                    onChange={(event) => setSettingsDraft({ ...taskConfig, base_url: event.target.value })}
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
                <FieldBlock label={t("settings.translationModel")}>
                  <Input
                    value={taskConfig.model}
                    onChange={(event) => setSettingsDraft({ ...taskConfig, model: event.target.value })}
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
              </div>

              <div className="grid-two">
                <FieldBlock label="Temperature">
                  <Input
                    type="number"
                    min="0"
                    max="1"
                    step="0.1"
                    value={taskConfig.temperature}
                    onChange={(event) =>
                      setSettingsDraft({ ...taskConfig, temperature: Number.parseFloat(event.target.value) || 0 })
                    }
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
                <FieldBlock label={t("settings.shardSize")}>
                  <Input
                    type="number"
                    min="1"
                    max="1000"
                    step="1"
                    value={taskConfig.translation_shard_size ?? defaultSettings.translation_shard_size}
                    onChange={(event) =>
                      setSettingsDraft({
                        ...taskConfig,
                        translation_shard_size:
                          Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
                      })
                    }
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
              </div>

              <div className="action-row end">
                <Button
                  variant="secondary"
                  onClick={() => setSettingsDraft(normalizeTaskSettings(task.settings))}
                  disabled={taskBusy(task) || !taskSettingsDirty}
                >
                  {t("settings.undo")}
                </Button>
                <Button onClick={saveTaskSettings} disabled={taskBusy(task) || !taskSettingsDirty}>
                  <Save data-icon="inline-start" />
                  {t("settings.saveTask")}
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>

        <div className="right-column">
          <Card className={cn("progress-panel", `progress-${task.status}`)}>
            <CardHeader>
              <SectionTitle
                icon={
                  task.status === "completed" || task.status === "exported" ? (
                    <CheckCircle2 />
                  ) : taskBusy(task) ? (
                    <Loader2 className="spin" />
                  ) : (
                    <Terminal />
                  )
                }
                title={t("common.progress")}
              />
              <CardAction>
                <StatusBadge status={task.status} label={stageText(task.stage, t)} />
              </CardAction>
            </CardHeader>
            <CardContent className="stack-panel">
              <div className="progress-head">
                <span>{task.message}</span>
                <strong>{progressLabel(task.progress)}</strong>
              </div>
              <Progress className="hotdog-progress large" value={progressValue(task.progress)} />
              {(task.source_file_name || task.translated_file_name) && (
                <div className="outputs">
                  <Button onClick={() => runOperation("export")} disabled={!canRunOperation(task, "export")}>
                    <Download data-icon="inline-start" />
                    {t("common.exportSubtitles")}
                  </Button>
                  {task.source_file_name && <code>{task.source_file_name}</code>}
                  {task.translated_file_name && <code>{task.translated_file_name}</code>}
                  {task.exported_output_dir && (
                    <Button variant="secondary" onClick={openOutputDir} title={t("common.openExportDir")}>
                      <ExternalLink data-icon="inline-start" />
                      {t("common.openExportDir")}
                    </Button>
                  )}
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <SectionTitle icon={<Subtitles />} title={t("subtitle.preview")} />
              <CardAction>
                <Button variant="secondary" size="sm" onClick={() => refreshPreview()}>
                  <RefreshCw data-icon="inline-start" />
                  {t("common.refresh")}
                </Button>
              </CardAction>
            </CardHeader>
            <CardContent>
              <Tabs
                value={subtitleView}
                onValueChange={(value) => setSubtitleView(value as "translated" | "source")}
                className="subtitle-tabs"
              >
                <TabsList>
                  <TabsTrigger value="source">{t("common.source")}</TabsTrigger>
                  {hasTranslatedSubtitle && <TabsTrigger value="translated">{t("common.translated")}</TabsTrigger>}
                </TabsList>
              </Tabs>

              {subtitlePreview ? (
                <>
                  <code className="subtitle-file">{activeSubtitleFileName}</code>
                  <ScrollArea className="subtitle-preview">
                    <pre>{activeSubtitleBody || t("subtitle.noContent")}</pre>
                  </ScrollArea>
                </>
              ) : (
                <div className="subtitle-empty">{t("subtitle.empty")}</div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <SectionTitle icon={<Terminal />} title={t("tabs.logs")} />
            </CardHeader>
            <CardContent>
              <ScrollArea className="log-list">
                {logs.length === 0 ? (
                  <span className="muted">{t("task.logsEmpty")}</span>
                ) : (
                  logs.map((line, index) => <p key={`${line}-${index}`}>{line}</p>)
                )}
              </ScrollArea>
            </CardContent>
          </Card>
        </div>
      </section>
    </>
  );
}


