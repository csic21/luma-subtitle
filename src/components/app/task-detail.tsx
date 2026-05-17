import type { Dispatch, ReactNode, SetStateAction } from "react";
import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Download,
  ExternalLink,
  FileVideo,
  Languages,
  Loader2,
  Play,
  RefreshCw,
  Subtitles,
  Terminal,
} from "lucide-react";

import { SectionTitle, StatusBadge } from "@/components/app/shared";
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
  progressLabel,
  progressValue,
  stageText,
  taskBusy,
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
}: {
  locale: Locale;
  task: TaskRecord;
  t: Translate;
  onCancelTask: () => void | Promise<void>;
  onRunOperation: OperationHandler;
}) {
  return (
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
            disabled={!canRunOperation(task, "transcribe")}
          >
            <Play data-icon="inline-start" />
            {t("common.transcribe")}
          </Button>
          <Button
            variant="secondary"
            onClick={() => onRunOperation("translate")}
            disabled={!canRunOperation(task, "translate")}
          >
            <Languages data-icon="inline-start" />
            {t("common.translate")}
          </Button>
          <Button onClick={() => onRunOperation("export")} disabled={!canRunOperation(task, "export")}>
            <Download data-icon="inline-start" />
            {t("common.export")}
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
}: {
  task: TaskRecord;
  t: Translate;
  onOpenOutputDir: () => void | Promise<void>;
  onRunOperation: OperationHandler;
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
            <Button onClick={() => onRunOperation("export")} disabled={!canRunOperation(task, "export")}>
              <Download data-icon="inline-start" />
              {t("common.exportSubtitles")}
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
  activeSubtitleBody,
  activeSubtitleFileName,
  hasTranslatedSubtitle,
  subtitlePreview,
  subtitleView,
  t,
  onRefreshPreview,
  setSubtitleView,
}: {
  activeSubtitleBody?: string | null;
  activeSubtitleFileName?: string | null;
  hasTranslatedSubtitle: boolean;
  subtitlePreview: SubtitlePreview | null;
  subtitleView: "translated" | "source";
  t: Translate;
  onRefreshPreview: () => void | Promise<void>;
  setSubtitleView: Dispatch<SetStateAction<"translated" | "source">>;
}) {
  return (
    <Card>
      <CardHeader>
        <SectionTitle icon={<Subtitles />} title={t("subtitle.preview")} />
        <CardAction>
          <Button variant="secondary" size="sm" onClick={onRefreshPreview}>
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
