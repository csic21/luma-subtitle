import type { Dispatch, SetStateAction } from "react";
import {
  AlertCircle,
  CheckCircle2,
  Download,
  FolderOpen,
  KeyRound,
  Loader2,
  RefreshCw,
  Save,
  Settings,
  Terminal,
} from "lucide-react";

import { DownloadProgress, FieldBlock, IconAction, SectionTitle, StatusBadge } from "@/components/app/shared";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defaultSettings, languageOptions, whisperLanguageOptions, whisperModelPresets } from "@/config";
import type { useI18n } from "@/i18n";
import { bytesLabel, fileName, progressLabel, progressValue } from "@/lib/app-utils";
import { cn } from "@/lib/utils";
import type { AppUpdateState, DependencyInstallEvent, EnvironmentState, ModelDownloadEvent, SettingsState } from "@/types";

type Translate = ReturnType<typeof useI18n>["t"];

export function ModelApiSettingsCard({
  apiKey,
  hasApiCredential,
  modelDownload,
  modelDownloading,
  selectedWhisperPreset,
  settings,
  t,
  whisperPresetId,
  onDownloadWhisperPreset,
  onPickWhisperModel,
  onSaveSettings,
  setApiKey,
  setSettings,
  setWhisperPresetId,
}: {
  apiKey: string;
  hasApiCredential: boolean;
  modelDownload: ModelDownloadEvent | null;
  modelDownloading: boolean;
  selectedWhisperPreset: (typeof whisperModelPresets)[number];
  settings: SettingsState;
  t: Translate;
  whisperPresetId: string;
  onDownloadWhisperPreset: () => void | Promise<void>;
  onPickWhisperModel: () => void | Promise<void>;
  onSaveSettings: () => void | Promise<void>;
  setApiKey: Dispatch<SetStateAction<string>>;
  setSettings: Dispatch<SetStateAction<SettingsState>>;
  setWhisperPresetId: Dispatch<SetStateAction<string>>;
}) {
  return (
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
              onClick={onPickWhisperModel}
              title={settings.whisper_model_path || t("settings.selectWhisper")}
            />
            <IconAction label={t("settings.selectWhisper")} onClick={onPickWhisperModel}>
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
              onClick={onDownloadWhisperPreset}
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
              onValueChange={(value) => setSettings((current) => ({ ...current, whisper_language: value }))}
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
              onValueChange={(value) => setSettings((current) => ({ ...current, target_language: value }))}
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
              onChange={(event) => setSettings((current) => ({ ...current, base_url: event.target.value }))}
            />
          </FieldBlock>
          <FieldBlock label={t("settings.translationModel")}>
            <Input
              value={settings.model}
              onChange={(event) => setSettings((current) => ({ ...current, model: event.target.value }))}
            />
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
                setSettings((current) => ({ ...current, temperature: Number.parseFloat(event.target.value) || 0 }))
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
              setSettings((current) => ({
                ...current,
                translation_shard_size:
                  Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
              }))
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
          <Button variant="secondary" onClick={onSaveSettings} title={t("common.save")}>
            <Save data-icon="inline-start" />
            {t("common.save")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

export function EnvironmentSettingsCard({
  dependencyInstall,
  dependencyInstalling,
  environmentReady,
  env,
  envRows,
  t,
  onInstallDependencies,
  onOpenManagedDir,
  onRefreshEnvironment,
}: {
  dependencyInstall: DependencyInstallEvent | null;
  dependencyInstalling: boolean;
  environmentReady: boolean;
  env: EnvironmentState | null;
  envRows: string[][];
  t: Translate;
  onInstallDependencies: () => void | Promise<void>;
  onOpenManagedDir: (path?: string | null) => void | Promise<void>;
  onRefreshEnvironment: () => void | Promise<void>;
}) {
  return (
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
          <Button variant="secondary" onClick={onRefreshEnvironment}>
            <RefreshCw data-icon="inline-start" />
            {t("common.refresh")}
          </Button>
          <Button variant="secondary" onClick={() => onOpenManagedDir(env?.sidecar_dir)} disabled={!env?.sidecar_dir}>
            <FolderOpen data-icon="inline-start" />
            {t("common.openDir")}
          </Button>
          <Button variant="secondary" onClick={onInstallDependencies} disabled={dependencyInstalling}>
            {dependencyInstalling ? <Loader2 data-icon="inline-start" className="spin" /> : <Download data-icon="inline-start" />}
            {t("download.installDependencies")}
          </Button>
        </div>
        {dependencyInstall && <DownloadProgress event={dependencyInstall} />}
      </CardContent>
    </Card>
  );
}

export function UpdateSettingsCard({
  appUpdate,
  appUpdating,
  t,
  onCheckForUpdates,
  onInstallUpdate,
}: {
  appUpdate: AppUpdateState;
  appUpdating: boolean;
  t: Translate;
  onCheckForUpdates: () => void | Promise<void>;
  onInstallUpdate: () => void | Promise<void>;
}) {
  const statusLabel = updateStatusLabel(appUpdate.status, t);
  const canInstall = appUpdate.status === "available";
  const progressMeta = updateProgressMeta(appUpdate);

  return (
    <Card>
      <CardHeader>
        <SectionTitle icon={<RefreshCw />} title={t("update.section")} description={t("update.description")} />
        <CardAction>
          <StatusBadge status={updateBadgeStatus(appUpdate.status)} label={statusLabel} />
        </CardAction>
      </CardHeader>
      <CardContent className="stack-panel">
        <div className="update-version-grid">
          <span>{t("update.currentVersion")}</span>
          <code>{appUpdate.currentVersion || t("settings.notSet")}</code>
          {appUpdate.availableVersion && (
            <>
              <span>{t("update.availableVersion")}</span>
              <code>{appUpdate.availableVersion}</code>
            </>
          )}
        </div>

        {appUpdate.body && <div className="update-notes">{appUpdate.body}</div>}

        {appUpdate.status === "downloading" && (
          <div className="download-card">
            <div className="progress-head compact">
              <span>{t("update.downloading")}</span>
              <strong>{progressLabel(appUpdate.progress)}</strong>
            </div>
            <Progress className="hotdog-progress" value={progressValue(appUpdate.progress)} />
            {progressMeta && <div className="download-meta">{progressMeta}</div>}
          </div>
        )}

        {appUpdate.error && (
          <Alert className="credential-alert warn">
            <AlertCircle />
            <AlertTitle>{t("update.failed")}</AlertTitle>
            <AlertDescription>{appUpdate.error}</AlertDescription>
          </Alert>
        )}

        <div className="action-row end">
          <Button variant="secondary" onClick={onCheckForUpdates} disabled={appUpdating}>
            {appUpdate.status === "checking" ? (
              <Loader2 data-icon="inline-start" className="spin" />
            ) : (
              <RefreshCw data-icon="inline-start" />
            )}
            {t("update.check")}
          </Button>
          <Button variant="secondary" onClick={onInstallUpdate} disabled={!canInstall || appUpdating}>
            {appUpdate.status === "downloading" ? (
              <Loader2 data-icon="inline-start" className="spin" />
            ) : (
              <Download data-icon="inline-start" />
            )}
            {t("update.install")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function updateStatusLabel(status: AppUpdateState["status"], t: Translate) {
  const labels: Record<AppUpdateState["status"], string> = {
    unsupported: t("update.unsupported"),
    idle: t("update.idle"),
    checking: t("update.checking"),
    available: t("update.available"),
    downloading: t("update.downloading"),
    ready: t("update.ready"),
    unavailable: t("update.none"),
    failed: t("update.failed"),
  };
  return labels[status];
}

function updateBadgeStatus(status: AppUpdateState["status"]) {
  if (status === "available") return "warn";
  if (status === "failed") return "failed";
  if (status === "checking" || status === "downloading") return "running";
  if (status === "ready" || status === "unavailable") return "ready";
  return "pending";
}

function updateProgressMeta(appUpdate: AppUpdateState) {
  const downloaded = bytesLabel(appUpdate.downloadedBytes);
  const total = bytesLabel(appUpdate.totalBytes);
  if (downloaded && total) return `${downloaded} / ${total}`;
  return downloaded;
}
