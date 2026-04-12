// Contextual inspector panel shown on the right side.
// Shows cue properties across four tabs: Basics, Time, Levels, Fade.

import { useEffect, useState } from "react";
import type { AudioCueData, CueSummary } from "../../lib/types";
import { getCue, updateCue, setAudioFile } from "../../lib/commands";
import { WaveformModal } from "../WaveformModal";
import { open } from "@tauri-apps/plugin-dialog";
import { BasicsTab } from "./BasicsTab";
import { TimeTab } from "./TimeTab";
import { LevelsTab } from "./LevelsTab";
import { FadeTab } from "./FadeTab";

interface Props {
  selectedCue: CueSummary | null;
  onRefresh: () => void;
}

type Tab = "basics" | "time" | "levels" | "fade";

export function InspectorPanel({ selectedCue, onRefresh }: Props) {
  const [cueData, setCueData] = useState<AudioCueData | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("basics");
  const [waveformModalOpen, setWaveformModalOpen] = useState(false);

  useEffect(() => {
    if (!selectedCue) {
      setCueData(null);
      return;
    }
    getCue(selectedCue.id)
      .then((data) => {
        setCueData({ ...data, cue_type: selectedCue.cue_type });
      })
      .catch(console.error);
  }, [selectedCue?.id]);

  if (!selectedCue || !cueData) {
    return (
      <div
        style={{
          padding: 24,
          color: "#475569",
          textAlign: "center",
          fontSize: 13,
        }}
      >
        Select a cue to inspect it.
      </div>
    );
  }

  const isAudio = selectedCue.cue_type === "audio";

  const save = async (partial: Partial<AudioCueData>) => {
    await updateCue(cueData.id, partial).catch(console.error);
    setCueData((prev) => (prev ? { ...prev, ...partial } : prev));
    onRefresh();
  };

  const handleBrowse = async () => {
    const result = await open({
      multiple: false,
      filters: [
        { name: "Audio Files", extensions: ["wav", "mp3", "flac", "ogg", "aac"] },
      ],
    });
    if (typeof result === "string") {
      await setAudioFile(cueData.id, result).catch(console.error);
      setCueData((prev) => (prev ? { ...prev, file_path: result } : prev));
      onRefresh();
    }
  };

  const tabStyle = (tab: Tab): React.CSSProperties => ({
    padding: "6px 14px",
    cursor: "pointer",
    fontSize: 12,
    background: activeTab === tab ? "#1e293b" : "transparent",
    color: activeTab === tab ? "#e2e8f0" : "#64748b",
    border: "none",
    borderBottom:
      activeTab === tab ? "2px solid #3b82f6" : "2px solid transparent",
    outline: "none",
  });

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "#0f172a",
        color: "#e2e8f0",
        fontSize: 13,
      }}
    >
      {/* Title */}
      <div
        style={{
          padding: "8px 12px",
          fontWeight: 600,
          borderBottom: "1px solid #1e293b",
          background: "#020617",
        }}
      >
        {isAudio ? "🔊" : "📝"} {cueData.name}
      </div>

      {/* Tabs */}
      <div style={{ display: "flex", borderBottom: "1px solid #1e293b" }}>
        <button style={tabStyle("basics")} onClick={() => setActiveTab("basics")}>
          Basics
        </button>
        <button style={tabStyle("time")} onClick={() => setActiveTab("time")}>
          Time
        </button>
        {isAudio && (
          <button style={tabStyle("levels")} onClick={() => setActiveTab("levels")}>
            Levels
          </button>
        )}
        {isAudio && (
          <button style={tabStyle("fade")} onClick={() => setActiveTab("fade")}>
            Fade
          </button>
        )}
      </div>

      {/* Tab content */}
      <div style={{ flex: 1, overflowY: "auto", padding: 12 }}>
        {activeTab === "basics" && (
          <BasicsTab
            cue={cueData}
            isAudio={isAudio}
            onSave={save}
            onBrowse={handleBrowse}
          />
        )}
        {activeTab === "time" && (
          <TimeTab
            cue={cueData}
            isAudio={isAudio}
            onSave={save}
            onOpenWaveform={() => setWaveformModalOpen(true)}
          />
        )}
        {activeTab === "levels" && isAudio && (
          <LevelsTab cue={cueData} onSave={save} />
        )}
        {activeTab === "fade" && isAudio && (
          <FadeTab cue={cueData} onSave={save} />
        )}
      </div>

      {waveformModalOpen && cueData && (
        <WaveformModal
          cue={cueData}
          onClose={() => setWaveformModalOpen(false)}
          onSave={save}
        />
      )}
    </div>
  );
}
