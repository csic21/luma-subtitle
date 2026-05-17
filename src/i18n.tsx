import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import i18n from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import { initReactI18next } from "react-i18next";

import { translations } from "@/locales";

export type Locale = "zh-CN" | "en-US";

type TranslationValues = Record<string, string | number>;

const STORAGE_KEY = "luma-subtitle.locale";

export const localeOptions: Array<{ value: Locale; label: string }> = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en-US", label: "English" },
];

function normalizeLocale(value: string | null | undefined): Locale {
  if (value === "en-US" || value?.toLowerCase().startsWith("en")) return "en-US";
  return "zh-CN";
}

function interpolate(template: string, values?: TranslationValues) {
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (match, key) =>
    values[key] === undefined ? match : String(values[key]),
  );
}

function translate(key: string, values: TranslationValues | undefined, locale: Locale) {
  const translated = String(i18n.t(key, values));
  if (translated !== key) return translated;
  const localeTranslations = translations[locale] as Record<string, string>;
  const fallbackTranslations = translations["zh-CN"] as Record<string, string>;
  const template = localeTranslations[key] ?? fallbackTranslations[key] ?? key;
  return interpolate(template, values);
}

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      "zh-CN": { translation: translations["zh-CN"] },
      "en-US": { translation: translations["en-US"] },
    },
    fallbackLng: "zh-CN",
    supportedLngs: ["zh-CN", "en-US"],
    nonExplicitSupportedLngs: true,
    keySeparator: false,
    initAsync: false,
    detection: {
      order: ["localStorage", "navigator"],
      lookupLocalStorage: STORAGE_KEY,
      caches: ["localStorage"],
    },
    interpolation: {
      escapeValue: false,
      prefix: "{",
      suffix: "}",
    },
    react: {
      useSuspense: false,
    },
  });

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState(() => normalizeLocale(i18n.resolvedLanguage ?? i18n.language));

  useEffect(() => {
    const updateLocale = () => {
      setLocale(normalizeLocale(i18n.resolvedLanguage ?? i18n.language));
    };

    i18n.on("languageChanged", updateLocale);
    i18n.on("loaded", updateLocale);
    updateLocale();

    return () => {
      i18n.off("languageChanged", updateLocale);
      i18n.off("loaded", updateLocale);
    };
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  return children;
}

export function useI18n() {
  const [locale, setLocaleState] = useState(() => normalizeLocale(i18n.resolvedLanguage ?? i18n.language));

  useEffect(() => {
    const updateLocale = () => {
      setLocaleState(normalizeLocale(i18n.resolvedLanguage ?? i18n.language));
    };

    i18n.on("languageChanged", updateLocale);
    i18n.on("loaded", updateLocale);
    updateLocale();

    return () => {
      i18n.off("languageChanged", updateLocale);
      i18n.off("loaded", updateLocale);
    };
  }, []);

  return {
    locale,
    setLocale: (nextLocale: Locale) => {
      void i18n.changeLanguage(nextLocale);
    },
    t: (key: string, values?: TranslationValues) => translate(key, values, locale),
  };
}
