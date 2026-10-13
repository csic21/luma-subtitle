/** App updater is unavailable in the browser fixture; no network or relaunch. */
import type { UpdaterProgress } from "@/lib/updater-progress";
export type AvailableAppUpdate = { currentVersion: string; version: string; date?: string; body?: string };
export async function currentAppVersion() { return "1.2.4-fixture"; }
export async function checkAppUpdate(): Promise<AvailableAppUpdate | null> { return null; }
export async function downloadAndInstallAppUpdate(_available: AvailableAppUpdate, _onProgress: (progress: UpdaterProgress) => void): Promise<void> { throw new Error("Fixture blocks application installation. No downloads or native actions occurred."); }
export async function relaunchApp(): Promise<void> { throw new Error("Fixture blocks application relaunch."); }
