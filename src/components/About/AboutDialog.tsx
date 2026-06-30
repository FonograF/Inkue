import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { InkueMark } from "../common/InkueMark";

export function AboutDialog({ onClose }: { onClose: () => void }) {
  const [version, setVersion] = useState("…");

  useEffect(() => {
    void getVersion().then(setVersion).catch(() => setVersion("1.0.0"));
  }, []);

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 99999,
        background: "rgba(0,0,0,0.6)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 12, padding: "28px 32px", width: 420,
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
          <InkueMark size={28} />
          <span style={{ fontSize: 20, fontWeight: 700, color: "var(--wc-text-bright)" }}>Inkue</span>
          <span style={{ fontSize: 13, color: "var(--wc-text-muted)" }}>v{version}</span>
        </div>

        <div style={{ fontSize: 12, color: "var(--wc-text-secondary)", marginBottom: 20 }}>
          Professional show control — cross-platform, open source.
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6, fontSize: 12, color: "var(--wc-text-secondary)", marginBottom: 20 }}>
          <Row label="Built with" value="Tauri v2 · Rust · React · TypeScript" />
          <Row label="Audio"      value="cpal · symphonia" />
          <Row label="Video"      value="libmpv (OpenGL Render API)" />
          <Row label="DMX"        value="sACN E1.31 · Art-Net" />
        </div>

        <div
          style={{
            background: "rgba(255,255,255,0.04)", border: "1px solid var(--wc-border)",
            borderRadius: 6, padding: "10px 12px", fontSize: 11,
            color: "var(--wc-text-muted)", marginBottom: 20, lineHeight: 1.6,
          }}
        >
          <strong style={{ color: "var(--wc-text-secondary)" }}>Inkue</strong> is free software,
          released under the <strong>GNU General Public License v3</strong> (GPL-3.0-or-later).
          <br />
          <strong style={{ color: "var(--wc-text-secondary)" }}>libmpv</strong> is licensed under the{" "}
          <strong>GNU Lesser General Public License v2.1+</strong> (LGPL-2.1-or-later);
          Inkue loads it at runtime as an unmodified shared library (source: <span style={{ fontFamily: "monospace" }}>mpv.io</span>).
          <br />
          ASIO is a trademark and software of Steinberg Media Technologies GmbH.
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
            <a
              href="https://github.com/FonograF/Inkue"
              target="_blank"
              rel="noopener noreferrer"
              style={{ fontSize: 12, color: "var(--wc-text-secondary)", textDecoration: "none" }}
              onClick={(e) => e.stopPropagation()}
            >
              github.com/FonograF/Inkue
            </a>
            <a
              href="https://github.com/sponsors/FonograF"
              target="_blank"
              rel="noopener noreferrer"
              style={{ fontSize: 12, color: "var(--wc-accent)", textDecoration: "none", fontWeight: 600 }}
              onClick={(e) => e.stopPropagation()}
            >
              ♥ Sponsor
            </a>
          </div>
          <button
            onClick={onClose}
            style={{
              background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border-strong)",
              borderRadius: 6, color: "var(--wc-text)", cursor: "pointer",
              fontSize: 13, padding: "6px 18px",
            }}
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", gap: 8 }}>
      <span style={{ width: 70, flexShrink: 0, color: "var(--wc-text-faint)" }}>{label}</span>
      <span>{value}</span>
    </div>
  );
}
