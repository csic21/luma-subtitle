import { ArrowLeft } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { NoticeAlert } from "@/components/app/shared";
import { EnvironmentSettingsCard, ModelApiSettingsCard } from "@/components/app/settings";
import { Button } from "@/components/ui/button";
import { useSettingsPageState } from "@/hooks/use-settings-page-state";
import { useI18n } from "@/i18n";

export function SettingsPage() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const {
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
  } = useSettingsPageState(t);

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
        <ModelApiSettingsCard
          apiKey={apiKey}
          hasApiCredential={hasApiCredential}
          modelDownload={modelDownload}
          modelDownloading={modelDownloading}
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
      </section>
    </>
  );
}
