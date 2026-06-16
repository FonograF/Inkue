import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { PreferencesStandalone } from "./components/Preferences/PreferencesStandalone";
import { FloatTimerWindow } from "./windows/FloatTimer";

// Synchronously read window label from Tauri internals — no function call,
// no async, no crash if the object isn't present (e.g. pure browser dev).
const tauriLabel: string | undefined =
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).__TAURI_INTERNALS__?.metadata?.currentWindow?.label;

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <React.StrictMode>
    {tauriLabel === "preferences" ? (
      <PreferencesStandalone />
    ) : tauriLabel === "float-timer" ? (
      <FloatTimerWindow />
    ) : (
      <App />
    )}
  </React.StrictMode>
);
