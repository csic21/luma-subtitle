import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { selectVideo } from "./tauri-api";

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
