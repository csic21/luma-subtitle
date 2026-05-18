import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("SettingsPage layout", () => {
  it("keeps update settings in the left column stack to avoid grid row gaps", () => {
    const source = readFileSync(new URL("./SettingsPage.tsx", import.meta.url), "utf8");

    expect(source).toContain('className="settings-main-stack"');
    expect(source).toMatch(/<div className="settings-main-stack">[\s\S]*<ModelApiSettingsCard[\s\S]*<UpdateSettingsCard/);
  });
});
