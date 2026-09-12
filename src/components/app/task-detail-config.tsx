import { useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { FolderOpen, Loader2, RefreshCw, Save, Settings } from "lucide-react";

import { FieldBlock, IconAction, SectionTitle } from "@/components/app/shared";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defaultSettings, languageOptions, whisperLanguageOptions } from "@/config";
import type { useI18n } from "@/i18n";
import { errorText, hasTauriRuntime, taskBusy } from "@/lib/app-utils";
import { listTranslationCliModels } from "@/lib/tauri-api";
import { normalizeTaskSettings } from "@/lib/task-data";
import type { TaskRecord, TaskSettingsSnapshot } from "@/types";

type Translate = ReturnType<typeof useI18n>["t"];

export function TaskConfigCard({
  task,
  taskConfig,
  taskSettingsDirty,
  t,
  onApplyCurrentSettings,
  onPickWhisperModel,
  onSaveTaskSettings,
  setSettingsDraft,
}: {
  task: TaskRecord;
  taskConfig: TaskSettingsSnapshot;
  taskSettingsDirty: boolean;
  t: Translate;
  onApplyCurrentSettings: () => void | Promise<void>;
  onPickWhisperModel: () => void | Promise<void>;
  onSaveTaskSettings: () => void | Promise<void>;
  setSettingsDraft: Dispatch<SetStateAction<TaskSettingsSnapshot | null>>;
}) {
  const missingWhisperModel = !taskConfig.whisper_model_path.trim();
  const missingBaseUrl = !taskConfig.base_url.trim();
  const missingTranslationModel = !taskConfig.model.trim();
  const normalizedProvider = taskConfig.translation_provider ?? defaultSettings.translation_provider;
  const isCli = normalizedProvider === "cli";
  const cliTool = taskConfig.translation_cli_tool ?? defaultSettings.translation_cli_tool;
  const isCustomCli = cliTool === "custom";
  const missingCliCommand = !(taskConfig.translation_cli_command ?? "").trim();
  const missingCliModel = !isCustomCli && !(taskConfig.translation_cli_model ?? "").trim();
  const normalizedBaseUrl = taskConfig.base_url.trim();
  const baseUrlEndpoint = normalizedBaseUrl
    ? taskConfig.base_url_is_complete
      ? normalizedBaseUrl
      : `${normalizedBaseUrl.replace(/\/+$/, "")}/v1/chat/completions`
    : "-";
  const [cliModels, setCliModels] = useState<string[]>([]);
  const [cliModelsLoading, setCliModelsLoading] = useState(false);
  const [cliNotice, setCliNotice] = useState("");

  const handleLoadCliModels = async () => {
    if (!hasTauriRuntime()) {
      setCliNotice(t("notice.requireTauriConfig"));
      return;
    }
    setCliModelsLoading(true);
    setCliNotice("");
    try {
      const models = await listTranslationCliModels(
        taskConfig.translation_cli_command || defaultSettings.translation_cli_command,
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
        <SectionTitle icon={<Settings />} title={t("tabs.taskConfig")} />
        <CardAction>
          <Button
            variant="secondary"
            size="sm"
            onClick={onApplyCurrentSettings}
            disabled={taskBusy(task)}
            title={t("settings.applyGlobalTitle")}
          >
            <RefreshCw data-icon="inline-start" />
            {t("settings.applyGlobal")}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="settings-form">
        <FieldBlock
          label={t("common.whisperModel")}
          invalid={missingWhisperModel}
          description={missingWhisperModel ? t("settings.requiredForTranscribe") : undefined}
        >
          <div className="input-action">
            <Input
              value={taskConfig.whisper_model_path}
              onChange={(event) =>
                setSettingsDraft((current) =>
                  current ? { ...current, whisper_model_path: event.target.value } : current,
                )
              }
              disabled={taskBusy(task)}
              placeholder={t("settings.notSet")}
              title={taskConfig.whisper_model_path || t("settings.selectWhisper")}
              aria-invalid={missingWhisperModel}
            />
            <IconAction label={t("settings.selectWhisper")} onClick={onPickWhisperModel} disabled={taskBusy(task)}>
              <FolderOpen />
            </IconAction>
          </div>
        </FieldBlock>

        <div className="grid-two">
          <FieldBlock label={t("settings.sourceLanguage")}>
            <Select
              value={taskConfig.whisper_language || "auto"}
              onValueChange={(value) =>
                setSettingsDraft((current) => (current ? { ...current, whisper_language: value } : current))
              }
              disabled={taskBusy(task)}
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
              value={taskConfig.target_language}
              onValueChange={(value) =>
                setSettingsDraft((current) => (current ? { ...current, target_language: value } : current))
              }
              disabled={taskBusy(task)}
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
            value={isCli ? "cli" : "api"}
            onValueChange={(value) =>
              setSettingsDraft((current) => (current ? { ...current, translation_provider: value } : current))
            }
            disabled={taskBusy(task)}
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

        {!isCli && (
          <div className="grid-two">
            <FieldBlock
              label="Base URL"
              invalid={missingBaseUrl}
              description={missingBaseUrl ? t("settings.requiredForTranslate") : undefined}
            >
              <Input
                value={taskConfig.base_url}
                onChange={(event) =>
                  setSettingsDraft((current) => (current ? { ...current, base_url: event.target.value } : current))
                }
                disabled={taskBusy(task)}
                aria-invalid={missingBaseUrl}
              />
              <label className="checkbox-row">
                <Checkbox
                  checked={taskConfig.base_url_is_complete}
                  onCheckedChange={(checked) =>
                    setSettingsDraft((current) =>
                      current ? { ...current, base_url_is_complete: checked === true } : current,
                    )
                  }
                  disabled={taskBusy(task)}
                />
                <span>{t("settings.baseUrlComplete")}</span>
              </label>
              <p className="field-hint">
                {taskConfig.base_url_is_complete
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
                value={taskConfig.model}
                onChange={(event) =>
                  setSettingsDraft((current) => (current ? { ...current, model: event.target.value } : current))
                }
                disabled={taskBusy(task)}
                aria-invalid={missingTranslationModel}
              />
            </FieldBlock>
          </div>
        )}

        {isCli && (
          <>
            <div className="grid-two">
              <FieldBlock label={t("settings.cliTool")}>
                <Select
                  value={cliTool}
                  onValueChange={(value) =>
                    setSettingsDraft((current) =>
                      current ? { ...current, translation_cli_tool: value } : current,
                    )
                  }
                  disabled={taskBusy(task)}
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
                <Input
                  value={taskConfig.translation_cli_command ?? ""}
                  placeholder={t("settings.cliCommandPlaceholder")}
                  onChange={(event) =>
                    setSettingsDraft((current) =>
                      current ? { ...current, translation_cli_command: event.target.value } : current,
                    )
                  }
                  disabled={taskBusy(task)}
                  aria-invalid={missingCliCommand}
                />
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
                    value={taskConfig.translation_cli_model ?? ""}
                    placeholder={t("settings.cliModelPlaceholder")}
                    onChange={(event) =>
                      setSettingsDraft((current) =>
                        current ? { ...current, translation_cli_model: event.target.value } : current,
                      )
                    }
                    disabled={taskBusy(task)}
                    aria-invalid={missingCliModel}
                  />
                  <IconAction
                    label={t("settings.cliLoadModels")}
                    onClick={handleLoadCliModels}
                    disabled={taskBusy(task) || cliModelsLoading}
                  >
                    {cliModelsLoading ? <Loader2 className="spin" /> : <RefreshCw />}
                  </IconAction>
                </div>
                {cliModels.length > 0 && (
                  <Select
                    value={taskConfig.translation_cli_model ?? ""}
                    onValueChange={(value) =>
                      setSettingsDraft((current) =>
                        current ? { ...current, translation_cli_model: value } : current,
                      )
                    }
                    disabled={taskBusy(task)}
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
                {cliNotice && <p className="field-hint">{cliNotice}</p>}
              </FieldBlock>
            )}
            {isCustomCli && (
              <>
                <FieldBlock label={t("settings.cliModel")}>
                  <Input
                    value={taskConfig.translation_cli_model ?? ""}
                    placeholder={t("settings.cliModelPlaceholder")}
                    onChange={(event) =>
                      setSettingsDraft((current) =>
                        current ? { ...current, translation_cli_model: event.target.value } : current,
                      )
                    }
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
                <FieldBlock label={t("settings.cliArgs")} description={t("settings.cliArgsHint")}>
                  <Input
                    value={taskConfig.translation_cli_args ?? ""}
                    placeholder={t("settings.cliArgsPlaceholder")}
                    onChange={(event) =>
                      setSettingsDraft((current) =>
                        current ? { ...current, translation_cli_args: event.target.value } : current,
                      )
                    }
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
              </>
            )}
          </>
        )}

        <div className="grid-two">
          <FieldBlock label="Temperature">
            <Input
              type="number"
              min="0"
              max="1"
              step="0.1"
              value={taskConfig.temperature}
              onChange={(event) =>
                setSettingsDraft((current) =>
                  current ? { ...current, temperature: Number.parseFloat(event.target.value) || 0 } : current,
                )
              }
              disabled={taskBusy(task)}
            />
          </FieldBlock>
          <FieldBlock label={t("settings.shardSize")}>
            <Input
              type="number"
              min="1"
              max="1000"
              step="1"
              value={taskConfig.translation_shard_size ?? defaultSettings.translation_shard_size}
              onChange={(event) =>
                setSettingsDraft((current) =>
                  current
                    ? {
                        ...current,
                        translation_shard_size:
                          Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
                      }
                    : current,
                )
              }
              disabled={taskBusy(task)}
            />
          </FieldBlock>
        </div>

        <div className="action-row end">
          <Button
            variant="secondary"
            onClick={() => setSettingsDraft(normalizeTaskSettings(task.settings))}
            disabled={taskBusy(task) || !taskSettingsDirty}
          >
            {t("settings.undo")}
          </Button>
          <Button onClick={onSaveTaskSettings} disabled={taskBusy(task) || !taskSettingsDirty}>
            <Save data-icon="inline-start" />
            {t("settings.saveTask")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
