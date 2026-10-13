import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
const read = (path: string) => readFileSync(path, "utf8");
describe("display-only luma branding", () => {
  it("uses lowercase display names without changing installed identities", () => {
    const native = JSON.parse(read("src-tauri/tauri.conf.json"));
    expect(native.app.windows[0].title).toBe("luma");
    expect(native.productName).toBe("Luma Subtitle");
    expect(native.identifier).toBe("com.luma.subtitle");
    expect(read("index.html")).toContain("<title>luma</title>");
    expect(read("src/layout/AppLayout.tsx")).toContain("<strong>luma</strong>");
    expect(JSON.parse(read("package.json")).name).toBe("luma-subtitle");
  });
  it("retains persistent locale, release assets and executable naming", () => {
    expect(read("src/i18n.tsx")).toContain("luma-subtitle.locale");
    expect(read(".github/workflows/release.yml")).toContain("Luma-Subtitle_[version]");
    expect(read("src-tauri/Cargo.toml")).toContain('name = "luma-subtitle"');
  });
});
