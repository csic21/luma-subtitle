import type { DownloadEvent } from "@tauri-apps/plugin-updater";

export type UpdaterProgress = {
  downloadedBytes: number;
  totalBytes: number | null;
  progress: number;
};

export function initialUpdaterProgress(): UpdaterProgress {
  return {
    downloadedBytes: 0,
    totalBytes: null,
    progress: 0,
  };
}

export function applyUpdaterDownloadEvent(current: UpdaterProgress, event: DownloadEvent): UpdaterProgress {
  if (event.event === "Started") {
    return {
      downloadedBytes: 0,
      totalBytes: event.data.contentLength ?? null,
      progress: 0,
    };
  }

  if (event.event === "Progress") {
    const downloadedBytes = current.downloadedBytes + event.data.chunkLength;
    return {
      ...current,
      downloadedBytes,
      progress: progressFromBytes(downloadedBytes, current.totalBytes),
    };
  }

  return {
    ...current,
    progress: 1,
  };
}

function progressFromBytes(downloadedBytes: number, totalBytes: number | null) {
  if (!totalBytes || totalBytes <= 0) return 0;
  return Math.min(downloadedBytes / totalBytes, 1);
}
