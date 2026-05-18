import type { SettingsState, TargetLanguageOption, WhisperLanguageOption, ModelPresetView } from "@/types";

export const defaultSettings: SettingsState = {
  base_url: "https://api.openai.com",
  base_url_is_complete: false,
  model: "gpt-4o-mini",
  temperature: 0.2,
  translation_shard_size: 200,
  whisper_model_path: "",
  whisper_language: "auto",
  target_language: "简体中文",
  has_api_key: false,
};

export const languageOptions: TargetLanguageOption[] = [
  { value: "简体中文", labelKey: "language.simplifiedChinese" },
  { value: "繁体中文", labelKey: "language.traditionalChinese" },
  { value: "English", labelKey: "language.en" },
  { value: "日本語", labelKey: "language.ja" },
  { value: "한국어", labelKey: "language.ko" },
  { value: "Deutsch", labelKey: "language.de" },
  { value: "Français", labelKey: "language.fr" },
  { value: "Español", labelKey: "language.es" },
];

export const whisperLanguageOptions: WhisperLanguageOption[] = [
  { value: "auto", labelKey: "language.auto" },
  { value: "zh", labelKey: "language.chinese" },
  { value: "en", labelKey: "language.en" },
  { value: "ja", labelKey: "language.ja" },
  { value: "ko", labelKey: "language.ko" },
  { value: "de", labelKey: "language.de" },
  { value: "fr", labelKey: "language.fr" },
  { value: "es", labelKey: "language.es" },
  { value: "it", labelKey: "language.it" },
  { value: "pt", labelKey: "language.pt" },
  { value: "ru", labelKey: "language.ru" },
];

export const whisperModelPresets: ModelPresetView[] = [
  {
    id: "tiny",
    labelKey: "model.tiny",
    fileName: "ggml-tiny.bin",
  },
  {
    id: "base",
    labelKey: "model.base",
    fileName: "ggml-base.bin",
  },
  {
    id: "small",
    labelKey: "model.small",
    fileName: "ggml-small.bin",
  },
  {
    id: "large-v3-turbo-q5_0",
    labelKey: "model.largeTurbo",
    fileName: "ggml-large-v3-turbo-q5_0.bin",
  },
];

