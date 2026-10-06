import { lazy, StrictMode, Suspense, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./index.css";

// Each window only loads the code it needs: the overlay and quick-capture
// webviews no longer pay for the whole main-window bundle at startup.
const App = lazy(() => import("./App"));
const QuickCapture = lazy(() => import("./quickcapture"));
const OverlayChat = lazy(() => import("./overlay"));
const TrayPanel = lazy(() => import("./tray-panel"));

// Keep the frontend previewable in an ordinary browser for layout and
// accessibility review; native commands remain best-effort outside Tauri.
const label = "__TAURI_INTERNALS__" in window ? getCurrentWindow().label : "main";

let root: ReactNode;
if (label === "quickcapture") {
  root = <QuickCapture />;
} else if (label === "overlay") {
  root = <OverlayChat />;
} else if (label === "tray-panel") {
  root = <TrayPanel />;
} else {
  root = <App />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Suspense fallback={null}>{root}</Suspense>
  </StrictMode>,
);
