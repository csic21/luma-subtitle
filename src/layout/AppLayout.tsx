import { Languages, ListTodo, Settings } from "lucide-react";
import { Link, Navigate, NavLink, Route, Routes } from "react-router-dom";

import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { localeOptions, type Locale, useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { SettingsPage } from "@/pages/SettingsPage";
import { TaskDetailPage } from "@/pages/TaskDetailPage";
import { TasksPage } from "@/pages/TasksPage";
import lumaLogoMark from "@/assets/luma-logo-mark.png";

export function AppLayout() {
  const { locale, setLocale, t } = useI18n();

  return (
    <main className="app-shell">
      <aside className="app-sidebar">
        <Link to="/tasks" className="brand-link" title="Luma Subtitle">
          <span className="brand-mark" aria-hidden="true">
            <img src={lumaLogoMark} alt="" />
          </span>
          <span className="brand-copy">
            <strong>Luma Subtitle</strong>
            <span>{t("app.tagline")}</span>
          </span>
        </Link>

        <nav className="app-nav" aria-label={t("app.tagline")}>
          <NavLink
            to="/tasks"
            className={({ isActive }) => cn("nav-item", isActive && "active")}
            title={t("task.queue")}
          >
            <ListTodo aria-hidden="true" />
            <span className="nav-label">{t("task.queue")}</span>
          </NavLink>
          <NavLink
            to="/settings"
            className={({ isActive }) => cn("nav-item", isActive && "active")}
            title={t("app.settings")}
          >
            <Settings aria-hidden="true" />
            <span className="nav-label">{t("app.settings")}</span>
          </NavLink>
        </nav>

        <footer className="sidebar-footer">
          <label className="sidebar-locale-label" htmlFor="app-locale">
            <Languages aria-hidden="true" />
            <span>{t("app.language")}</span>
          </label>
          <Select value={locale} onValueChange={(value) => setLocale(value as Locale)}>
            <SelectTrigger id="app-locale" className="locale-select" aria-label={t("app.language")}>
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
        </footer>
      </aside>

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
