/** Isolated QA entry; never imported by the shipping application. */
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";
import { realpathSync } from "node:fs";

const local = (path: string) => fileURLToPath(new URL(path, import.meta.url));
const projectRoot = local("../..");
export default defineConfig({
  root: local("."),
  plugins: [react(), tailwindcss()],
  resolve: { alias: [
    { find: "@/lib/tauri-api", replacement: local("./tauri-api.ts") },
    { find: "@/lib/updater", replacement: local("./updater.ts") },
    { find: "@tauri-apps/api/event", replacement: local("./events.ts") },
    { find: "@", replacement: local("../../src") },
  ] },
  server: { host: "127.0.0.1", port: 1433, strictPort: true, fs: { allow: [projectRoot, realpathSync(`${projectRoot}/node_modules`)] } },
  build: { outDir: local("./.preview-dist"), emptyOutDir: true },
});
