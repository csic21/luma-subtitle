import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("SettingsPage layout", () => {
  it("keeps update settings in the left column stack to avoid grid row gaps", () => {
    const source = readFileSync(new URL("./SettingsPage.tsx", import.meta.url), "utf8");

    expect(source).toContain('className="settings-main-stack"');
    expect(source).toMatch(/<div className="settings-main-stack">[\s\S]*<ModelApiSettingsCard[\s\S]*<UpdateSettingsCard/);
  });

  it("keeps app navigation available when the sidebar collapses", () => {
    const layoutSource = readFileSync(new URL("../layout/AppLayout.tsx", import.meta.url), "utf8");
    const shellStyles = readFileSync(new URL("../styles/shell.css", import.meta.url), "utf8");
    const responsiveStyles = readFileSync(new URL("../styles/responsive.css", import.meta.url), "utf8");

    expect(layoutSource).toContain('className="app-sidebar"');
    expect(layoutSource).toContain('to="/settings"');
    expect(shellStyles).toMatch(/\.app-shell\s*{[\s\S]*grid-template-columns:\s*196px minmax\(0, 1fr\);/);
    expect(responsiveStyles).toMatch(
      /@media \(max-width:\s*860px\)\s*{[\s\S]*\.app-shell\s*{[\s\S]*grid-template-columns:\s*62px minmax\(0, 1fr\);/
    );
  });

  it("lets the quick start card fill the remaining side-column height on wide settings layouts", () => {
    const settingsStyles = readFileSync(new URL("../styles/settings.css", import.meta.url), "utf8");
    const responsiveStyles = readFileSync(new URL("../styles/responsive.css", import.meta.url), "utf8");

    expect(settingsStyles).toMatch(
      /@media \(min-width:\s*981px\)\s*{[\s\S]*\.settings-page-grid\s*{[\s\S]*align-items:\s*stretch;/
    );
    expect(settingsStyles).toMatch(/\.guide-card\s*{[\s\S]*flex:\s*1 1 auto;/);
    expect(settingsStyles).toMatch(/\.guide-card \[data-slot="card-content"\]\s*{[\s\S]*flex:\s*1;/);
    expect(responsiveStyles).toMatch(/\.settings-page-grid\s*{[\s\S]*align-items:\s*start;/);
  });
});
