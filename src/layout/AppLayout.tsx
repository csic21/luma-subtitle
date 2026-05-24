import { Settings } from "lucide-react";
import { Link, Navigate, Route, Routes } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { localeOptions, type Locale, useI18n } from "@/i18n";
import { SettingsPage } from "@/pages/SettingsPage";
import { TaskDetailPage } from "@/pages/TaskDetailPage";
import { TasksPage } from "@/pages/TasksPage";
import lumaLogoMark from "@/assets/luma-logo-mark.png";

export function AppLayout() {
  const { locale, setLocale, t } = useI18n();

  return (
    <main className="app-shell">
      <header className="topbar">
        <Link to="/tasks" className="brand-link">
          <span className="brand-mark" aria-hidden="true">
            <img src={lumaLogoMark} alt="" />
          </span>
          <span className="brand-copy">
            <strong>Luma Subtitle</strong>
            <span>{t("app.tagline")}</span>
          </span>
        </Link>
        <div className="topbar-actions">
          <Select value={locale} onValueChange={(value) => setLocale(value as Locale)}>
            <SelectTrigger className="locale-select" aria-label={t("app.language")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {localeOptions.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
          <Button asChild variant="secondary" size="icon" className="topbar-settings-button" title={t("app.settings")}>
            <Link to="/settings" aria-label={t("app.settings")}>
              <Settings />
            </Link>
          </Button>
        </div>
      </header>

      <section className="app-content">
        <Routes>
          <Route path="/" element={<Navigate to="/tasks" replace />} />
          <Route path="/tasks" element={<TasksPage />} />
          <Route path="/tasks/:taskId" element={<TaskDetailPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/tasks" replace />} />
        </Routes>
      </section>
    </main>
  );
}
