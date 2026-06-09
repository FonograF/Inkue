import { getCurrentWindow } from "@tauri-apps/api/window";
import { PreferencesModal } from "./PreferencesModal";

export function PreferencesStandalone() {
  return (
    <PreferencesModal
      standalone
      onClose={() => void getCurrentWindow().hide()}
    />
  );
}
