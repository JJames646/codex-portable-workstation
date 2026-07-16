import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";
// 已确认的网页原型是工作站界面的最终视觉来源，必须最后加载，
// 避免被 Codex++ 上游全局样式再次覆盖。
import "./features/workstation/prototype-fidelity.css";

/* ── Bundled fonts (offline, no Google Fonts request) ──
     Fontsource packages ship woff2 files that Vite bundles into dist/.
     CSS @font-face declarations are injected at build time.              */
import "@fontsource/jetbrains-mono";

const app = document.getElementById("app");

if (app instanceof HTMLElement) {
  createRoot(app).render(<App />);
}
