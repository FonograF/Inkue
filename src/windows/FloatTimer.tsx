import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

export function FloatTimerWindow() {
    const [text, setText] = useState("--:--.---");

    useEffect(() => {
        const unlisten = listen<string>("float-timer-text", (event) => {
            setText(event.payload || "--:--.---");
        });
        return () => { unlisten.then((f) => f()); };
    }, []);

    return (
        <div
            data-tauri-drag-region
            style={{
                width: "100%",
                height: "100%",
                backgroundColor: "rgba(0, 0, 0, 0.85)",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                userSelect: "none",
                fontFamily: "monospace",
                fontSize: "3.2rem",
                fontWeight: "bold",
                color: "#ffffff",
                letterSpacing: "0.05em",
                borderRadius: "8px",
                boxSizing: "border-box",
                cursor: "move",
                // Subtle border so the window is visible against any background.
                outline: "1px solid rgba(255,255,255,0.12)",
            }}
        >
            {text}
        </div>
    );
}
