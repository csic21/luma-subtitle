import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("SettingsPage layout", () => {
  it("prioritizes models with named persistent environment and update panels", () => {
    const source = readFileSync(new URL("./SettingsPage.tsx", import.meta.url), "utf8");
    expect(source).toContain('useState("models")');
    expect(source).toContain('<TabsList variant="line" aria-label={t("app.settings")}>');
    for (const name of ["models", "environment", "updates"]) {
      expect(source).toContain(`<TabsTrigger value="${name}">`);
      expect(source).toContain(`<TabsContent value="${name}" forceMount hidden={section !== "${name}"}`);
    }
    expect(source).toMatch(/value="models" forceMount[\s\S]*<ModelApiSettingsCard/);
    expect(source).toMatch(/value="environment" forceMount[\s\S]*<EnvironmentSettingsCard[\s\S]*<QuickStartGuideCard/);
    expect(source).toMatch(/value="updates" forceMount[\s\S]*<UpdateSettingsCard/);
  });

  it("keeps named app navigation available when the sidebar collapses", () => {
    const layout = readFileSync(new URL("../layout/AppLayout.tsx", import.meta.url), "utf8");
    const shell = readFileSync(new URL("../styles/shell.css", import.meta.url), "utf8");
    const responsive = readFileSync(new URL("../styles/responsive.css", import.meta.url), "utf8");
    expect(layout).toContain('className="app-sidebar"');
    expect(layout).toContain('to="/settings"');
    expect(layout).toContain('aria-label={t("app.settings")}');
    expect(layout).toContain('aria-label={t("task.queue")}');
    expect(shell).toMatch(/\.app-shell\s*{[^}]*grid-template-columns:\s*164px minmax\(0, 1fr\);/);
    expect(responsive).toMatch(/@media \(max-width:\s*860px\)\s*{[\s\S]*\.app-shell\s*{[^}]*grid-template-columns:\s*62px minmax\(0, 1fr\);/);
  });

  it("keeps Save and tabs outside the bounded settings scroller", () => {
    const source = readFileSync(new URL("./SettingsPage.tsx", import.meta.url), "utf8");
    const styles = readFileSync(new URL("../styles/settings.css", import.meta.url), "utf8");
    expect(styles).toMatch(/\.settings-workspace\s*{[^}]*min-height:\s*0;[^}]*overflow:\s*hidden;/);
    expect(styles).toMatch(/\.settings-scroll-area\s*{[^}]*min-height:\s*0;[^}]*overflow:\s*auto;/);
    expect(styles).toMatch(/\.settings-panel\[hidden\]\s*{\s*display:\s*none;/);
    expect(source).toMatch(/<header className="settings-page-heading">[\s\S]*saveSettings\(\)[\s\S]*<\/header>[\s\S]*className="settings-scroll-area"/);
    expect(source).toContain('t("settings.description")');
    expect(styles).toMatch(/@media \(max-width:\s*680px\)[\s\S]*grid-template-columns:\s*minmax\(0, 1fr\)/);
  });
});
