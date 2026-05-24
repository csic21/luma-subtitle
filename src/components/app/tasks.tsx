import { memo } from "react";
import {
  CircleStop,
  Database,
  Download,
  Eye,
  FileAudio,
  FileText,
  FileVideo,
  FolderOpen,
  Languages,
  Play,
  Plus,
  RefreshCw,
  Trash2,
  Upload,
} from "lucide-react";

import { IconAction, SectionTitle, StatusBadge } from "@/components/app/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import type { Locale, useI18n } from "@/i18n";
import {
  fileName,
  formattedTime,
  operationRequirementIssues,
  operationRequirementSummary,
  type OperationReadinessContext,
  progressValue,
  stageText,
  taskBusy,
  taskSourcePath,
} from "@/lib/app-utils";
import type { QueueSettings, TaskOperation, TaskRecord } from "@/types";

type Translate = ReturnType<typeof useI18n>["t"];
type OperationHandler = (taskId: string, operation: TaskOperation) => void | Promise<void>;
type SelectedOperationHandler = (operation: TaskOperation) => void | Promise<void>;

export function TaskMetricGrid({
  busyCount,
  doneCount,
  failedCount,
  taskCount,
  t,
}: {
  busyCount: number;
  doneCount: number;
  failedCount: number;
  taskCount: number;
  t: Translate;
}) {
  return (
    <section className="metric-grid">
      <MetricCard label={t("metric.all")} value={taskCount} />
      <MetricCard label={t("metric.running")} value={busyCount} />
      <MetricCard label={t("metric.done")} value={doneCount} />
      <MetricCard label={t("metric.failed")} value={failedCount} />
    </section>
  );
}

function MetricCard({ label, value }: { label: string; value: number }) {
  return (
    <Card size="sm" className="metric-card">
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardTitle>{value}</CardTitle>
      </CardHeader>
    </Card>
  );
}

export function TaskToolbar({
  canRunSelectedOperations,
  outputDir,
  queueSettings,
  t,
  onCancelSelected,
  onCreateAudioTask,
  onCreateSrtTask,
  onCreateVideoTask,
  onPickOutputDir,
  onRefreshTasks,
  onRunSelected,
  onSaveAutoStartNext,
  onSaveConcurrency,
}: {
  canRunSelectedOperations: Record<TaskOperation, boolean>;
  outputDir: string;
  queueSettings: QueueSettings;
  t: Translate;
  onCancelSelected: () => void | Promise<void>;
  onCreateAudioTask: () => void | Promise<void>;
  onCreateSrtTask: () => void | Promise<void>;
  onCreateVideoTask: () => void | Promise<void>;
  onPickOutputDir: () => void | Promise<void>;
  onRefreshTasks: () => void | Promise<void>;
  onRunSelected: SelectedOperationHandler;
  onSaveAutoStartNext: (autoStartNext: boolean) => void | Promise<void>;
  onSaveConcurrency: (maxConcurrency: number) => void | Promise<void>;
}) {
  return (
    <Card className="toolbar-card">
      <CardContent className="toolbar-content">
        <div className="toolbar-main">
          <Button size="sm" onClick={onCreateVideoTask} title={t("task.videoNewTitle")}>
            <Plus data-icon="inline-start" />
            {t("task.videoNew")}
          </Button>
          <Button size="sm" variant="secondary" onClick={onCreateAudioTask} title={t("task.audioNewTitle")}>
            <FileAudio data-icon="inline-start" />
            {t("task.audioNew")}
          </Button>
          <Button size="sm" variant="secondary" onClick={onCreateSrtTask} title={t("task.srtImportTitle")}>
            <Upload data-icon="inline-start" />
            {t("task.srtImport")}
          </Button>
          <Button size="sm" variant="secondary" onClick={onPickOutputDir} title={t("task.pickOutputDir")}>
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
              onChange={(event) => void onSaveConcurrency(Number.parseInt(event.target.value, 10) || 1)}
            />
          </div>
          <label className="auto-next-field" htmlFor="auto-start-next" title={t("task.autoStartNextTitle")}>
            <Checkbox
              id="auto-start-next"
              checked={queueSettings.auto_start_next}
              onCheckedChange={(checked) => void onSaveAutoStartNext(checked === true)}
            />
            <span>{t("task.autoStartNext")}</span>
          </label>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onRunSelected("transcribe")}
            disabled={!canRunSelectedOperations.transcribe}
          >
            <Play data-icon="inline-start" />
            {t("common.transcribe")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onRunSelected("translate")}
            disabled={!canRunSelectedOperations.translate}
          >
            <Languages data-icon="inline-start" />
            {t("common.translate")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onRunSelected("export")}
            disabled={!canRunSelectedOperations.export}
          >
            <Download data-icon="inline-start" />
            {t("common.export")}
          </Button>
          <Button size="sm" variant="destructive" onClick={onCancelSelected}>
            <CircleStop data-icon="inline-start" />
            {t("common.cancel")}
          </Button>
          <IconAction label={t("common.refresh")} onClick={onRefreshTasks}>
            <RefreshCw />
          </IconAction>
        </div>
      </CardContent>
    </Card>
  );
}

