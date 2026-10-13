import { memo, type ReactNode } from "react";
import { DropdownMenu } from "radix-ui";
import {
  CircleStop, Download, Eye, FileAudio, FileText, FileVideo, FolderOpen,
  Languages, MoreHorizontal, Play, Plus, RefreshCw, Trash2, Upload,
} from "lucide-react";

import { IconAction, StatusBadge } from "@/components/app/shared";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import type { Locale, useI18n } from "@/i18n";
import {
  formattedTime, hasPartialTranslationProgress, operationRequirementIssues,
  operationRequirementSummary, type OperationReadinessContext, progressValue,
  stageText, statusText, taskBusy, taskSourcePath,
} from "@/lib/app-utils";
import type { QueueSettings, TaskOperation, TaskRecord } from "@/types";

type Translate = ReturnType<typeof useI18n>["t"];
type OperationHandler = (taskId: string, operation: TaskOperation) => void | Promise<void>;
type SelectedOperationHandler = (operation: TaskOperation) => void | Promise<void>;

export function TaskMetricGrid({ busyCount, doneCount, failedCount, taskCount, t }: {
  busyCount: number; doneCount: number; failedCount: number; taskCount: number; t: Translate;
}) {
  return (
    <dl className="queue-summary">
      <div><dt>{t("metric.all")}</dt><dd>{taskCount}</dd></div>
      <div data-tone="running"><dt>{t("metric.running")}</dt><dd>{busyCount}</dd></div>
      <div><dt>{t("metric.done")}</dt><dd>{doneCount}</dd></div>
      <div data-tone={failedCount ? "failed" : undefined}><dt>{t("metric.failed")}</dt><dd>{failedCount}</dd></div>
    </dl>
  );
}

export function TaskToolbar({
  canRunSelectedOperations, outputDir, queueSettings, selectedCount, summary, t,
  onCancelSelected, onCreateAudioTask, onCreateSrtTask, onCreateVideoTask,
  onPickOutputDir, onRefreshTasks, onRunSelected, onSaveAutoStartNext, onSaveConcurrency,
}: {
  canRunSelectedOperations: Record<TaskOperation, boolean>;
  outputDir: string;
  queueSettings: QueueSettings;
  selectedCount: number;
  summary: ReactNode;
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
    <>
      <header className="queue-header">
        <div className="queue-heading">
          <h1>{t("task.queue")}</h1>
          {summary}
        </div>
        <div className="queue-import-actions">
          <Button size="sm" onClick={onCreateVideoTask} title={t("task.videoNewTitle")}>
            <Plus data-icon="inline-start" />{t("task.videoNew")}
          </Button>
          <Button size="sm" variant="outline" onClick={onCreateAudioTask} title={t("task.audioNewTitle")}>
            <FileAudio data-icon="inline-start" />{t("task.audioNew")}
          </Button>
          <Button size="sm" variant="outline" onClick={onCreateSrtTask} title={t("task.srtImportTitle")}>
            <Upload data-icon="inline-start" />{t("task.srtImport")}
          </Button>
        </div>
      </header>

      <div className="queue-settings" role="group" aria-label={t("task.queueSettings")}>
        <div className="queue-output">
          <Button size="sm" variant="ghost" onClick={onPickOutputDir} title={t("task.pickOutputDir")}>
            <FolderOpen data-icon="inline-start" />{t("task.inputDir")}
          </Button>
          <span title={outputDir || t("task.defaultOutput")}>{outputDir || t("task.defaultOutput")}</span>
        </div>
        <FieldGroup className="queue-settings-fields">
          <Field orientation="horizontal" className="queue-concurrency">
            <FieldLabel htmlFor="max-concurrency">{t("task.concurrency")}</FieldLabel>
            <Input id="max-concurrency" type="number" min="1" max="4" value={queueSettings.max_concurrency}
              onChange={(event) => void onSaveConcurrency(Number.parseInt(event.target.value, 10) || 1)} />
          </Field>
          <Field orientation="horizontal" className="queue-auto-next" title={t("task.autoStartNextTitle")}>
            <Checkbox id="auto-start-next" checked={queueSettings.auto_start_next}
              onCheckedChange={(checked) => void onSaveAutoStartNext(checked === true)} />
            <FieldLabel htmlFor="auto-start-next">{t("task.autoStartNext")}</FieldLabel>
          </Field>
        </FieldGroup>
      </div>

      <div className="queue-selection-bar" data-selected={selectedCount > 0 || undefined}>
        {selectedCount > 0 ? (
          <div className="queue-batch-actions" role="group" aria-label={t("task.selected", { count: selectedCount })}>
            <span className="queue-selection-count" aria-live="polite">{t("task.selected", { count: selectedCount })}</span>
            <Button size="sm" variant="ghost" onClick={() => onRunSelected("transcribe")} disabled={!canRunSelectedOperations.transcribe}>
              <Play data-icon="inline-start" />{t("common.transcribe")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onRunSelected("translate")} disabled={!canRunSelectedOperations.translate}>
              <Languages data-icon="inline-start" />{t("common.translate")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onRunSelected("export")} disabled={!canRunSelectedOperations.export}>
              <Download data-icon="inline-start" />{t("common.export")}
            </Button>
            <Button size="sm" variant="ghost" onClick={onCancelSelected}>
              <CircleStop data-icon="inline-start" />{t("common.cancel")}
            </Button>
          </div>
        ) : <p className="queue-selection-hint">{t("task.queueDescription")}</p>}
        <IconAction label={t("common.refresh")} onClick={onRefreshTasks}><RefreshCw /></IconAction>
      </div>
    </>
  );
}

