import { lazy, Suspense, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Download,
  ExternalLink,
  FileAudio,
  FileText,
  FileVideo,
  Languages,
  Loader2,
  Pencil,
  Play,
  RefreshCw,
  Subtitles,
  Terminal,
} from "lucide-react";

import { SectionTitle, StatusBadge } from "@/components/app/shared";
import { SubtitleSourceEditor } from "@/components/app/subtitle-source-editor";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { Locale, useI18n } from "@/i18n";
import {
  canRunOperation,
  formattedTime,
  hasPartialTranslationProgress,
  operationRequirementIssues,
  operationRequirementSummary,
  type OperationReadinessContext,
  progressLabel,
  progressValue,
  stageText,
  taskBusy,
  taskSourcePath,
} from "@/lib/app-utils";
import { cn } from "@/lib/utils";
import type { SubtitlePreview, TaskOperation, TaskRecord } from "@/types";

type Translate = ReturnType<typeof useI18n>["t"];

type FlowStep = {
  label: string;
  detail: string;
  state: string;
};

type OperationHandler = (operation: TaskOperation) => void | Promise<void>;
const NO_EDITS = {};
const ignoreTextChange = () => {};

const SubtitleEditorDialog = lazy(() => import("@/components/app/subtitle-editor-dialog")
  .then((module) => ({ default: module.SubtitleEditorDialog })));

export function TaskFlowStrip({ flowSteps, progressLabel }: { flowSteps: FlowStep[]; progressLabel: string }) {
  return (
    <section className="flow-strip" aria-label={progressLabel}>
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
  );
}

