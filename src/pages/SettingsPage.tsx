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
    downloadedTranslationModelFiles,
    downloadedWhisperModelFiles,
    downloadTranslationPreset,
    downloadWhisperPreset,
    env,
    environmentReady,
    envRows,
    hasApiCredential,
    installUpdate,
    installDependencies,
    installLocalTranslation,
    llamaReady,
    modelDownload,
    modelDownloading,
    notice,
    openManagedDir,
    pickTranslationModel,
    pickWhisperModel,
    refreshEnvironment,
    saveSettings,
    selectedTranslationPreset,
    selectedWhisperPreset,
    setApiKey,
    setSettings,
    setTranslationPresetId,
    setWhisperPresetId,
    settings,
    translationPresetId,
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
            downloadedTranslationModelFiles={downloadedTranslationModelFiles}
            downloadedWhisperModelFiles={downloadedWhisperModelFiles}
            llamaBackend={env?.llama_backend}
            llamaInstalling={dependencyInstalling}
            llamaReady={llamaReady}
            selectedTranslationPreset={selectedTranslationPreset}
            selectedWhisperPreset={selectedWhisperPreset}
            settings={settings}
            t={t}
            translationPresetId={translationPresetId}
            whisperPresetId={whisperPresetId}
            onDownloadTranslationPreset={downloadTranslationPreset}
            onDownloadWhisperPreset={downloadWhisperPreset}
            onInstallLocalTranslation={installLocalTranslation}
            onPickTranslationModel={pickTranslationModel}
            onPickWhisperModel={pickWhisperModel}
            onSaveSettings={() => saveSettings()}
            setApiKey={setApiKey}
            setSettings={setSettings}
            setTranslationPresetId={setTranslationPresetId}
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
