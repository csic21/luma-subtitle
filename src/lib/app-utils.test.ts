import { describe, expect, it } from "vitest";

import { canRunOperation, operationRequirementIssues } from "./app-utils";
import type { TaskRecord } from "@/types";

function task(overrides: Partial<TaskRecord> = {}): TaskRecord {
  return {
    id: "task-1",
    source_type: "video",
    video_path: "/tmp/video.mp4",
    file_name: "video.mp4",
    status: "created",
    stage: "created",
    message: "Waiting",
    progress: 0,
    settings: {
      target_language: "zh-CN",
      whisper_model_path: "/models/whisper.gguf",
      whisper_language: "auto",
      base_url: "https://api.openai.com",
      base_url_is_complete: false,
      model: "gpt-4.1-mini",
      temperature: 0.2,
      translation_shard_size: 30,
    },
    created_at: 1,
    updated_at: 1,
    ...overrides,
  };
}

describe("operation readiness", () => {
  it("blocks transcription until the model and local tools are configured", () => {
    const pending = task({
      settings: {
        ...task().settings,
        whisper_model_path: "",
      },
    });

    expect(operationRequirementIssues(pending, "transcribe", { environmentReady: false, hasApiCredential: false })).toEqual([
      "missingWhisperModel",
      "missingEnvironment",
    ]);
    expect(canRunOperation(pending, "transcribe", { environmentReady: false, hasApiCredential: false })).toBe(false);
  });

  it("blocks translation until source subtitles and translation API configuration are ready", () => {
    const pending = task({
      source_srt_path: "",
      settings: {
        ...task().settings,
        base_url: "",
        model: "",
      },
    });

    expect(operationRequirementIssues(pending, "translate", { environmentReady: true, hasApiCredential: false })).toEqual([
      "missingSourceSubtitles",
      "missingBaseUrl",
      "missingTranslationModel",
      "missingApiKey",
    ]);
    expect(canRunOperation(pending, "translate", { environmentReady: true, hasApiCredential: false })).toBe(false);
  });

  it("allows export with source subtitles even when translation configuration is missing", () => {
    const ready = task({
      source_srt_path: "/tmp/video.srt",
      settings: {
        ...task().settings,
        base_url: "",
        model: "",
      },
    });

    expect(operationRequirementIssues(ready, "export", { environmentReady: false, hasApiCredential: false })).toEqual([]);
    expect(canRunOperation(ready, "export", { environmentReady: false, hasApiCredential: false })).toBe(true);
  });
});
