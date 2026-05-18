import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { defaultSettings, whisperModelPresets } from "@/config";
import { errorText, fileName, hasTauriRuntime } from "@/lib/app-utils";
import {
  checkEnvironment,
  downloadStatus,
  downloadWhisperModel,
  installDependencies as installDependenciesCommand,
  loadSettings,
  openPath,
  saveSettings as saveSettingsCommand,
  selectWhisperModel,
} from "@/lib/tauri-api";
import type { DependencyInstallEvent, DownloadStatus, EnvironmentState, ModelDownloadEvent, SettingsState, TFunction } from "@/types";

export function useSettingsPageState(t: TFunction) {
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [apiKey, setApiKey] = useState("");
  const [env, setEnv] = useState<EnvironmentState | null>(null);
  const [notice, setNotice] = useState("");
  const [whisperPresetId, setWhisperPresetId] = useState(whisperModelPresets[1].id);
  const [modelDownload, setModelDownload] = useState<ModelDownloadEvent | null>(null);
  const [dependencyInstall, setDependencyInstall] = useState<DependencyInstallEvent | null>(null);

  const selectedWhisperPreset = useMemo(
    () => whisperModelPresets.find((preset) => preset.id === whisperPresetId) ?? whisperModelPresets[0],
    [whisperPresetId],
  );
  const modelDownloading = modelDownload?.status === "running";
  const dependencyInstalling = dependencyInstall?.status === "running";
  const hasApiCredential = settings.has_api_key || apiKey.trim().length > 0;
  const environmentReady = Boolean(env?.ffmpeg_path && env?.whisper_path);
  const tauriReady = hasTauriRuntime();

  const envRows = useMemo(() => {
    if (!env) return [];
    return [
      ["FFmpeg", env.ffmpeg_path ?? t("env.missing")],
      ["whisper.cpp", env.whisper_path ?? t("env.missing")],
      ["GPU", env.gpu_name ? `${env.gpu_name}${env.cuda_driver ? ` / ${env.cuda_driver}` : ""}` : t("env.gpuMissing")],
      [t("env.dependencyDir"), env.sidecar_dir],
      [t("env.modelDir"), env.model_dir],
      [t("env.resourceDir"), env.resource_dir],
      [t("env.configDir"), env.config_dir],
    ];
  }, [env, t]);

  const refreshSettings = useCallback(async () => {
    try {
      const loaded = await loadSettings();
      setSettings({ ...defaultSettings, ...loaded });
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const refreshEnvironment = useCallback(async () => {
    try {
      setEnv(await checkEnvironment());
    } catch (error) {
      setNotice(errorText(error));
    }
  }, []);

  const refreshDownloadStatus = useCallback(async () => {
    try {
      const status = await downloadStatus();
      if (status.model) setModelDownload(status.model);
      if (status.dependency) setDependencyInstall(status.dependency);
    } catch {
      // Event updates still work if polling is unavailable in an older backend.
    }
  }, []);

  useEffect(() => {
    if (!tauriReady) {
      setNotice(t("notice.requireTauriConfig"));
      return;
    }

    void refreshSettings();
    void refreshEnvironment();
    void refreshDownloadStatus();

    let disposed = false;
    let unlistenModelDownload: (() => void) | undefined;
    let unlistenDependencyInstall: (() => void) | undefined;

    listen<ModelDownloadEvent>("model-download-event", (event) => {
      setModelDownload(event.payload);
      if (event.payload.status === "failed") setNotice(event.payload.error ?? event.payload.message);
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenModelDownload = fn;
    });

    listen<DependencyInstallEvent>("dependency-install-event", (event) => {
      setDependencyInstall(event.payload);
      if (event.payload.status === "failed") setNotice(event.payload.error ?? event.payload.message);
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenDependencyInstall = fn;
    });

    return () => {
      disposed = true;
      unlistenModelDownload?.();
      unlistenDependencyInstall?.();
    };
  }, [refreshDownloadStatus, refreshEnvironment, refreshSettings, t, tauriReady]);

  useEffect(() => {
    if (!modelDownloading && !dependencyInstalling) return;
    const timer = window.setInterval(() => {
      void refreshDownloadStatus();
    }, 1000);
    return () => window.clearInterval(timer);
  }, [dependencyInstalling, modelDownloading, refreshDownloadStatus]);

  const saveSettings = useCallback(
    async (showNotice = true) => {
      try {
        const saved = await saveSettingsCommand({
          ...settings,
          api_key: apiKey,
        });
        setSettings(saved);
        if (showNotice) setNotice(t("notice.settingsSaved"));
      } catch (error) {
        if (showNotice) setNotice(t("error.saveSettings", { error: errorText(error) }));
        throw error;
      }
    },
    [apiKey, settings, t],
  );

  const pickWhisperModel = useCallback(async () => {
    try {
      const picked = await selectWhisperModel();
      if (picked) setSettings((current) => ({ ...current, whisper_model_path: picked }));
    } catch (error) {
      setNotice(t("error.pickWhisperModel", { error: errorText(error) }));
    }
  }, [t]);

  const downloadWhisperPreset = useCallback(async () => {
    setNotice("");
    setModelDownload({
      preset_id: selectedWhisperPreset.id,
      file_name: selectedWhisperPreset.fileName,
      status: "running",
      message: t("download.modelPreparing"),
      progress: 0,
    });

    try {
      const modelPath = await downloadWhisperModel(selectedWhisperPreset.id);
      const nextSettings = { ...settings, whisper_model_path: modelPath };
      setSettings(nextSettings);
      const saved = await saveSettingsCommand({
        ...nextSettings,
        api_key: apiKey,
      });
      setSettings(saved);
      setModelDownload((current) => ({
        ...(current ?? {}),
        preset_id: selectedWhisperPreset.id,
        file_name: selectedWhisperPreset.fileName,
        status: "completed",
        message: t("download.completed"),
        progress: 1,
        path: modelPath,
        error: null,
      }));
      setNotice(t("notice.downloadedAndSelected", { fileName: fileName(modelPath) }));
    } catch (error) {
      setModelDownload((current) =>
        current
          ? {
              ...current,
              status: "failed",
              message: t("download.failed"),
              progress: 0,
              error: String(error),
            }
          : null,
      );
      setNotice(String(error));
    }
  }, [apiKey, selectedWhisperPreset, settings, t]);

  const installDependencies = useCallback(async () => {
    setNotice("");
    setDependencyInstall({
      item: t("download.dependencyItem"),
      status: "running",
      message: t("download.dependencyPreparing"),
      progress: 0,
    });

    try {
      const installedPaths = await installDependenciesCommand();
      await refreshEnvironment();
      setDependencyInstall((current) => ({
        item: current?.item ?? t("download.dependencyItem"),
        ...(current ?? {}),
        status: "completed",
        message: t("download.dependencyCompleted"),
        progress: 1,
        path: installedPaths[installedPaths.length - 1] ?? current?.path ?? null,
        error: null,
      }));
      setNotice(t("notice.depsInstalled"));
    } catch (error) {
      setDependencyInstall((current) =>
        current
          ? {
              ...current,
              status: "failed",
              message: t("download.dependencyFailed"),
              progress: 0,
              error: String(error),
            }
          : null,
      );
      setNotice(String(error));
    }
  }, [refreshEnvironment, t]);

  const openManagedDir = useCallback(async (path?: string | null) => {
    if (!path) return;
    await openPath(path);
  }, []);

  return {
    apiKey,
    dependencyInstall,
    dependencyInstalling,
    downloadWhisperPreset,
    env,
    environmentReady,
    envRows,
    hasApiCredential,
    installDependencies,
    modelDownload,
    modelDownloading,
    notice,
    openManagedDir,
    pickWhisperModel,
    refreshEnvironment,
    saveSettings,
    selectedWhisperPreset,
    setApiKey,
    setSettings,
    setWhisperPresetId,
    settings,
    whisperPresetId,
  };
}
