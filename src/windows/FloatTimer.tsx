import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

export function FloatTimerWindow() {
    const [text, setText]   = useState("--:--.---");
    const [font, setFont]   = useState("DSEG7 Classic");

    useEffect(() => {
        const unlistenText = listen<string>("float-timer-text", (e) => {
            setText(e.payload || "--:--.---");
        });
        const unlistenFont = listen<string>("float-timer-font", (e) => {
            if (e.payload) setFont(e.payload);
        });
        return () => {
            unlistenText.then((f) => f());
            unlistenFont.then((f) => f());
        };
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
                fontFamily: `"${font}", monospace`,
                fontSize: "3.2rem",
                fontWeight: "bold",
                color: "#ffffff",
                letterSpacing: "0.05em",
                borderRadius: "8px",
                boxSizing: "border-box",
                cursor: "move",
                outline: "1px solid rgba(255,255,255,0.12)",
            }}
        >
            {text}
        </div>
    );
}
