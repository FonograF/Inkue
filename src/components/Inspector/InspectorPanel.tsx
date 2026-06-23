// Contextual inspector panel shown on the right side.
// Shows cue properties across four tabs: Basics, Time, Levels, Fade.

import { useEffect, useState } from "react";
import type { AudioCueData, CueSummary, FadeCueData, ImageCueData, LightCueData, MidiCueData, OscCueData, StopCueData, VideoCueData, WaitCueData } from "../../lib/types";
import { getCue, updateCue, setAudioFile, setVideoFile, setImageFile } from "../../lib/commands";
import { WaveformModal } from "../WaveformModal";
import { open } from "@tauri-apps/plugin-dialog";
import { BasicsTab } from "./BasicsTab";
import { TimeTab } from "./TimeTab";
import { LevelsTab } from "./LevelsTab";
import { FadeTab } from "./FadeTab";
import { MidiTab } from "./MidiTab";
import { OscTab } from "./OscTab";
import { LightTab } from "./LightTab";

interface Props {
  selectedCue: CueSummary | null;
  selectedCueIds: string[];
  onRefresh: () => void;
}

type Tab = "basics" | "time" | "levels" | "fade" | "messages" | "light";

export function InspectorPanel({ selectedCue, selectedCueIds, onRefresh }: Props) {
  const [cueData, setCueData] = useState<AudioCueData | VideoCueData | ImageCueData | WaitCueData | FadeCueData | MidiCueData | OscCueData | StopCueData | LightCueData | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("basics");
  const [waveformModalOpen, setWaveformModalOpen] = useState(false);

  useEffect(() => {
    if (!selectedCue) {
      setCueData(null);
      return;
    }
    // Clear stale data immediately so isAudio/isVideo flags never mismatch cueData.
    setCueData(null);
    const hasLevels = selectedCue.cue_type === "audio" || selectedCue.cue_type === "video";
    const hasFade = selectedCue.cue_type === "audio" || selectedCue.cue_type === "image";
    const hasMessages = selectedCue.cue_type === "osc" || selectedCue.cue_type === "midi";
    const hasLight = selectedCue.cue_type === "light";
    setActiveTab((prev) => {
      if ((prev === "levels" && !hasLevels) || (prev === "fade" && !hasFade) || (prev === "messages" && !hasMessages) || (prev === "light" && !hasLight)) return "basics";
      return prev;
    });
    getCue(selectedCue.id)
      .then((data) => {
        // Merge cue_type from the summary in case the serialised form uses
        // a different key ("type" vs "cue_type").
        setCueData({ ...data, cue_type: selectedCue.cue_type } as AudioCueData | VideoCueData | ImageCueData);
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
  const isVideo = selectedCue.cue_type === "video";
  const isImage = selectedCue.cue_type === "image";
  const isGroup = selectedCue.cue_type === "group";
  const isWait  = selectedCue.cue_type === "wait";
  const isFade  = selectedCue.cue_type === "fade";
  const isMidi  = selectedCue.cue_type === "midi";
  const isOsc   = selectedCue.cue_type === "osc";
  const isStop  = selectedCue.cue_type === "stop";
  const isLight = selectedCue.cue_type === "light";

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const save = async (partial: Partial<any>) => {
    // Color changes fan out to every selected cue; everything else applies to
    // the primary (inspector) cue only.
    if ("color" in partial && selectedCueIds.length > 1) {
      await Promise.all(
        selectedCueIds.map((id) => updateCue(id, { color: partial.color }).catch(console.error)),
      );
      // Apply any remaining non-color fields to the primary cue.
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { color: _c, ...rest } = partial;
      if (Object.keys(rest).length > 0) {
        await updateCue(cueData.id, rest).catch(console.error);
      }
    } else {
      await updateCue(cueData.id, partial).catch(console.error);
    }
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

  const handleBrowseVideo = async () => {
    const result = await open({
      multiple: false,
      filters: [
        { name: "Video Files", extensions: ["mp4", "m4v", "webm", "mov", "mkv", "avi", "ogv"] },
      ],
    });
    if (typeof result === "string") {
      await setVideoFile(cueData.id, result).catch(console.error);
      setCueData((prev) => (prev ? { ...prev, file_path: result } : prev));
      onRefresh();
    }
  };

  const handleBrowseImage = async () => {
    const result = await open({
      multiple: false,
      filters: [
        { name: "Image Files", extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"] },
      ],
    });
    if (typeof result === "string") {
      await setImageFile(cueData.id, result).catch(console.error);
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
        {isAudio ? "🔊" : isVideo ? "🎬" : isImage ? "🖼" : isGroup ? "📦" : isWait ? "⏱" : isFade ? "📉" : isMidi ? "🎹" : isOsc ? "📡" : isStop ? "⏹" : isLight ? "💡" : "📝"} {cueData.name}
      </div>

      {/* Tabs */}
      <div style={{ display: "flex", borderBottom: "1px solid #1e293b" }}>
        <button style={tabStyle("basics")} onClick={() => setActiveTab("basics")}>
          Basics
        </button>
        <button style={tabStyle("time")} onClick={() => setActiveTab("time")}>
          Time
        </button>
        {(isAudio || isVideo) && (
          <button style={tabStyle("levels")} onClick={() => setActiveTab("levels")}>
            Levels
          </button>
        )}
        {(isAudio || isImage) && (
          <button style={tabStyle("fade")} onClick={() => setActiveTab("fade")}>
            Fade
          </button>
        )}
        {(isOsc || isMidi) && (
          <button style={tabStyle("messages")} onClick={() => setActiveTab("messages")}>
            Messages
          </button>
        )}
        {isLight && (
          <button style={tabStyle("light")} onClick={() => setActiveTab("light")}>
            Light
          </button>
        )}
      </div>

      {/* Tab content */}
      <div style={{ flex: 1, overflowY: "auto", padding: 12 }}>
        {activeTab === "basics" && (
          <BasicsTab
            cue={cueData}
            isAudio={isAudio}
            isVideo={isVideo}
            isImage={isImage}
            isGroup={isGroup}
            isFade={isFade}
            isStop={isStop}
            onSave={save}
            onRefresh={onRefresh}
            onBrowse={handleBrowse}
            onBrowseVideo={handleBrowseVideo}
            onBrowseImage={handleBrowseImage}
          />
        )}
        {activeTab === "time" && (
          <TimeTab
            cue={cueData}
            selectedCue={selectedCue}
            isAudio={isAudio}
            isVideo={isVideo}
            isImage={isImage}
            isWait={isWait}
            isFade={isFade}
            onSave={save}
            onOpenWaveform={() => setWaveformModalOpen(true)}
          />
        )}
        {activeTab === "levels" && (isAudio || isVideo) && (
          <LevelsTab cue={cueData as AudioCueData | VideoCueData} isAudio={isAudio} onSave={save} />
        )}
        {activeTab === "fade" && (isAudio || isImage) && (
          <FadeTab cue={cueData as AudioCueData | ImageCueData} onSave={save} />
        )}
        {activeTab === "messages" && isOsc && (
          <OscTab cue={cueData as OscCueData} onSave={save} />
        )}
        {activeTab === "messages" && isMidi && (
          <MidiTab cue={cueData as MidiCueData} onSave={save} />
        )}
        {activeTab === "light" && isLight && (
          <LightTab cue={cueData as LightCueData} onSave={save} />
        )}
      </div>

      {waveformModalOpen && cueData && isAudio && (
        <WaveformModal
          cue={cueData as AudioCueData}
          onClose={() => setWaveformModalOpen(false)}
          onSave={save}
        />
      )}
    </div>
  );
}
