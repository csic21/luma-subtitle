import { configDefaults, defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(viteConfig, defineConfig({
  test: {
    // Release scripts use node:test and run separately in CI.
    exclude: [
      ...configDefaults.exclude,
      "scripts/prepare-release.test.cjs",
      "scripts/publish-release.test.cjs",
    ],
  },
}));
