import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

import { applyUpdaterDownloadEvent, initialUpdaterProgress, type UpdaterProgress } from "@/lib/updater-progress";

export type AvailableAppUpdate = {
  update: Update;
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
};

export async function currentAppVersion() {
  return getVersion();
}

export async function checkAppUpdate(): Promise<AvailableAppUpdate | null> {
  const update = await check();
  if (!update) return null;

  return {
    update,
    currentVersion: update.currentVersion,
    version: update.version,
    date: update.date,
    body: update.body,
  };
}

export async function downloadAndInstallAppUpdate(
  available: AvailableAppUpdate,
  onProgress: (progress: UpdaterProgress) => void,
) {
  let progress = initialUpdaterProgress();
  onProgress(progress);

  await available.update.downloadAndInstall((event) => {
    progress = applyUpdaterDownloadEvent(progress, event);
    onProgress(progress);
  });
}

export function relaunchApp() {
  return relaunch();
}
