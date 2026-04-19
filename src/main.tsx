import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { ImageSurface } from "./components/ImageSurface";

const label = getCurrentWindow().label;
const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

if (label.startsWith("image-surface-")) {
  const voiceId = label.replace("image-surface-", "");
  root.render(
    <React.StrictMode>
      <ImageSurface voiceId={voiceId} />
    </React.StrictMode>
  );
} else {
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
