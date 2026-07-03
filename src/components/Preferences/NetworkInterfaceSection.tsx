// Network Interface section shown in Preferences → Network.
// Selects which local interface carries all of Inkue's network traffic
// (OSC receive/send, OSC feedback, sACN/Art-Net DMX).

import { useEffect, useState } from "react";
import type { NetworkInterfaceConfig, NetworkInterfaceInfo } from "../../lib/types";
import { getNetworkConfig, listNetworkInterfaces, setNetworkConfig } from "../../lib/commands";
import { Select } from "../common/Select";

const AUTOMATIC = "__automatic__";

const selectStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "4px 8px",
  width: "100%",
  cursor: "pointer",
};

const labelStyle: React.CSSProperties = {
  fontSize: 10, fontWeight: 600, color: "var(--wc-text-muted)",
  textTransform: "uppercase", letterSpacing: "0.07em",
  marginBottom: 10, paddingBottom: 5,
  borderBottom: "1px solid var(--wc-border)",
};

export function NetworkInterfaceSection() {
  const [interfaces, setInterfaces] = useState<NetworkInterfaceInfo[]>([]);
  const [config, setConfigState] = useState<NetworkInterfaceConfig>({
    interface_name: null,
    interface_ip: null,
  });
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listNetworkInterfaces().then(setInterfaces).catch(console.error);
    getNetworkConfig().then(setConfigState).catch(console.error);
  }, []);

  const apply = async (next: NetworkInterfaceConfig) => {
    setConfigState(next);
    setError(null);
    try {
      await setNetworkConfig(next);
    } catch (e) {
      setError(String(e));
    }
  };

  const optionValue = (i: NetworkInterfaceInfo) => `${i.name}|${i.ip}`;
  const selected = config.interface_name === null
    ? AUTOMATIC
    : `${config.interface_name}|${config.interface_ip ?? ""}`;
  // A saved interface that is currently absent still needs an entry so the
  // selector shows what is configured instead of silently jumping around.
  const selectedIsMissing =
    config.interface_name !== null &&
    !interfaces.some((i) => optionValue(i) === selected);

  return (
    <div style={{ marginBottom: 24 }}>
      <div style={labelStyle}>Network Interface</div>

      <div style={{ marginBottom: 6 }}>
        <Select
          style={selectStyle}
          value={selected}
          onChange={(e) => {
            const value = e.target.value;
            if (value === AUTOMATIC) {
              void apply({ interface_name: null, interface_ip: null });
              return;
            }
            const separator = value.lastIndexOf("|");
            void apply({
              interface_name: value.slice(0, separator),
              interface_ip: value.slice(separator + 1) || null,
            });
          }}
        >
          <option value={AUTOMATIC}>Automatic (all interfaces)</option>
          {interfaces.map((i) => (
            <option key={optionValue(i)} value={optionValue(i)}>
              {i.name} — {i.ip}{i.is_loopback ? " (loopback)" : ""}
            </option>
          ))}
          {selectedIsMissing && (
            <option value={selected}>
              {config.interface_name} — {config.interface_ip ?? "?"} (not found)
            </option>
          )}
        </Select>
      </div>

      {selectedIsMissing && (
        <div style={{ fontSize: 11, color: "#f59e0b", marginBottom: 4 }}>
          This interface is currently not available — traffic falls back to all interfaces.
        </div>
      )}
      {error && (
        <div style={{ fontSize: 11, color: "#ef4444", marginBottom: 4 }}>{error}</div>
      )}

      <span style={{ fontSize: 11, color: "var(--wc-text-faint)" }}>
        Applies to all OSC (receive, send, feedback) and DMX (sACN / Art-Net) traffic.
        Takes effect immediately.
      </span>
    </div>
  );
}
