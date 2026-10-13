/** Developer-only harness: actual React app and CSS, synthetic IPC boundary. */
import { useState } from "react";
import { createRoot } from "react-dom/client";
import App from "@/App";
import "./preview.css";

const params = new URLSearchParams(location.search);
const frame = params.has("frame");
Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
localStorage.setItem("luma-subtitle.locale", params.get("locale") || "zh-CN");
if (frame && !location.hash) {
  location.hash = params.get("page") === "settings" ? "/settings" : params.get("page") === "queue" ? "/tasks" : "/tasks/fixture";
}

function Harness() {
  const [size, setSize] = useState([Number(params.get("width") || 1100), Number(params.get("height") || 700)]);
  const scenario = params.get("scenario") || "completed";
  const locale = params.get("locale") || "zh-CN";
  const page = params.get("page") || "detail";
  const present = params.get("present") === "1";
  const query = (next: Record<string, string>) => `/?${new URLSearchParams({ width: String(size[0]), height: String(size[1]), scenario, locale, page, ...next })}`;
  return <div style={{ background: "#f4f5f7", color: "#606873", fontFamily: "sans-serif", minHeight: "100vh", width: "max-content", minWidth: "100%", padding: present ? 0 : 12 }}>
    <div style={{ width: size[0], marginInline: present ? "auto" : 0 }}>
      {present ? <div style={{ height: 28, padding: "0 12px", display: "flex", alignItems: "center", fontSize: 11 }}>luma Subtitle · 示例数据 / Sample data</div> : <header style={{ marginBottom: 10, color: "#111" }}>
        Developer QA · Production React components · Synthetic data only<br />
        {[[1280, 720], [1100, 700], [1024, 600], [760, 620], [1440, 1080]].map(([width, height]) => <button style={{ marginRight: 12 }} key={width} onClick={() => setSize([width, height])}>{width}×{height}</button>)}
        <a href={query({ locale: locale === "en-US" ? "zh-CN" : "en-US" })}>Switch language</a><br />
        {["completed", "running", "failed", "loading", "empty", "long"].map(value => <a style={{ marginRight: 12 }} href={query({ scenario: value })} key={value}>{value}</a>)}
      </header>}
      <iframe id="audit-frame" title="luma preview" src={`/?${new URLSearchParams({ frame: "1", scenario, locale, page })}`} style={{ width: size[0], height: size[1], border: 0, display: "block" }} />
    </div>
  </div>;
}

createRoot(document.getElementById("root")!).render(frame ? <App /> : <Harness />);
