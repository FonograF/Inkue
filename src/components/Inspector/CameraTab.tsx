// Camera tab: pick the live source — a connected capture device (webcam,
// USB camera, HDMI capture card) or a network stream URL (IP camera, phone
// camera app via RTSP/HTTP).

import { useEffect, useState } from "react";
import type { CameraCueData, CameraDeviceInfo, CameraSource } from "../../lib/types";
import { listCameraDevices } from "../../lib/commands";
import { Field, inputStyle } from "./Field";
import { Select } from "../common/Select";

export function CameraTab({
  cue,
  onSave,
}: {
  cue: CameraCueData;
  onSave: (p: Partial<CameraCueData>) => void;
}) {
  const [devices, setDevices] = useState<CameraDeviceInfo[]>([]);
  const [loading, setLoading] = useState(false);

  const source: CameraSource = cue.source ?? { kind: "device", id: "", name: "" };
  const isUrl = source.kind === "url";

  const refresh = () => {
    setLoading(true);
    listCameraDevices()
      .then(setDevices)
      .catch(console.error)
      .finally(() => setLoading(false));
  };

  useEffect(refresh, []);

  const deviceId = source.kind === "device" ? source.id : "";
  // A saved device that is currently absent still needs an entry so the
  // selector shows what is configured instead of silently jumping around.
  const deviceIsMissing = deviceId !== "" && !devices.some((d) => d.id === deviceId);

  return (
    <>
      <Field label="Source">
        <div style={{ display: "flex", gap: 16 }}>
          <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
            <input
              type="radio"
              checked={!isUrl}
              onChange={() => onSave({ source: { kind: "device", id: "", name: "" } })}
              style={{ cursor: "pointer" }}
            />
            <span style={{ fontSize: 13 }}>Device</span>
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
            <input
              type="radio"
              checked={isUrl}
              onChange={() => onSave({ source: { kind: "url", url: "" } })}
              style={{ cursor: "pointer" }}
            />
            <span style={{ fontSize: 13 }}>Network URL</span>
          </label>
        </div>
      </Field>

      {!isUrl && (
        <>
          <Field label="Camera">
            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <Select
                style={{ ...inputStyle, cursor: "pointer" }}
                value={deviceId}
                onChange={(e) => {
                  const d = devices.find((dev) => dev.id === e.target.value);
                  onSave({
                    source: {
                      kind: "device",
                      id: e.target.value,
                      name: d?.name ?? e.target.value,
                    },
                  });
                }}
              >
                <option value="">— select a camera —</option>
                {deviceIsMissing && (
                  <option value={deviceId}>
                    {source.kind === "device" ? source.name : deviceId} (missing)
                  </option>
                )}
                {devices.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                  </option>
                ))}
              </Select>
              <button
                title="Rescan connected cameras"
                onClick={refresh}
                disabled={loading}
                style={{
                  padding: "3px 10px",
                  fontSize: 13,
                  borderRadius: 4,
                  border: "1px solid var(--wc-border-strong)",
                  background: "var(--wc-bg-surface)",
                  color: "var(--wc-text)",
                  cursor: loading ? "default" : "pointer",
                }}
              >
                ↺
              </button>
            </div>
          </Field>
          {!loading && devices.length === 0 && (
            <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 10 }}>
              No camera detected. Connect a webcam, USB camera or HDMI capture
              card, then hit ↺.
            </div>
          )}
        </>
      )}

      {isUrl && (
        <>
          <Field label="Stream URL">
            <input
              style={inputStyle}
              type="text"
              key={`cam-url-${source.kind === "url" ? source.url : ""}`}
              defaultValue={source.kind === "url" ? source.url : ""}
              placeholder="rtsp://192.168.1.50:8554/live"
              onBlur={(e) =>
                onSave({ source: { kind: "url", url: e.target.value.trim() } })
              }
            />
          </Field>
          <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 10 }}>
            Any stream mpv can play: RTSP / HTTP / UDP / HLS. For a phone
            camera, use an app that serves RTSP (e.g. "IP Webcam") and enter
            its URL here.
          </div>
        </>
      )}

      <div style={{ fontSize: 11, color: "var(--wc-text-faint)" }}>
        The feed runs until stopped and is replaced by the next visual cue.
        Use the Fade tab for fade in/out and the Geometry tab to place, scale
        or crop it on the output.
      </div>
    </>
  );
}