export function TaskQueueTable({
  allSelected,
  locale,
  operationContext,
  selectedIds,
  tasks,
  t,
  onCancelTask,
  onDeleteTask,
  onOpenTask,
  onRunOperation,
  onToggleAll,
  onToggleTask,
}: {
  allSelected: boolean;
  locale: Locale;
  operationContext: OperationReadinessContext;
  selectedIds: Set<string>;
  tasks: TaskRecord[];
  t: Translate;
  onCancelTask: (taskId: string) => void | Promise<void>;
  onDeleteTask: (taskId: string) => void | Promise<void>;
  onOpenTask: (taskId: string) => void;
  onRunOperation: OperationHandler;
  onToggleAll: () => void;
  onToggleTask: (taskId: string) => void;
}) {
  return (
    <Card className="task-table-card">
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
                <Checkbox checked={allSelected} onCheckedChange={onToggleAll} aria-label={t("task.ariaSelectAll")} />
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
                <TaskQueueRow
                  key={task.id}
                  locale={locale}
                  operationContext={operationContext}
                  selected={selectedIds.has(task.id)}
                  task={task}
                  t={t}
                  onCancelTask={onCancelTask}
                  onDeleteTask={onDeleteTask}
                  onOpenTask={onOpenTask}
                  onRunOperation={onRunOperation}
                  onToggleTask={onToggleTask}
                />
              ))
            )}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

const TaskQueueRow = memo(function TaskQueueRow({
  locale,
  operationContext,
  selected,
  task,
  t,
  onCancelTask,
  onDeleteTask,
  onOpenTask,
  onRunOperation,
  onToggleTask,
}: {
  locale: Locale;
  operationContext: OperationReadinessContext;
  selected: boolean;
  task: TaskRecord;
  t: Translate;
  onCancelTask: (taskId: string) => void | Promise<void>;
  onDeleteTask: (taskId: string) => void | Promise<void>;
  onOpenTask: (taskId: string) => void;
  onRunOperation: OperationHandler;
  onToggleTask: (taskId: string) => void;
}) {
  const transcribeIssues = operationRequirementIssues(task, "transcribe", operationContext);
  const translateIssues = operationRequirementIssues(task, "translate", operationContext);
  const exportIssues = operationRequirementIssues(task, "export", operationContext);
  const canTranscribe = transcribeIssues.length === 0;
  const canTranslate = translateIssues.length === 0;
  const canExport = exportIssues.length === 0;

  return (
    <TableRow data-state={taskBusy(task) ? "selected" : undefined}>
      <TableCell className="select-cell">
        <Checkbox
          checked={selected}
          onCheckedChange={() => onToggleTask(task.id)}
          aria-label={t("task.ariaSelect", { fileName: task.file_name })}
        />
      </TableCell>
      <TableCell>
        <Button variant="ghost" className="file-button" onClick={() => onOpenTask(task.id)}>
          {task.source_type === "audio" ? (
            <FileAudio data-icon="inline-start" />
          ) : task.source_type === "srt" ? (
            <FileText data-icon="inline-start" />
          ) : (
            <FileVideo data-icon="inline-start" />
          )}
          <span>{task.file_name}</span>
        </Button>
        <small>{task.source_type === "srt" ? "SRT" : fileName(taskSourcePath(task))}</small>
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
        <code className="table-code">{task.output_dir || task.settings.output_dir || t("task.sameAsSourceDir")}</code>
      </TableCell>
      <TableCell>{formattedTime(task.updated_at, locale)}</TableCell>
      <TableCell>
        <div className="row-actions">
          <IconAction
            label={transcribeIssues.length ? operationRequirementSummary(transcribeIssues, t) : t("common.transcribe")}
            onClick={() => onRunOperation(task.id, "transcribe")}
            disabled={!canTranscribe}
          >
            <Play />
          </IconAction>
          <IconAction
            label={translateIssues.length ? operationRequirementSummary(translateIssues, t) : t("common.translate")}
            onClick={() => onRunOperation(task.id, "translate")}
            disabled={!canTranslate}
          >
            <Languages />
          </IconAction>
          <IconAction
            label={exportIssues.length ? operationRequirementSummary(exportIssues, t) : t("common.export")}
            onClick={() => onRunOperation(task.id, "export")}
            disabled={!canExport}
          >
            <Download />
          </IconAction>
          <IconAction label={t("common.cancel")} onClick={() => onCancelTask(task.id)} disabled={!taskBusy(task)}>
            <CircleStop />
          </IconAction>
          <IconAction label={t("common.details")} onClick={() => onOpenTask(task.id)}>
            <Eye />
          </IconAction>
          <IconAction
            label={t("common.delete")}
            onClick={() => onDeleteTask(task.id)}
            disabled={taskBusy(task)}
            className="danger-action"
          >
            <Trash2 />
          </IconAction>
        </div>
      </TableCell>
    </TableRow>
  );
});
