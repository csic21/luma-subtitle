import { describe, expect, it } from "vitest";

import { mergeTaskLogs, shouldReplaceTaskSettingsDraft } from "@/lib/task-data";
import type { TaskSettingsSnapshot } from "@/types";

const settings: TaskSettingsSnapshot = {
  output_dir: null,
  target_language: "简体中文",
  whisper_model_path: "models/ggml.bin",
  whisper_language: "auto",
  base_url: "https://api.openai.com",
  base_url_is_complete: false,
  model: "gpt-4o-mini",
  temperature: 0.2,
  translation_shard_size: 200,
};

describe("shouldReplaceTaskSettingsDraft", () => {
  it("preserves a dirty draft for the current task and replaces clean or forced drafts", () => {
    const currentTask = { id: "task-1", settings };
    const dirtyDraft = { ...settings, model: "custom-model" };

    expect(shouldReplaceTaskSettingsDraft(currentTask, dirtyDraft, currentTask)).toBe(false);
    expect(shouldReplaceTaskSettingsDraft(currentTask, settings, currentTask)).toBe(true);
    expect(shouldReplaceTaskSettingsDraft(currentTask, dirtyDraft, { id: "task-2" })).toBe(true);
    expect(shouldReplaceTaskSettingsDraft(currentTask, dirtyDraft, currentTask, true)).toBe(true);
  });
});

describe("mergeTaskLogs", () => {
  it("keeps realtime events that arrive after a log snapshot starts loading", () => {
    expect(mergeTaskLogs(["created · pending"], ["created · pending", "transcribing · working"])).toEqual([
      "created · pending",
      "transcribing · working",
    ]);
  });
});
