import { ArrowLeft } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { NoticeAlert } from "@/components/app/shared";
import {
  EnvironmentSettingsCard,
  ModelApiSettingsCard,
  QuickStartGuideCard,
  UpdateSettingsCard,
} from "@/components/app/settings";
import { Button } from "@/components/ui/button";
import { useSettingsPageState } from "@/hooks/use-settings-page-state";
import { useI18n } from "@/i18n";

export function SettingsPage() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const {
    apiKey,
    appUpdate,
    appUpdating,
    checkForUpdates,
    dependencyInstall,
    dependencyInstalling,
    downloadedWhisperModelFiles,
    downloadWhisperPreset,
    env,
    environmentReady,
    envRows,
    hasApiCredential,
    installUpdate,
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
  } = useSettingsPageState(t);

  return (
    <>
      <section className="page-heading">
        <Button variant="secondary" size="sm" onClick={() => navigate("/tasks")}>
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
        <div className="settings-main-stack">
          <ModelApiSettingsCard
            apiKey={apiKey}
            hasApiCredential={hasApiCredential}
            modelDownload={modelDownload}
            modelDownloading={modelDownloading}
            downloadedWhisperModelFiles={downloadedWhisperModelFiles}
            selectedWhisperPreset={selectedWhisperPreset}
            settings={settings}
            t={t}
            whisperPresetId={whisperPresetId}
            onDownloadWhisperPreset={downloadWhisperPreset}
            onPickWhisperModel={pickWhisperModel}
            onSaveSettings={() => saveSettings()}
            setApiKey={setApiKey}
            setSettings={setSettings}
            setWhisperPresetId={setWhisperPresetId}
          />

          <UpdateSettingsCard
            appUpdate={appUpdate}
            appUpdating={appUpdating}
            t={t}
            onCheckForUpdates={checkForUpdates}
            onInstallUpdate={installUpdate}
          />
        </div>

        <div className="settings-side-stack">
          <EnvironmentSettingsCard
            dependencyInstall={dependencyInstall}
            dependencyInstalling={dependencyInstalling}
            environmentReady={environmentReady}
            env={env}
            envRows={envRows}
            t={t}
            onInstallDependencies={installDependencies}
            onOpenManagedDir={openManagedDir}
            onRefreshEnvironment={refreshEnvironment}
          />
          <QuickStartGuideCard t={t} />
        </div>
      </section>
    </>
  );
}
