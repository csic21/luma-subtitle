import { enUS } from "./en-US";
import { zhCN } from "./zh-CN";

export const translations = {
  "zh-CN": zhCN,
  "en-US": enUS,
} as const;

export type TranslationKey = keyof typeof zhCN;
