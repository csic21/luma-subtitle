import { useNavigate } from "react-router-dom";

import { NoticeAlert } from "@/components/app/shared";
import { TaskMetricGrid, TaskQueueTable, TaskToolbar } from "@/components/app/tasks";
import { useTasksPageState } from "@/hooks/use-tasks-page-state";
import { useI18n } from "@/i18n";

export function TasksPage() {
  const navigate = useNavigate();
  const { locale, t } = useI18n();
  const {
    allSelected,
    busyCount,
    cancelSelected,
    cancelTask,
    createSrtTask,
    createVideoTask,
    deleteTask,
    doneCount,
    failedCount,
    notice,
    outputDir,
    pickOutputDir,
    queueSettings,
    refreshTasks,
    runOperation,
    runSelected,
    saveAutoStartNext,
    saveConcurrency,
    selectedIds,
    tasks,
    toggleAll,
    toggleTask,
  } = useTasksPageState(t);

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
        onSaveAutoStartNext={saveAutoStartNext}
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
