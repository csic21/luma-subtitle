import { useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import {
  AlertCircle,
  BookOpenCheck,
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defaultSettings, languageOptions, whisperLanguageOptions, whisperModelPresets } from "@/config";
import type { useI18n } from "@/i18n";
import { bytesLabel, errorText, fileName, hasTauriRuntime, progressLabel, progressValue } from "@/lib/app-utils";
import { checkTranslationCli, listTranslationCliModels } from "@/lib/tauri-api";
import { cn } from "@/lib/utils";
import type { AppUpdateState, DependencyInstallEvent, EnvironmentState, ModelDownloadEvent, SettingsState, TranslationCliStatus } from "@/types";

type Translate = ReturnType<typeof useI18n>["t"];

export function ModelApiSettingsCard({
  apiKey,
  downloadedWhisperModelFiles,
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
  downloadedWhisperModelFiles: Set<string>;
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
  const selectedPresetDownloaded = downloadedWhisperModelFiles.has(selectedWhisperPreset.fileName);
  const missingWhisperModel = !settings.whisper_model_path.trim();
  const translationProvider = settings.translation_provider ?? defaultSettings.translation_provider;
  const isCliProvider = translationProvider === "cli";
  const cliTool = settings.translation_cli_tool ?? defaultSettings.translation_cli_tool;
  const isCustomCli = cliTool === "custom";
  const missingBaseUrl = !settings.base_url.trim();
  const missingTranslationModel = !settings.model.trim();
  const missingApiKey = !hasApiCredential;
  const missingCliCommand = !(settings.translation_cli_command ?? "").trim();
  const missingCliModel = !isCustomCli && !(settings.translation_cli_model ?? "").trim();
  const [cliStatus, setCliStatus] = useState<TranslationCliStatus | null>(null);
  const [cliChecking, setCliChecking] = useState(false);
  const [cliModels, setCliModels] = useState<string[]>([]);
  const [cliModelsLoading, setCliModelsLoading] = useState(false);
  const [cliNotice, setCliNotice] = useState("");
  const normalizedBaseUrl = settings.base_url.trim();
  const baseUrlEndpoint = normalizedBaseUrl
    ? settings.base_url_is_complete
      ? normalizedBaseUrl
      : `${normalizedBaseUrl.replace(/\/+$/, "")}/v1/chat/completions`
    : "-";
  const missingRequiredLabels = [
    missingWhisperModel ? t("requirement.missingWhisperModel") : "",
    ...(isCliProvider
      ? [
          missingCliCommand ? t("requirement.missingCliCommand") : "",
          missingCliModel ? t("requirement.missingCliModel") : "",
        ]
      : [
          missingBaseUrl ? t("requirement.missingBaseUrl") : "",
          missingTranslationModel ? t("requirement.missingTranslationModel") : "",
          missingApiKey ? t("requirement.missingApiKey") : "",
        ]),
  ].filter(Boolean);

  const handleCheckCli = async () => {
    if (!hasTauriRuntime()) {
      setCliNotice(t("notice.requireTauriConfig"));
      return;
    }
    setCliChecking(true);
    setCliNotice("");
    try {
      const status = await checkTranslationCli(
        settings.translation_cli_command || defaultSettings.translation_cli_command,
        cliTool,
      );
      setCliStatus(status);
      if (!status.available && status.error) setCliNotice(status.error);
    } catch (error) {
      setCliStatus(null);
      setCliNotice(errorText(error));
    } finally {
      setCliChecking(false);
    }
  };

  const handleLoadCliModels = async () => {
    if (!hasTauriRuntime()) {
      setCliNotice(t("notice.requireTauriConfig"));
      return;
    }
    setCliModelsLoading(true);
    setCliNotice("");
    try {
      const models = await listTranslationCliModels(
        settings.translation_cli_command || defaultSettings.translation_cli_command,
      );
      setCliModels(models);
      if (models.length === 0) setCliNotice(t("settings.cliModelsEmpty"));
    } catch (error) {
      setCliModels([]);
      setCliNotice(errorText(error));
    } finally {
      setCliModelsLoading(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <SectionTitle icon={<Settings />} title={t("settings.modelApi")} description={t("settings.modelApiDescription")} />
      </CardHeader>
      <CardContent className="settings-form">
        {missingRequiredLabels.length > 0 && (
          <Alert className="required-alert">
            <AlertCircle />
            <AlertTitle>{t("settings.requiredMissing")}</AlertTitle>
            <AlertDescription>
              {t("settings.requiredMissingDescription", {
                requirements: missingRequiredLabels.join(t("requirement.separator")),
              })}
            </AlertDescription>
          </Alert>
        )}

        <FieldBlock
          label={t("common.whisperModel")}
          invalid={missingWhisperModel}
          description={missingWhisperModel ? t("settings.requiredForTranscribe") : undefined}
        >
          <div className="input-action">
            <Input
              value={settings.whisper_model_path ? fileName(settings.whisper_model_path) : ""}
              readOnly
              placeholder={selectedWhisperPreset.fileName}
              onClick={onPickWhisperModel}
              title={settings.whisper_model_path || t("settings.selectWhisper")}
              aria-invalid={missingWhisperModel}
            />
            <IconAction label={t("settings.selectWhisper")} onClick={onPickWhisperModel}>
              <FolderOpen />
            </IconAction>
          </div>
        </FieldBlock>

        <FieldBlock label={t("model.preset")}>
          <div className="preset-control">
            <div className="input-action">
              <Select value={whisperPresetId} onValueChange={setWhisperPresetId}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {whisperModelPresets.map((preset) => {
                      const downloaded = downloadedWhisperModelFiles.has(preset.fileName);
                      return (
                        <SelectItem key={preset.id} value={preset.id}>
                          <span className="preset-option">
                            <span>{t(preset.labelKey)}</span>
                            {downloaded && (
                              <Badge variant="secondary" className="downloaded-badge">
                                <CheckCircle2 />
                                {t("model.downloaded")}
                              </Badge>
                            )}
                          </span>
                        </SelectItem>
                      );
                    })}
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
            {selectedPresetDownloaded && (
              <div className="preset-status">
                <CheckCircle2 />
                <span>{t("model.downloaded")}</span>
              </div>
            )}
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

        <FieldBlock
          label={t("settings.translationProvider")}
          description={t("settings.translationProviderDescription")}
        >
          <Select
            value={isCliProvider ? "cli" : "api"}
            onValueChange={(value) =>
              setSettings((current) => ({ ...current, translation_provider: value }))
            }
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                <SelectItem value="api">{t("settings.translationProviderApi")}</SelectItem>
                <SelectItem value="cli">{t("settings.translationProviderCli")}</SelectItem>
              </SelectGroup>
            </SelectContent>
          </Select>
        </FieldBlock>

        {!isCliProvider && (
          <>
            <div className="grid-two">
              <FieldBlock
                label="Base URL"
                invalid={missingBaseUrl}
                description={missingBaseUrl ? t("settings.requiredForTranslate") : undefined}
              >
                <Input
                  value={settings.base_url}
                  onChange={(event) => setSettings((current) => ({ ...current, base_url: event.target.value }))}
                  aria-invalid={missingBaseUrl}
                />
                <label className="checkbox-row">
                  <Checkbox
                    checked={settings.base_url_is_complete}
                    onCheckedChange={(checked) =>
                      setSettings((current) => ({ ...current, base_url_is_complete: checked === true }))
                    }
                  />
                  <span>{t("settings.baseUrlComplete")}</span>
                </label>
                <p className="field-hint">
                  {settings.base_url_is_complete
                    ? t("settings.baseUrlCompleteDescription", { endpoint: baseUrlEndpoint })
                    : t("settings.baseUrlAppendDescription", { endpoint: baseUrlEndpoint })}
                </p>
              </FieldBlock>
              <FieldBlock
                label={t("settings.translationModel")}
                invalid={missingTranslationModel}
                description={missingTranslationModel ? t("settings.requiredForTranslate") : undefined}
              >
                <Input
                  value={settings.model}
                  onChange={(event) => setSettings((current) => ({ ...current, model: event.target.value }))}
                  aria-invalid={missingTranslationModel}
                />
              </FieldBlock>
            </div>

            <div className="grid-two">
              <FieldBlock
                label="API Key"
                invalid={missingApiKey}
                description={missingApiKey ? t("settings.requiredForTranslate") : undefined}
              >
                <div className="key-field">
                  <KeyRound />
                  <Input
                    type="password"
                    value={apiKey}
                    onChange={(event) => setApiKey(event.target.value)}
                    placeholder={settings.has_api_key ? t("settings.apiKeySaved") : t("settings.apiKeyUnset")}
                    aria-invalid={missingApiKey}
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
          </>
        )}

        {isCliProvider && (
          <>
            <div className="grid-two">
              <FieldBlock label={t("settings.cliTool")}>
                <Select
                  value={cliTool}
                  onValueChange={(value) =>
                    setSettings((current) => ({ ...current, translation_cli_tool: value }))
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectItem value="opencode">{t("settings.cliToolOpencode")}</SelectItem>
                      <SelectItem value="custom">{t("settings.cliToolCustom")}</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </FieldBlock>
              <FieldBlock
                label={t("settings.cliCommand")}
                invalid={missingCliCommand}
                description={missingCliCommand ? t("settings.requiredForTranslate") : undefined}
              >
                <div className="input-action">
                  <Input
                    value={settings.translation_cli_command}
                    placeholder={t("settings.cliCommandPlaceholder")}
                    onChange={(event) =>
                      setSettings((current) => ({ ...current, translation_cli_command: event.target.value }))
                    }
                    aria-invalid={missingCliCommand}
                  />
                  <IconAction label={t("settings.cliCheck")} onClick={handleCheckCli} disabled={cliChecking}>
                    {cliChecking ? <Loader2 className="spin" /> : <Terminal />}
                  </IconAction>
                </div>
                {cliStatus && (
                  <p className="field-hint">
                    {cliStatus.available
                      ? `${t("settings.cliAvailable")}${cliStatus.version ? ` · ${cliStatus.version}` : ""}${cliStatus.path ? ` · ${cliStatus.path}` : ""}`
                      : `${t("settings.cliUnavailable")}${cliStatus.error ? ` · ${cliStatus.error}` : ""}`}
                  </p>
                )}
                {cliNotice && !cliStatus?.available && <p className="field-hint">{cliNotice}</p>}
              </FieldBlock>
            </div>

            {!isCustomCli && (
              <FieldBlock
                label={t("settings.cliModel")}
                invalid={missingCliModel}
                description={missingCliModel ? t("settings.requiredForTranslate") : undefined}
              >
                <div className="input-action">
                  <Input
                    value={settings.translation_cli_model}
                    placeholder={t("settings.cliModelPlaceholder")}
                    onChange={(event) =>
                      setSettings((current) => ({ ...current, translation_cli_model: event.target.value }))
                    }
                    aria-invalid={missingCliModel}
                  />
                  <IconAction
                    label={t("settings.cliLoadModels")}
                    onClick={handleLoadCliModels}
                    disabled={cliModelsLoading}
                  >
                    {cliModelsLoading ? <Loader2 className="spin" /> : <RefreshCw />}
                  </IconAction>
                </div>
                {cliModels.length > 0 && (
                  <Select
                    value={settings.translation_cli_model}
                    onValueChange={(value) =>
                      setSettings((current) => ({ ...current, translation_cli_model: value }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t("settings.cliModelPlaceholder")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {cliModels.map((model) => (
                          <SelectItem key={model} value={model}>
                            {model}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                )}
              </FieldBlock>
            )}

            {isCustomCli && (
              <>
                <FieldBlock label={t("settings.cliModel")}>
                  <Input
                    value={settings.translation_cli_model}
                    placeholder={t("settings.cliModelPlaceholder")}
                    onChange={(event) =>
                      setSettings((current) => ({ ...current, translation_cli_model: event.target.value }))
                    }
                  />
                </FieldBlock>
                <FieldBlock
                  label={t("settings.cliArgs")}
                  description={t("settings.cliArgsHint")}
                >
                  <Input
                    value={settings.translation_cli_args}
                    placeholder={t("settings.cliArgsPlaceholder")}
                    onChange={(event) =>
                      setSettings((current) => ({ ...current, translation_cli_args: event.target.value }))
                    }
                  />
                </FieldBlock>
              </>
            )}

            <Alert className="credential-alert warn">
              <AlertCircle />
              <AlertTitle>{t("settings.translationProviderCli")}</AlertTitle>
              <AlertDescription>{t("settings.cliNote")}</AlertDescription>
            </Alert>
          </>
        )}

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

        {!isCliProvider && (
          <Alert className={cn("credential-alert", hasApiCredential ? "ready" : "warn")}>
            {hasApiCredential ? <CheckCircle2 /> : <AlertCircle />}
            <AlertTitle>{hasApiCredential ? t("settings.apiReady") : t("settings.apiWarn")}</AlertTitle>
            <AlertDescription>
              {hasApiCredential ? t("settings.apiReadyDescription") : t("settings.apiWarnDescription")}
            </AlertDescription>
          </Alert>
        )}

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
        {!environmentReady && (
          <Alert className="required-alert">
            <AlertCircle />
            <AlertTitle>{t("env.requiredMissing")}</AlertTitle>
            <AlertDescription>{t("env.requiredMissingDescription")}</AlertDescription>
          </Alert>
        )}
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

export function QuickStartGuideCard({ t }: { t: Translate }) {
  const steps = [
    {
      title: t("guide.stepEnvironmentTitle"),
      body: t("guide.stepEnvironmentBody"),
    },
    {
      title: t("guide.stepModelTitle"),
      body: t("guide.stepModelBody"),
    },
    {
      title: t("guide.stepApiTitle"),
      body: t("guide.stepApiBody"),
    },
    {
      title: t("guide.stepRunTitle"),
      body: t("guide.stepRunBody"),
    },
  ];

  return (
    <Card className="guide-card">
      <CardHeader>
        <SectionTitle icon={<BookOpenCheck />} title={t("guide.section")} description={t("guide.description")} />
      </CardHeader>
      <CardContent>
        <ol className="guide-steps">
          {steps.map((step, index) => (
            <li key={step.title}>
              <span className="guide-index">{index + 1}</span>
              <div>
                <strong>{step.title}</strong>
                <p>{step.body}</p>
              </div>
            </li>
          ))}
        </ol>
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
