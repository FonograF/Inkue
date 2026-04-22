import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { OutputSurface } from "./components/ImageSurface";

const label = getCurrentWindow().label;
const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

if (label.startsWith("output-surface-")) {
  root.render(
    <React.StrictMode>
      <OutputSurface />
    </React.StrictMode>
  );
} else {
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
