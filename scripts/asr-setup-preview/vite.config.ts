/** Isolated developer-only entry. Never imported by the shipping application. */
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";
import { realpathSync } from "node:fs";
const projectRoot = fileURLToPath(new URL("../..", import.meta.url));

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  plugins: [react(), tailwindcss()],
  resolve: { alias: [
    { find: "@/lib/tauri-api", replacement: fileURLToPath(new URL("./tauri-api.ts", import.meta.url)) },
    { find: "@tauri-apps/api/event", replacement: fileURLToPath(new URL("./events.ts", import.meta.url)) },
    { find: "@", replacement: fileURLToPath(new URL("../../src", import.meta.url)) },
  ] },
  server: { host: "127.0.0.1", port: 1427, strictPort: true, fs: { allow: [projectRoot, realpathSync(`${projectRoot}/node_modules`)] } },
  build: { outDir: fileURLToPath(new URL("./.preview-dist", import.meta.url)), emptyOutDir: true },
});
