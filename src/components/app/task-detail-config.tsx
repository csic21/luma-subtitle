import type { Dispatch, SetStateAction } from "react";
import { FolderOpen, RefreshCw, Save, Settings } from "lucide-react";

import { FieldBlock, IconAction, SectionTitle } from "@/components/app/shared";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defaultSettings, languageOptions, whisperLanguageOptions } from "@/config";
import type { useI18n } from "@/i18n";
import { taskBusy } from "@/lib/app-utils";
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
        <FieldBlock label={t("common.whisperModel")}>
          <div className="input-action">
            <Input
              value={taskConfig.whisper_model_path}
              onChange={(event) => setSettingsDraft({ ...taskConfig, whisper_model_path: event.target.value })}
              disabled={taskBusy(task)}
              placeholder={t("settings.notSet")}
              title={taskConfig.whisper_model_path || t("settings.selectWhisper")}
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
              onValueChange={(value) => setSettingsDraft({ ...taskConfig, whisper_language: value })}
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
              onValueChange={(value) => setSettingsDraft({ ...taskConfig, target_language: value })}
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

        <div className="grid-two">
          <FieldBlock label="Base URL">
            <Input
              value={taskConfig.base_url}
              onChange={(event) => setSettingsDraft({ ...taskConfig, base_url: event.target.value })}
              disabled={taskBusy(task)}
            />
          </FieldBlock>
          <FieldBlock label={t("settings.translationModel")}>
            <Input
              value={taskConfig.model}
              onChange={(event) => setSettingsDraft({ ...taskConfig, model: event.target.value })}
              disabled={taskBusy(task)}
            />
          </FieldBlock>
        </div>

        <div className="grid-two">
          <FieldBlock label="Temperature">
            <Input
              type="number"
              min="0"
              max="1"
              step="0.1"
              value={taskConfig.temperature}
              onChange={(event) =>
                setSettingsDraft({ ...taskConfig, temperature: Number.parseFloat(event.target.value) || 0 })
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
                setSettingsDraft({
                  ...taskConfig,
                  translation_shard_size:
                    Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
                })
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
