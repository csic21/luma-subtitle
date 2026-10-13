import { useState } from "react";
import { ArrowLeft, Save } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { NoticeAlert } from "@/components/app/shared";
import {
  EnvironmentSettingsCard,
  ModelApiSettingsCard,
  QuickStartGuideCard,
  UpdateSettingsCard,
} from "@/components/app/settings";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useSettingsPageState } from "@/hooks/use-settings-page-state";
import { useI18n } from "@/i18n";

export function SettingsPage() {
  const navigate = useNavigate();
  const [section, setSection] = useState("models");
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
    <div className="settings-workspace">
      <header className="settings-page-heading">
        <Button variant="ghost" size="icon-sm" onClick={() => navigate("/tasks")} aria-label={t("common.backToQueue")} title={t("common.backToQueue")}>
          <ArrowLeft />
        </Button>
        <div className="settings-heading-copy">
          <h1>{t("app.settings")}</h1>
          <p>{t("settings.description")}</p>
        </div>
        <Button size="sm" onClick={() => saveSettings()}>
          <Save data-icon="inline-start" />
          {t("common.save")}
        </Button>
      </header>

      <Tabs value={section} onValueChange={setSection} className="settings-tabs">
        <TabsList variant="line" aria-label={t("app.settings")}>
          <TabsTrigger value="models">{t("settings.modelApi")}</TabsTrigger>
          <TabsTrigger value="environment">
            {t("env.section")}
            {!environmentReady && <span className="settings-attention-dot" role="img" aria-label={t("env.warn")} />}
          </TabsTrigger>
          <TabsTrigger value="updates">{t("update.section")}</TabsTrigger>
        </TabsList>
        <div className="settings-scroll-area">
          <NoticeAlert message={notice} />
          <TabsContent value="models" forceMount hidden={section !== "models"} className="settings-panel">
            <ModelApiSettingsCard
              showSaveAction={false}
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
          </TabsContent>
          <TabsContent value="environment" forceMount hidden={section !== "environment"} className="settings-panel">
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
          </TabsContent>
          <TabsContent value="updates" forceMount hidden={section !== "updates"} className="settings-panel">
            <UpdateSettingsCard
              appUpdate={appUpdate}
              appUpdating={appUpdating}
              t={t}
              onCheckForUpdates={checkForUpdates}
              onInstallUpdate={installUpdate}
            />
          </TabsContent>
        </div>
      </Tabs>
    </div>
  );
}
