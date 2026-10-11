import React, { useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { ManagedAsrSetup } from "@/components/app/managed-asr-setup";
import { translations } from "@/locales";
import type { AsrConfig, TFunction } from "@/types";
import { activeId, calls, failCurrent, finishCurrent, getRevision, scenario, subscribe } from "./tauri-api";
import "./preview.css";

Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
const params = new URLSearchParams(location.search);
const width = params.get("width") === "400" ? 400 : 1100;
const frame = params.has("frame");
const locale = params.get("locale") === "en-US" ? "en-US" : "zh-CN";
const t: TFunction = (key, values) => {
  const message = (translations[locale] as Record<string, string>)[key] ?? key;
  return message.replace(/\{(\w+)\}/g, (match, name: string) => String(values?.[name] ?? match));
};
function Preview() {
  const [config, setConfig] = useState<AsrConfig>({ engine: "qwen3-asr", python_path: "", model_path: "", aligner_path: "", device: "cpu" });
  useSyncExternalStore(subscribe, getRevision);
  return <>
    <header className="fixture-label">Developer QA fixture · Memory-only · No downloads</header>
    <main className="fixture-surface">
      <h1 className="fixture-title">{locale === "zh-CN" ? "本地语音识别" : "Local speech recognition"}</h1>
      <ManagedAsrSetup config={config} onChange={setConfig} disabled={false} t={t} />
    </main>
    <aside className="fixture-controls" aria-label="Developer fixture controls">
      <strong>Developer controls (not shipping UI)</strong>
      <div><button type="button" onClick={finishCurrent} disabled={!activeId()}>Complete simulated step</button><button type="button" onClick={failCurrent} disabled={!activeId()}>Fail simulated step</button></div>
      <output data-testid="fixture-active">Active: {activeId() ?? "none"}</output>
      <output data-testid="fixture-calls">{calls.length ? calls.join("\n") : "No simulated install commands"}</output>
      <output data-testid="fixture-config">{JSON.stringify(config)}</output>
    </aside>
  </>;
}
function Harness() {
  const source = `/?frame=1&scenario=${encodeURIComponent(scenario)}&locale=${locale}`;
  return <div className="fixture-harness">
    <nav className="fixture-nav" aria-label="Preview scenarios"><strong>ASR setup developer fixture</strong>{["runtime-installed", "fresh", "ready"].map((name) => <a key={name} href={`/?scenario=${name}&width=${width}&locale=${locale}`}>{name}</a>)}<a href={`/?scenario=${scenario}&width=${width === 400 ? 1100 : 400}&locale=${locale}`}>Width {width === 400 ? 1100 : 400}</a></nav>
    <iframe id="fixture-frame" title={`Real ManagedAsrSetup at ${width}px`} src={source} style={{ width, height: 800 }} />
  </div>;
}
createRoot(document.getElementById("root")!).render(frame ? <Preview /> : <Harness />);
