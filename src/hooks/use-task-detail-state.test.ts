import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("useTaskDetailState resume behavior", () => {
  it("does not recheck run prerequisites when the detail view regains focus", () => {
    const source = readFileSync(new URL("./use-task-detail-state.ts", import.meta.url), "utf8");
    const resumeHandler = source.match(/const refreshTaskOnResume = useCallback\(\(\) => \{([\s\S]*?)\n  \},/);

    expect(resumeHandler?.[1]).toContain("refreshTask");
    expect(resumeHandler?.[1]).not.toContain("refreshRunPrerequisites");
  });

  it("avoids refetching logs for every task event and fetches the initial task data in parallel", () => {
    const source = readFileSync(new URL("./use-task-detail-state.ts", import.meta.url), "utf8");
    const taskUpdatedHandler = source.match(/listen<TaskRecord>\("task-updated", \(event\) => \{([\s\S]*?)\n    \}\)\.then/);

    expect(source).toContain("Promise.all([getTask(taskId), getTaskLogs(taskId).catch(() => [])])");
    expect(source).toContain("shouldReplaceTaskSettingsDraft");
    expect(taskUpdatedHandler?.[1]).not.toContain("refreshLogs");
    expect(taskUpdatedHandler?.[1]).toContain("subtitlePathsChanged");
  });
});