export function TaskQueueTable({
  allSelected, locale, operationContext, selectedIds, tasks, t,
  onCancelTask, onDeleteTask, onOpenTask, onRunOperation, onToggleAll, onToggleTask,
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
    <section className="queue-table-panel" aria-label={t("task.queue")}>
      <Table className="queue-table" aria-label={t("task.queue")}>
        <colgroup>
          <col className="queue-select-column" />
          <col />
          <col className="queue-status-column" />
          <col className="queue-language-column" />
          <col className="queue-updated-column" />
          <col className="queue-actions-column" />
        </colgroup>
        <TableHeader>
          <TableRow>
            <TableHead className="queue-select-cell">
              <Checkbox checked={allSelected ? true : selectedIds.size > 0 ? "indeterminate" : false}
                disabled={tasks.length === 0} onCheckedChange={onToggleAll} aria-label={t("task.ariaSelectAll")} />
            </TableHead>
            <TableHead>{t("common.file")}</TableHead>
            <TableHead>{t("common.status")}</TableHead>
            <TableHead>{t("common.targetLanguage")}</TableHead>
            <TableHead className="queue-updated-cell">{t("common.updatedAt")}</TableHead>
            <TableHead>{t("task.tableAction")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {tasks.length === 0 ? (
            <TableRow>
              <TableCell colSpan={6}>
                <div className="queue-empty-state">
                  <FileVideo aria-hidden="true" /><strong>{t("task.empty")}</strong>
                  <span>{t("task.videoNewTitle")} · {t("task.audioNewTitle")} · {t("task.srtImportTitle")}</span>
                </div>
              </TableCell>
            </TableRow>
          ) : tasks.map((task) => (
            <TaskQueueRow key={task.id} locale={locale} operationContext={operationContext}
              selected={selectedIds.has(task.id)} task={task} t={t}
              onCancelTask={onCancelTask} onDeleteTask={onDeleteTask} onOpenTask={onOpenTask}
              onRunOperation={onRunOperation} onToggleTask={onToggleTask} />
          ))}
        </TableBody>
      </Table>
    </section>
  );
}

const TaskQueueRow = memo(function TaskQueueRow({
  locale, operationContext, selected, task, t, onCancelTask, onDeleteTask, onOpenTask, onRunOperation, onToggleTask,
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
  const busy = taskBusy(task);
  const outputDir = task.output_dir || task.settings.output_dir || t("task.sameAsSourceDir");
  const updatedAt = formattedTime(task.updated_at, locale);
  const stageLabel = stageText(task.stage, t);
  const showStage = stageLabel !== statusText(task.status, t);
  return (
    <TableRow data-state={selected ? "selected" : undefined} data-busy={busy || undefined} aria-selected={selected}>
      <TableCell className="queue-select-cell">
        <Checkbox checked={selected} onCheckedChange={() => onToggleTask(task.id)} aria-label={t("task.ariaSelect", { fileName: task.file_name })} />
      </TableCell>
      <TableCell className="queue-file-cell">
        <Button variant="ghost" className="queue-file-button" title={task.file_name} onClick={() => onOpenTask(task.id)}>
          {task.source_type === "audio" ? <FileAudio data-icon="inline-start" /> : task.source_type === "srt" ? <FileText data-icon="inline-start" /> : <FileVideo data-icon="inline-start" />}
          <span>{task.file_name}</span>
        </Button>
        <div className="queue-file-meta" title={`${t("common.source")}: ${taskSourcePath(task)}\n${t("task.outputDir")}: ${outputDir}`}>
          <FolderOpen aria-hidden="true" /><span>{outputDir}</span>
        </div>
        <time className="queue-compact-date" title={t("common.updatedAt")}>{updatedAt}</time>
      </TableCell>
      <TableCell className="queue-status-cell">
        <div className="queue-status-heading"><StatusBadge status={task.status} />{showStage ? <span>{stageLabel}</span> : null}</div>
        <div className="queue-progress" title={task.message}>
          {busy ? <Progress value={progressValue(task.progress)} aria-label={`${t("common.progress")}: ${task.file_name}`} /> : null}
          <span className="queue-status-message">{task.message}</span>
          {busy ? <span className="queue-progress-value">{progressValue(task.progress)}%</span> : null}
        </div>
      </TableCell>
      <TableCell className="queue-language-cell">{task.settings.target_language}</TableCell>
      <TableCell className="queue-updated-cell"><time title={updatedAt}>{updatedAt}</time></TableCell>
      <TableCell>
        <div className="queue-row-actions">
          <IconAction label={t("common.details")} onClick={() => onOpenTask(task.id)}><Eye /></IconAction>
          <TaskRowMenu task={task} operationContext={operationContext} t={t}
            onRunOperation={onRunOperation} onCancelTask={onCancelTask} onDeleteTask={onDeleteTask} />
        </div>
      </TableCell>
    </TableRow>
  );
});

export function TaskRowMenu({ task, operationContext, t, onRunOperation, onCancelTask, onDeleteTask }: {
  task: TaskRecord;
  operationContext: OperationReadinessContext;
  t: Translate;
  onRunOperation: OperationHandler;
  onCancelTask: (taskId: string) => void | Promise<void>;
  onDeleteTask: (taskId: string) => void | Promise<void>;
}) {
  const operations: Array<{ operation: TaskOperation; label: string; icon: ReactNode }> = [
    { operation: "transcribe", label: t("common.transcribe"), icon: <Play /> },
    { operation: "translate", label: t("common.translate"), icon: <Languages /> },
    ...(hasPartialTranslationProgress(task) ? [{ operation: "resume_translate" as const, label: t("common.resumeTranslate"), icon: <RefreshCw /> }] : []),
    { operation: "export", label: t("common.export"), icon: <Download /> },
  ];
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <Button size="icon-sm" variant="ghost" aria-label={t("task.rowActions", { fileName: task.file_name })}><MoreHorizontal /></Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content className="queue-action-menu" align="end" sideOffset={4} collisionPadding={8}>
          <DropdownMenu.Group>
            {operations.map(({ operation, label, icon }) => {
              const issues = operationRequirementIssues(task, operation, operationContext);
              const explanation = issues.length ? operationRequirementSummary(issues, t) : undefined;
              return <DropdownMenu.Item key={operation} className="queue-menu-item" disabled={issues.length > 0}
                textValue={label} onSelect={() => void onRunOperation(task.id, operation)}>
                {icon}<span>{label}{explanation ? <small>{explanation}</small> : null}</span>
              </DropdownMenu.Item>;
            })}
          </DropdownMenu.Group>
          <DropdownMenu.Separator className="queue-menu-separator" />
          <DropdownMenu.Group>
            <DropdownMenu.Item className="queue-menu-item" disabled={!taskBusy(task)} onSelect={() => void onCancelTask(task.id)}><CircleStop /><span>{t("common.cancel")}</span></DropdownMenu.Item>
            <DropdownMenu.Item className="queue-menu-item" data-destructive="true" disabled={taskBusy(task)} onSelect={() => void onDeleteTask(task.id)}><Trash2 /><span>{t("common.delete")}</span></DropdownMenu.Item>
          </DropdownMenu.Group>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
