import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { selectAudio, selectVideo, subtitlePreview } from "./tauri-api";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

describe("selectVideo", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("opens the frontend video picker with the supported video extensions", async () => {
    vi.mocked(open).mockResolvedValue("/videos/clip.mp4");

    await expect(selectVideo()).resolves.toBe("/videos/clip.mp4");

    expect(open).toHaveBeenCalledWith({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mkv", "mov", "avi", "webm", "m4v"] }],
    });
    expect(invoke).not.toHaveBeenCalledWith("select_video");
  });
});

describe("selectAudio", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("opens the frontend audio picker with the supported audio extensions", async () => {
    vi.mocked(open).mockResolvedValue("/audio/voice.m4a");

    await expect(selectAudio()).resolves.toBe("/audio/voice.m4a");

    expect(open).toHaveBeenCalledWith({
      multiple: false,
      filters: [
        {
          name: "Audio",
          extensions: [
            "mp3",
            "wav",
            "m4a",
            "aac",
            "flac",
            "ogg",
            "opus",
            "webm",
            "wma",
            "aiff",
            "aif",
            "caf",
            "mka",
          ],
        },
      ],
    });
  });
});

describe("subtitlePreview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("keeps the persisted task command and camelCase argument", () => {
    void subtitlePreview("task-1");

    expect(invoke).toHaveBeenCalledWith("subtitle_preview", { jobId: "task-1" });
  });
});