export function TaskSummaryCard({
  locale,
  task,
  t,
  onCancelTask,
  onRunOperation,
  operationContext,
  commandPending = false,
  taskSettingsDirty = false,
}: {
  locale: Locale;
  task: TaskRecord;
  t: Translate;
  onCancelTask: () => void | Promise<void>;
  onRunOperation: OperationHandler;
  operationContext: OperationReadinessContext;
  commandPending?: boolean;
  taskSettingsDirty?: boolean;
}) {
  const transcribeIssues = operationRequirementIssues(task, "transcribe", operationContext);
  const translateIssues = operationRequirementIssues(task, "translate", operationContext);
  const resumeTranslateIssues = operationRequirementIssues(task, "resume_translate", operationContext);
  const exportIssues = operationRequirementIssues(task, "export", operationContext);
  const showResumeTranslate = hasPartialTranslationProgress(task);
  const runLabel = (key: string) => taskSettingsDirty ? t("settings.saveAndRun", { operation: t(key) }) : t(key);
  const materialIcon =
    task.source_type === "audio" ? (
      <FileAudio />
    ) : task.source_type === "srt" ? (
      <FileText />
    ) : (
      <FileVideo />
    );

  return (
    <Card>
      <CardHeader>
        <SectionTitle icon={materialIcon} title={t("tabs.task")} />
      </CardHeader>
      <CardContent className="stack-panel">
        <div className="detail-list">
          <span>{t("flow.material")}</span>
          <code>{taskSourcePath(task)}</code>
          <span>{t("task.outputDir")}</span>
          <code>{task.output_dir || task.settings.output_dir || t("task.sameAsSourceDir")}</code>
          <span>{t("common.updatedAt")}</span>
          <code>{formattedTime(task.updated_at, locale)}</code>
          <span>{t("task.segmentCount")}</span>
          <code>{task.segment_count ?? "-"}</code>
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
            onClick={() => onRunOperation("transcribe")}
            disabled={commandPending || !canRunOperation(task, "transcribe", operationContext)}
            title={transcribeIssues.length ? operationRequirementSummary(transcribeIssues, t) : t("common.transcribe")}
          >
            <Play data-icon="inline-start" />
            {runLabel("common.transcribe")}
          </Button>
          <Button
            variant="secondary"
            onClick={() => onRunOperation("translate")}
            disabled={commandPending || !canRunOperation(task, "translate", operationContext)}
            title={translateIssues.length ? operationRequirementSummary(translateIssues, t) : t("common.translate")}
          >
            <Languages data-icon="inline-start" />
            {runLabel("common.translate")}
          </Button>
          {showResumeTranslate && (
            <Button
              variant="secondary"
              onClick={() => onRunOperation("resume_translate")}
              disabled={commandPending || !canRunOperation(task, "resume_translate", operationContext)}
              title={
                resumeTranslateIssues.length
                  ? operationRequirementSummary(resumeTranslateIssues, t)
                  : t("common.resumeTranslate")
              }
            >
              <RefreshCw data-icon="inline-start" />
              {runLabel("common.resumeTranslate")}
              {typeof task.translation_completed_count === "number" && task.segment_count
                ? ` (${task.translation_completed_count}/${task.segment_count})`
                : ""}
            </Button>
          )}
          <Button
            onClick={() => onRunOperation("export")}
            disabled={commandPending || !canRunOperation(task, "export", operationContext)}
            title={exportIssues.length ? operationRequirementSummary(exportIssues, t) : t("common.export")}
          >
            <Download data-icon="inline-start" />
            {runLabel("common.export")}
          </Button>
          <Button variant="destructive" onClick={onCancelTask} disabled={!taskBusy(task)}>
            <CircleStop data-icon="inline-start" />
            {t("common.cancel")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

export function TaskProgressCard({
  task,
  t,
  onOpenOutputDir,
  onRunOperation,
  operationContext,
  commandPending = false,
  taskSettingsDirty = false,
}: {
  task: TaskRecord;
  t: Translate;
  onOpenOutputDir: () => void | Promise<void>;
  onRunOperation: OperationHandler;
  operationContext: OperationReadinessContext;
  commandPending?: boolean;
  taskSettingsDirty?: boolean;
}) {
  const statusIcon: ReactNode =
    task.status === "completed" || task.status === "exported" ? (
      <CheckCircle2 />
    ) : taskBusy(task) ? (
      <Loader2 className="spin" />
    ) : (
      <Terminal />
    );

  return (
    <Card className={cn("progress-panel", `progress-${task.status}`)}>
      <CardHeader>
        <SectionTitle icon={statusIcon} title={t("common.progress")} />
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
            <Button onClick={() => onRunOperation("export")} disabled={commandPending || !canRunOperation(task, "export", operationContext)}>
              <Download data-icon="inline-start" />
              {taskSettingsDirty ? t("settings.saveAndRun", { operation: t("common.exportSubtitles") }) : t("common.exportSubtitles")}
            </Button>
            {task.source_file_name && <code>{task.source_file_name}</code>}
            {task.translated_file_name && <code>{task.translated_file_name}</code>}
            {task.exported_output_dir && (
              <Button variant="secondary" onClick={onOpenOutputDir} title={t("common.openExportDir")}>
                <ExternalLink data-icon="inline-start" />
                {t("common.openExportDir")}
              </Button>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function SubtitlePreviewCard({
  task,
  activeSubtitleBody,
  activeSubtitleFileName,
  hasTranslatedSubtitle,
  subtitlePreview,
  subtitleView,
  t,
  onRefreshPreview,
  setSubtitleView,
  onSourceSaved,
  commandPending = false,
}: {
  task: TaskRecord;
  activeSubtitleBody?: string | null;
  activeSubtitleFileName?: string | null;
  hasTranslatedSubtitle: boolean;
  subtitlePreview: SubtitlePreview | null;
  subtitleView: "translated" | "source" | "parallel";
  t: Translate;
  onRefreshPreview: () => void | Promise<void>;
  setSubtitleView: Dispatch<SetStateAction<"translated" | "source" | "parallel">>;
  onSourceSaved: (task: TaskRecord, kind: "source" | "translated") => Promise<void>;
  commandPending?: boolean;
}) {
  const [editorPreview, setEditorPreview] = useState<SubtitlePreview | null>(null);
  const [editorKind, setEditorKind] = useState<"source" | "translated">("source");
  const editButtonRef = useRef<HTMLButtonElement>(null);
  const editTranslationButtonRef = useRef<HTMLButtonElement>(null);
  return (
    <Card>
      <CardHeader>
        <SectionTitle icon={<Subtitles />} title={t("subtitle.preview")} />
        <CardAction className="flex flex-wrap gap-2">
          <Button
            ref={editButtonRef}
            variant="outline"
            size="sm"
            disabled={commandPending || !subtitlePreview?.source_segments.length || taskBusy(task)}
            title={taskBusy(task) ? t("subtitle.editBusy") : undefined}
            onClick={() => { setEditorKind("source"); setEditorPreview(subtitlePreview); }}
          >
            <Pencil data-icon="inline-start" />
            {t("subtitle.editSource")}
          </Button>
          {hasTranslatedSubtitle && <Button
            ref={editTranslationButtonRef}
            variant="outline"
            size="sm"
            disabled={commandPending || !subtitlePreview?.translated_segments?.length || taskBusy(task)}
            onClick={() => { setEditorKind("translated"); setEditorPreview(subtitlePreview); }}
          >
            <Pencil data-icon="inline-start" />
            {t("subtitle.editTranslated")}
          </Button>}
          <Button variant="secondary" size="sm" onClick={onRefreshPreview}>
            <RefreshCw data-icon="inline-start" />
            {t("common.refresh")}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent>
        <Tabs
          value={subtitleView}
          onValueChange={(value) => setSubtitleView(value as "translated" | "source" | "parallel")}
          className="subtitle-tabs"
        >
          <TabsList>
            <TabsTrigger value="source">{t("common.source")}</TabsTrigger>
            {hasTranslatedSubtitle && <TabsTrigger value="translated">{t("common.translated")}</TabsTrigger>}
            {Boolean(subtitlePreview?.translated_segments?.length) && <TabsTrigger value="parallel">{t("subtitle.parallel")}</TabsTrigger>}
          </TabsList>
        </Tabs>

        {subtitlePreview ? (
          <>
            {subtitleView === "parallel" ? <SubtitleSourceEditor
              segments={subtitlePreview.translated_segments ?? []}
              referenceSegments={subtitlePreview.source_segments}
              referenceLabel={t("common.source")}
              edits={NO_EDITS}
              onTextChange={ignoreTextChange}
              disabled={false}
              readOnly
              t={t}
            /> : <>
            <code className="subtitle-file">{activeSubtitleFileName}</code>
            <ScrollArea className="subtitle-preview">
              <pre>{activeSubtitleBody || t("subtitle.noContent")}</pre>
            </ScrollArea>
            </>}
          </>
        ) : (
          <div className="subtitle-empty">{t("subtitle.empty")}</div>
        )}
      </CardContent>
      {editorPreview && (
        <Suspense fallback={<span role="status">{t("subtitle.loadingEditor")}</span>}>
          <SubtitleEditorDialog
            taskId={task.id}
            preview={editorPreview}
            kind={editorKind}
            busy={commandPending || taskBusy(task)}
            t={t}
            onClose={() => setEditorPreview(null)}
            onSaved={onSourceSaved}
            onReturnFocus={() => (editorKind === "source" ? editButtonRef : editTranslationButtonRef).current?.focus()}
          />
        </Suspense>
      )}
    </Card>
  );
}

export function TaskLogsCard({ logs, t }: { logs: string[]; t: Translate }) {
  return (
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
  );
}
