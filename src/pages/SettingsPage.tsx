import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, ArrowLeft, CheckCircle2, Download, FolderOpen, KeyRound, Loader2, RefreshCw, Save, Settings, Terminal } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { DownloadProgress, FieldBlock, IconAction, NoticeAlert, SectionTitle, StatusBadge } from "@/components/app/shared";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defaultSettings, languageOptions, whisperLanguageOptions, whisperModelPresets } from "@/config";
import { useI18n } from "@/i18n";
import { errorText, fileName, hasTauriRuntime } from "@/lib/app-utils";
import { cn } from "@/lib/utils";
import type { DependencyInstallEvent, DownloadStatus, EnvironmentState, ModelDownloadEvent, SettingsState } from "@/types";

export function SettingsPage() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [apiKey, setApiKey] = useState("");
  const [env, setEnv] = useState<EnvironmentState | null>(null);
  const [notice, setNotice] = useState("");
  const [whisperPresetId, setWhisperPresetId] = useState(whisperModelPresets[1].id);
  const [modelDownload, setModelDownload] = useState<ModelDownloadEvent | null>(null);
  const [dependencyInstall, setDependencyInstall] = useState<DependencyInstallEvent | null>(null);

  const selectedWhisperPreset =
    whisperModelPresets.find((preset) => preset.id === whisperPresetId) ?? whisperModelPresets[0];
  const modelDownloading = modelDownload?.status === "running";
  const dependencyInstalling = dependencyInstall?.status === "running";
  const hasApiCredential = settings.has_api_key || apiKey.trim().length > 0;
  const environmentReady = Boolean(env?.ffmpeg_path && env?.whisper_path);

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

  useEffect(() => {
    if (!hasTauriRuntime()) {
      setNotice(t("notice.requireTauriConfig"));
      return;
    }

    void refreshSettings();
    void refreshEnvironment();
    void refreshDownloadStatus();

    let unlistenModelDownload: (() => void) | undefined;
    let unlistenDependencyInstall: (() => void) | undefined;
    listen<ModelDownloadEvent>("model-download-event", (event) => {
      setModelDownload(event.payload);
      if (event.payload.status === "failed") setNotice(event.payload.error ?? event.payload.message);
    }).then((fn) => {
      unlistenModelDownload = fn;
    });
    listen<DependencyInstallEvent>("dependency-install-event", (event) => {
      setDependencyInstall(event.payload);
      if (event.payload.status === "failed") setNotice(event.payload.error ?? event.payload.message);
    }).then((fn) => {
      unlistenDependencyInstall = fn;
    });
    return () => {
      unlistenModelDownload?.();
      unlistenDependencyInstall?.();
    };
  }, [t]);

  useEffect(() => {
    if (!modelDownloading && !dependencyInstalling) return;
    const timer = window.setInterval(() => {
      void refreshDownloadStatus();
    }, 1000);
    return () => window.clearInterval(timer);
  }, [modelDownloading, dependencyInstalling]);

  async function refreshSettings() {
    try {
      const loaded = await invoke<SettingsState>("load_settings");
      setSettings({ ...defaultSettings, ...loaded });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshEnvironment() {
    try {
      setEnv(await invoke<EnvironmentState>("check_environment"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshDownloadStatus() {
    try {
      const status = await invoke<DownloadStatus>("download_status");
      if (status.model) setModelDownload(status.model);
      if (status.dependency) setDependencyInstall(status.dependency);
    } catch {
      // Event updates still work if polling is unavailable in an older backend.
    }
  }

  async function saveSettings(showNotice = true) {
    try {
      const saved = await invoke<SettingsState>("save_settings", {
        payload: {
          ...settings,
          api_key: apiKey,
        },
      });
      setSettings(saved);
      if (showNotice) setNotice(t("notice.settingsSaved"));
    } catch (error) {
      if (showNotice) setNotice(t("error.saveSettings", { error: errorText(error) }));
      throw error;
    }
  }

  async function pickWhisperModel() {
    try {
      const picked = await invoke<string | null>("select_whisper_model");
      if (picked) setSettings((current) => ({ ...current, whisper_model_path: picked }));
    } catch (error) {
      setNotice(t("error.pickWhisperModel", { error: errorText(error) }));
    }
  }

  async function downloadWhisperPreset() {
    setNotice("");
    setModelDownload({
      preset_id: selectedWhisperPreset.id,
      file_name: selectedWhisperPreset.fileName,
      status: "running",
      message: t("download.modelPreparing"),
      progress: 0,
    });

    try {
      const modelPath = await invoke<string>("download_whisper_model", {
        request: { preset_id: selectedWhisperPreset.id },
      });
      const nextSettings = { ...settings, whisper_model_path: modelPath };
      setSettings(nextSettings);
      const saved = await invoke<SettingsState>("save_settings", {
        payload: {
          ...nextSettings,
          api_key: apiKey,
        },
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
  }

  async function installDependencies() {
    setNotice("");
    setDependencyInstall({
      item: t("download.dependencyItem"),
      status: "running",
      message: t("download.dependencyPreparing"),
      progress: 0,
    });

    try {
      const installedPaths = await invoke<string[]>("install_dependencies");
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
  }

  async function openManagedDir(path?: string | null) {
    if (!path) return;
    await invoke("open_path", { path });
  }

  return (
    <>
      <section className="page-heading">
        <Button variant="secondary" onClick={() => navigate("/tasks")}>
          <ArrowLeft data-icon="inline-start" />
          {t("common.backToQueue")}
        </Button>
        <div>
          <h1>{t("app.settings")}</h1>
          <p>{t("settings.description")}</p>
        </div>
      </section>

      <NoticeAlert message={notice} />

      <section className="settings-page-grid">
        <Card>
          <CardHeader>
            <SectionTitle icon={<Settings />} title={t("settings.modelApi")} description={t("settings.modelApiDescription")} />
          </CardHeader>
          <CardContent className="settings-form">
            <FieldBlock label={t("common.whisperModel")}>
              <div className="input-action">
                <Input
                  value={settings.whisper_model_path ? fileName(settings.whisper_model_path) : ""}
                  readOnly
                  placeholder={selectedWhisperPreset.fileName}
                  onClick={pickWhisperModel}
                  title={settings.whisper_model_path || t("settings.selectWhisper")}
                />
                <IconAction label={t("settings.selectWhisper")} onClick={pickWhisperModel}>
                  <FolderOpen />
                </IconAction>
              </div>
            </FieldBlock>

            <FieldBlock label={t("model.preset")}>
              <div className="input-action">
                <Select value={whisperPresetId} onValueChange={setWhisperPresetId}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {whisperModelPresets.map((preset) => (
                        <SelectItem key={preset.id} value={preset.id}>
                          {t(preset.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
                <IconAction
                  label={t("download.pickPreset", { fileName: selectedWhisperPreset.fileName })}
                  onClick={downloadWhisperPreset}
                  disabled={modelDownloading}
                >
                  {modelDownloading ? <Loader2 className="spin" /> : <Download />}
                </IconAction>
              </div>
            </FieldBlock>

            {modelDownload && <DownloadProgress event={modelDownload} />}

            <div className="grid-two">
              <FieldBlock label={t("settings.sourceLanguage")}>
                <Select
                  value={settings.whisper_language}
                  onValueChange={(value) => setSettings({ ...settings, whisper_language: value })}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {whisperLanguageOptions.map((language) => (
                        <SelectItem key={language.value} value={language.value}>
                          {t(language.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </FieldBlock>
              <FieldBlock label={t("common.targetLanguage")}>
                <Select
                  value={settings.target_language}
                  onValueChange={(value) => setSettings({ ...settings, target_language: value })}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {languageOptions.map((language) => (
                        <SelectItem key={language.value} value={language.value}>
                          {t(language.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </FieldBlock>
            </div>

            <div className="grid-two">
              <FieldBlock label="Base URL">
                <Input
                  value={settings.base_url}
                  onChange={(event) => setSettings({ ...settings, base_url: event.target.value })}
                />
              </FieldBlock>
              <FieldBlock label={t("settings.translationModel")}>
                <Input value={settings.model} onChange={(event) => setSettings({ ...settings, model: event.target.value })} />
              </FieldBlock>
            </div>

            <div className="grid-two">
              <FieldBlock label="API Key">
                <div className="key-field">
                  <KeyRound />
                  <Input
                    type="password"
                    value={apiKey}
                    onChange={(event) => setApiKey(event.target.value)}
                    placeholder={settings.has_api_key ? t("settings.apiKeySaved") : t("settings.apiKeyUnset")}
                  />
                </div>
              </FieldBlock>
              <FieldBlock label="Temperature">
                <Input
                  type="number"
                  min="0"
                  max="1"
                  step="0.1"
                  value={settings.temperature}
                  onChange={(event) =>
                    setSettings({ ...settings, temperature: Number.parseFloat(event.target.value) || 0 })
                  }
                />
              </FieldBlock>
            </div>

            <FieldBlock label={t("settings.shardSize")}>
              <Input
                type="number"
                min="1"
                max="1000"
                step="1"
                value={settings.translation_shard_size}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    translation_shard_size:
                      Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
                  })
                }
              />
            </FieldBlock>

            <Alert className={cn("credential-alert", hasApiCredential ? "ready" : "warn")}>
              {hasApiCredential ? <CheckCircle2 /> : <AlertCircle />}
              <AlertTitle>{hasApiCredential ? t("settings.apiReady") : t("settings.apiWarn")}</AlertTitle>
              <AlertDescription>
                {hasApiCredential ? t("settings.apiReadyDescription") : t("settings.apiWarnDescription")}
              </AlertDescription>
            </Alert>

            <div className="action-row end">
              <Button variant="secondary" onClick={() => saveSettings()} title={t("common.save")}>
                <Save data-icon="inline-start" />
                {t("common.save")}
              </Button>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <SectionTitle icon={<Terminal />} title={t("env.section")} description={t("env.description")} />
            <CardAction>
              <StatusBadge status={environmentReady ? "ready" : "warn"} label={environmentReady ? t("env.ready") : t("env.warn")} />
            </CardAction>
          </CardHeader>
          <CardContent className="stack-panel">
            <div className="env-table">
              {envRows.map(([name, value]) => (
                <div className="env-row" key={name}>
                  <span>{name}</span>
                  <code>{value}</code>
                </div>
              ))}
            </div>
            <div className="action-row end">
              <Button variant="secondary" onClick={() => refreshEnvironment()}>
                <RefreshCw data-icon="inline-start" />
                {t("common.refresh")}
              </Button>
              <Button variant="secondary" onClick={() => openManagedDir(env?.sidecar_dir)} disabled={!env?.sidecar_dir}>
                <FolderOpen data-icon="inline-start" />
                {t("common.openDir")}
              </Button>
              <Button variant="secondary" onClick={installDependencies} disabled={dependencyInstalling}>
                {dependencyInstalling ? <Loader2 data-icon="inline-start" className="spin" /> : <Download data-icon="inline-start" />}
                {t("download.installDependencies")}
              </Button>
            </div>
            {dependencyInstall && <DownloadProgress event={dependencyInstall} />}
          </CardContent>
        </Card>
      </section>
    </>
  );
}


