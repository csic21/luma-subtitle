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
import { useTaskDetailState } from "@/hooks/use-task-detail-state";
import { useI18n } from "@/i18n";

export function TaskDetailPage() {
  const { taskId = "" } = useParams();
  const navigate = useNavigate();
  const { locale, t } = useI18n();
  const {
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
  } = useTaskDetailState(taskId, t);

  if (!task || !taskConfig) {
    return (
      <>
        <NoticeAlert message={notice} />
        <Card className="loading-panel">
          <CardContent>{t("task.loading")}</CardContent>
        </Card>
      </>
    );
  }

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
            operationContext={operationContext}
          />
          <TaskConfigCard
            task={task}
            taskConfig={taskConfig}
            taskSettingsDirty={taskSettingsDirty}
            t={t}
            onApplyCurrentSettings={applyCurrentSettings}
            onPickWhisperModel={pickTaskWhisperModel}
            onSaveTaskSettings={saveTaskSettings}
            setSettingsDraft={setSettingsDraft}
          />
        </div>

        <div className="right-column">
          <TaskProgressCard
            task={task}
            t={t}
            onOpenOutputDir={openOutputDir}
            onRunOperation={runOperation}
            operationContext={operationContext}
          />
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
