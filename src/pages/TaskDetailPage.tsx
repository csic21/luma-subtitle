import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ChevronDown, Download, ExternalLink, PanelRight, Terminal, X } from "lucide-react";
import { useNavigate, useParams } from "react-router-dom";
import { NoticeAlert, StatusBadge } from "@/components/app/shared";
import { TaskConfigCard } from "@/components/app/task-detail-config";
import { SubtitlePreviewCard, TaskFlowStrip, TaskLogsCard, TaskOperationActions } from "@/components/app/task-detail";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { useTaskDetailState } from "@/hooks/use-task-detail-state";
import { useI18n } from "@/i18n";
import { canRunOperation, operationRequirementIssues, operationRequirementSummary, progressValue, taskBusy, taskSourcePath } from "@/lib/app-utils";

export function TaskDetailPage() {
  const { taskId = "" } = useParams();
  const navigate = useNavigate();
  const { t } = useI18n();
  const state = useTaskDetailState(taskId, t);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [logsOpen, setLogsOpen] = useState(false);
  const inspectorToggle = useRef<HTMLButtonElement>(null);
  const inspectorClose = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (inspectorOpen) inspectorClose.current?.focus(); }, [inspectorOpen]);
  const closeInspector = () => { setInspectorOpen(false); inspectorToggle.current?.focus(); };
  const { task, taskConfig, operationTask, commandPending, taskSettingsDirty, operationContext, notice } = state;
  if (!task || !taskConfig || !operationTask) return <section className="workbench-loading" role="status"><NoticeAlert message={notice} />{t("task.loading")}</section>;
  const exportIssues = operationRequirementIssues(operationTask, "export", operationContext);
  const visibleNotice = [task.error, notice].filter((message, index, all) => message && all.indexOf(message) === index).join("\n");
  return <section className="task-workbench">
    <header className="workbench-header">
      <Button variant="ghost" size="icon-sm" onClick={() => navigate("/tasks")} aria-label={t("common.backToQueue")} title={t("common.backToQueue")}><ArrowLeft /></Button>
      <div className="workbench-title"><span>{t("workbench.title")}</span><h1 title={task.file_name}>{task.file_name}</h1></div>
      <StatusBadge status={task.status} />
      <div className="workbench-header-actions">
        <TaskOperationActions task={operationTask} commandPending={commandPending} taskSettingsDirty={taskSettingsDirty} operationContext={operationContext} t={t} onRunOperation={state.runOperation} onCancelTask={state.cancelTask} />
        <Button ref={inspectorToggle} variant="ghost" size="sm" aria-expanded={inspectorOpen} aria-controls="task-inspector" onClick={() => setInspectorOpen(!inspectorOpen)}><PanelRight data-icon="inline-start" />{t("workbench.configure")}</Button>
        <Button size="sm" disabled={commandPending || !canRunOperation(operationTask, "export", operationContext)} title={exportIssues.length ? operationRequirementSummary(exportIssues, t) : t("common.exportSubtitles")} onClick={() => void state.runOperation("export")}><Download data-icon="inline-start" />{taskSettingsDirty ? t("settings.saveAndRun", { operation: t("common.export") }) : t("common.exportSubtitles")}</Button>
      </div>
    </header>
    <TaskFlowStrip flowSteps={state.flowSteps} progressLabel={t("common.progress")} />
    {visibleNotice && <div className="workbench-notices" tabIndex={0}><NoticeAlert message={visibleNotice} /></div>}
    <div className="workbench-body">
      <SubtitlePreviewCard key={task.id} task={task} activeSubtitleBody={state.activeSubtitleBody} activeSubtitleFileName={state.activeSubtitleFileName} hasTranslatedSubtitle={state.hasTranslatedSubtitle} subtitlePreview={state.subtitlePreview} subtitleView={state.subtitleView} t={t} onRefreshPreview={() => { void state.refreshPreview(); }} setSubtitleView={state.setSubtitleView} onSourceSaved={state.sourceSubtitlesSaved} commandPending={commandPending} />
      <aside id="task-inspector" className="task-inspector" hidden={!inspectorOpen} aria-label={t("workbench.configure")} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); closeInspector(); } }}>
        <header><strong>{t("workbench.configure")}</strong><Button ref={inspectorClose} variant="ghost" size="icon-sm" onClick={closeInspector} aria-label={t("common.close")}><X /></Button></header>
        <div className="inspector-content">
          <TaskConfigCard task={task} taskConfig={taskConfig} taskSettingsDirty={taskSettingsDirty} commandPending={commandPending} t={t} onApplyCurrentSettings={state.applyCurrentSettings} onPickTranslationModel={state.pickTaskTranslationModel} onPickWhisperModel={state.pickTaskWhisperModel} onSaveTaskSettings={state.saveTaskSettings} setSettingsDraft={state.setSettingsDraft} />
          <dl className="inspector-metadata"><dt>{t("flow.material")}</dt><dd>{taskSourcePath(task)}</dd><dt>{t("task.outputDir")}</dt><dd>{task.output_dir || task.settings.output_dir || t("task.sameAsSourceDir")}</dd></dl>
          {task.exported_output_dir && <Button variant="outline" size="sm" onClick={state.openOutputDir}><ExternalLink data-icon="inline-start" />{t("common.openExportDir")}</Button>}
        </div>
      </aside>
    </div>
    <div className="workbench-logs" hidden={!logsOpen}><TaskLogsCard logs={state.logs} t={t} /></div>
    <footer className="workbench-footer">
      <Button variant="ghost" size="sm" aria-expanded={logsOpen} onClick={() => setLogsOpen(!logsOpen)}><Terminal data-icon="inline-start" />{t("tabs.logs")}<ChevronDown /></Button>
      <details className="workbench-message" onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.currentTarget.open = false; event.currentTarget.querySelector("summary")?.focus(); } }}><summary>{task.message || t("common.progress")}</summary><p tabIndex={0}>{task.message}</p></details>
      {taskBusy(task) && <Progress value={progressValue(task.progress)} />}
      <span>{task.segment_count != null ? t("workbench.cueCount", { count: task.segment_count }) : ""}</span>
    </footer>
  </section>;
}
