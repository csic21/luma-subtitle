import { describe, expect, it } from "vitest";

import { applyUpdaterDownloadEvent, initialUpdaterProgress } from "./updater-progress";

describe("applyUpdaterDownloadEvent", () => {
  it("tracks content length and accumulated downloaded bytes", () => {
    let progress = initialUpdaterProgress();

    progress = applyUpdaterDownloadEvent(progress, { event: "Started", data: { contentLength: 100 } });
    expect(progress).toEqual({ downloadedBytes: 0, totalBytes: 100, progress: 0 });

    progress = applyUpdaterDownloadEvent(progress, { event: "Progress", data: { chunkLength: 25 } });
    expect(progress).toEqual({ downloadedBytes: 25, totalBytes: 100, progress: 0.25 });

    progress = applyUpdaterDownloadEvent(progress, { event: "Progress", data: { chunkLength: 100 } });
    expect(progress).toEqual({ downloadedBytes: 125, totalBytes: 100, progress: 1 });
  });

  it("marks downloads complete even when the server does not report content length", () => {
    const progress = applyUpdaterDownloadEvent(initialUpdaterProgress(), { event: "Finished" });

    expect(progress).toEqual({ downloadedBytes: 0, totalBytes: null, progress: 1 });
  });
});
