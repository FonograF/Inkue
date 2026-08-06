// Contextual inspector panel shown on the right side.
//
// Tab model: Basics (identity) | the cue type's main tab (Fade / Stop / Group /
// Camera / Messages / Light / Mic / Timecode / Text) | Time | Levels | Fade |
// Layer | Geometry | Triggers — only the tabs that apply to the selected cue
// type are shown.

import { useEffect, useState } from "react";
import type { ScriptCueData, AudioCueData, CameraCueData, CueSummary, CueType, DevampCueData, FadeCueData, ImageCueData, LightCueData, MicCueData, MidiCueData, OscCueData, StopCueData, TextCueData, TimecodeCueData, VideoCueData, WaitCueData } from "../../lib/types";
import { isCommandCueType } from "../../lib/types";
import { getCue, updateCue, setAudioFile, setVideoFile, setImageFile } from "../../lib/commands";
import { AUDIO_EXTENSIONS, VIDEO_EXTENSIONS, IMAGE_EXTENSIONS } from "../../lib/mediaTypes";
import { open } from "@tauri-apps/plugin-dialog";
import { BasicsTab } from "./BasicsTab";
import { TimeTab } from "./TimeTab";
import { LevelsTab } from "./LevelsTab";
import { FadeTab } from "./FadeTab";
import { FadeCueTab } from "./FadeCueTab";
import { StopTab } from "./StopTab";
import { CommandTab } from "./CommandTab";
import { ScriptTab } from "./ScriptTab";
import { DevampTab } from "./DevampTab";
import { GroupTab } from "./GroupTab";
import { LayerTab } from "./LayerTab";
import { GeometryTab } from "./GeometryTab";
import { MidiTab } from "./MidiTab";
import { OscTab } from "./OscTab";
import { LightTab } from "./LightTab";
import { MicTab } from "./MicTab";
import { TextTab } from "./TextTab";
import { TimecodeTab } from "./TimecodeTab";
import { TriggersTab } from "./TriggersTab";
import { CameraTab } from "./CameraTab";

interface Props {
  selectedCue: CueSummary | null;
  selectedCueIds: string[];
  onRefresh: () => void;
  /** Open the clip editor dock (trim + slices) for a cue. */
  onOpenEditor?: (cueId: string) => void;
  /** Bump to force a re-fetch of the inspected cue (dock edits). */
  reloadToken?: number;
  /** Called after every inspector save, so the clip editor dock can reload. */
  onCueSaved?: () => void;
}

type Tab =
  | "basics" | "time" | "levels" | "fade" | "layer" | "geometry" | "messages"
  | "fade-cue" | "stop" | "devamp" | "group" | "light" | "mic" | "timecode"
  | "text" | "camera" | "triggers" | "command" | "script";

type CueData =
  | AudioCueData | VideoCueData | ImageCueData | WaitCueData | FadeCueData
  | MidiCueData | OscCueData | StopCueData | DevampCueData | LightCueData
  | MicCueData | TimecodeCueData | TextCueData | CameraCueData;

const CUE_ICONS: Partial<Record<CueType, string>> = {
  audio: "🔊", video: "🎬", image: "🖼", group: "📦", wait: "⏱", fade: "📉",
  midi: "🎹", osc: "📡", stop: "⏹", light: "💡", mic: "🎤", timecode: "🕐",
  text: "🔤", camera: "📷", devamp: "🔁",
  start: "▶", pause: "⏸", resume: "⏯", load: "⏏", reset: "⏮",
  goto: "↪", arm: "🔓", disarm: "🔒", script: "⚙",
};

/** Ordered tab list for a cue type: identity first, the type's main tab next,
 *  then timing, shared A/V tabs, and Triggers last. */
function tabsFor(type: CueType): { id: Tab; label: string }[] {
  const isAV = type === "audio" || type === "video";
  const isVisual = type === "video" || type === "image" || type === "camera";
  const hasAvFade = isAV || type === "image" || type === "camera";

  const tabs: { id: Tab; label: string }[] = [{ id: "basics", label: "Basics" }];
  if (type === "fade") tabs.push({ id: "fade-cue", label: "Fade" });
  if (type === "stop") tabs.push({ id: "stop", label: "Stop" });
  if (isCommandCueType(type)) tabs.push({ id: "command", label: "Command" });
  if (type === "devamp") tabs.push({ id: "devamp", label: "Devamp" });
  if (type === "group") tabs.push({ id: "group", label: "Group" });
  if (type === "camera") tabs.push({ id: "camera", label: "Camera" });
  if (type === "osc" || type === "midi") tabs.push({ id: "messages", label: "Messages" });
  if (type === "light") tabs.push({ id: "light", label: "Light" });
  if (type === "mic") tabs.push({ id: "mic", label: "Mic" });
  if (type === "timecode") tabs.push({ id: "timecode", label: "Timecode" });
  if (type === "text") tabs.push({ id: "text", label: "Text" });
  if (type === "script") tabs.push({ id: "script", label: "Script" });
  tabs.push({ id: "time", label: "Time" });
  if (isAV) tabs.push({ id: "levels", label: "Levels" });
  if (hasAvFade) tabs.push({ id: "fade", label: "Fade" });
  if (isVisual) tabs.push({ id: "layer", label: "Layer" });
  if (isVisual) tabs.push({ id: "geometry", label: "Geometry" });
  tabs.push({ id: "triggers", label: "Triggers" });
  return tabs;
}

export function InspectorPanel({ selectedCue, selectedCueIds, onRefresh, onOpenEditor, reloadToken, onCueSaved }: Props) {
  const [cueData, setCueData] = useState<CueData | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("basics");

  useEffect(() => {
    if (!selectedCue) {
      setCueData(null);
      return;
    }
    // Clear stale data immediately so type flags never mismatch cueData.
    setCueData(null);
    const available = tabsFor(selectedCue.cue_type).map((t) => t.id);
    setActiveTab((prev) => (available.includes(prev) ? prev : "basics"));
    getCue(selectedCue.id)
      .then((data) => {
        // Merge cue_type from the summary in case the serialised form uses
        // a different key ("type" vs "cue_type").
        setCueData({ ...data, cue_type: selectedCue.cue_type } as CueData);
      })
      .catch(console.error);
    // reloadToken: the clip editor dock saved this cue — re-fetch it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedCue?.id, reloadToken]);

  if (!selectedCue || !cueData) {
    return (
      <div
        style={{
          padding: 24,
          color: "var(--wc-text-faint)",
          textAlign: "center",
          fontSize: 13,
        }}
      >
        Select a cue to inspect it.
      </div>
    );
  }

  const type = selectedCue.cue_type;
  const isAudio = type === "audio";
  const isVideo = type === "video";
  const isImage = type === "image";
  const isWait  = type === "wait";
  const isFade  = type === "fade";
  const isCamera = type === "camera";

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
    onCueSaved?.();
  };

  const browseMedia = (kind: "audio" | "video" | "image") => async () => {
    const filters = {
      audio: { name: "Audio Files", extensions: [...AUDIO_EXTENSIONS] },
      video: { name: "Video Files", extensions: [...VIDEO_EXTENSIONS] },
      image: { name: "Image Files", extensions: [...IMAGE_EXTENSIONS] },
    }[kind];
    const setFile = { audio: setAudioFile, video: setVideoFile, image: setImageFile }[kind];
    const result = await open({ multiple: false, filters: [filters] });
    if (typeof result === "string") {
      await setFile(cueData.id, result).catch(console.error);
      // The backend rebuilt the cue (a changed file also resets start/end/
      // slices) — re-fetch instead of patching locally, or the inspector
      // keeps showing the old clip window until the cue is re-selected.
      const type = cueData.cue_type;
      await getCue(cueData.id)
        .then((data) => setCueData({ ...data, cue_type: type } as CueData))
        .catch(console.error);
      onRefresh();
      onCueSaved?.(); // the clip editor dock reloads too
    }
  };

  const tabStyle = (tab: Tab): React.CSSProperties => ({
    padding: "7px 11px",
    cursor: "pointer",
    fontSize: 12,
    whiteSpace: "nowrap",
    background: activeTab === tab ? "var(--wc-bg-surface)" : "transparent",
    color: activeTab === tab ? "var(--wc-text)" : "var(--wc-text-muted)",
    fontWeight: activeTab === tab ? 600 : 400,
    border: "none",
    borderBottom:
      activeTab === tab ? "2px solid var(--wc-accent)" : "2px solid transparent",
    outline: "none",
  });

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "var(--wc-bg-app)",
        color: "var(--wc-text)",
        fontSize: 13,
      }}
    >
      {/* Title */}
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: 8,
          padding: "8px 12px",
          borderBottom: "1px solid var(--wc-border)",
          background: "var(--wc-bg-deepest)",
        }}
      >
        <span style={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {CUE_ICONS[type] ?? "📝"} {cueData.name}
        </span>
        {selectedCue.number && (
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)", flexShrink: 0 }}>
            #{selectedCue.number}
          </span>
        )}
      </div>

      {/* Tabs — flexWrap so every tab stays reachable however narrow the
          inspector gets (a fixed row cropped the trailing tabs). */}
      <div style={{ display: "flex", flexWrap: "wrap", borderBottom: "1px solid var(--wc-border)" }}>
        {tabsFor(type).map((t) => (
          <button key={t.id} style={tabStyle(t.id)} onClick={() => setActiveTab(t.id)}>
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div style={{ flex: 1, overflowY: "auto", padding: 12 }}>
        {activeTab === "basics" && (
          <BasicsTab
            cue={cueData}
            isAudio={isAudio}
            isVideo={isVideo}
            isImage={isImage}
            onSave={save}
            onBrowse={browseMedia("audio")}
            onBrowseVideo={browseMedia("video")}
            onBrowseImage={browseMedia("image")}
          />
        )}
        {activeTab === "fade-cue" && isFade && (
          <FadeCueTab cue={cueData as FadeCueData} onSave={save} />
        )}
        {activeTab === "script" && type === "script" && (
          <ScriptTab cue={cueData as unknown as ScriptCueData} onSave={save} />
        )}
        {activeTab === "command" && isCommandCueType(type) && (
          <CommandTab cue={cueData as StopCueData} onSave={save} />
        )}
        {activeTab === "stop" && type === "stop" && (
          <StopTab cue={cueData as StopCueData} onSave={save} />
        )}
        {activeTab === "devamp" && type === "devamp" && (
          <DevampTab cue={cueData as DevampCueData} onSave={save} />
        )}
        {activeTab === "group" && type === "group" && (
          <GroupTab cue={cueData} onRefresh={onRefresh} />
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
            onOpenWaveform={() => onOpenEditor?.(cueData.id)}
          />
        )}
        {activeTab === "levels" && (isAudio || isVideo) && (
          <LevelsTab cue={cueData as AudioCueData | VideoCueData} isAudio={isAudio} onSave={save} />
        )}
        {activeTab === "fade" && (isAudio || isVideo || isImage || isCamera) && (
          <FadeTab cue={cueData as AudioCueData | VideoCueData | ImageCueData | CameraCueData} onSave={save} />
        )}
        {activeTab === "layer" && (isVideo || isImage || isCamera) && (
          <LayerTab cue={cueData as VideoCueData | ImageCueData | CameraCueData} onSave={save} />
        )}
        {activeTab === "geometry" && (isVideo || isImage || isCamera) && (
          <GeometryTab cue={cueData as VideoCueData | ImageCueData | CameraCueData} onSave={save} />
        )}
        {activeTab === "camera" && isCamera && (
          <CameraTab cue={cueData as CameraCueData} onSave={save} />
        )}
        {activeTab === "messages" && type === "osc" && (
          <OscTab cue={cueData as OscCueData} onSave={save} />
        )}
        {activeTab === "messages" && type === "midi" && (
          <MidiTab cue={cueData as MidiCueData} onSave={save} />
        )}
        {activeTab === "light" && type === "light" && (
          <LightTab cue={cueData as LightCueData} onSave={save} />
        )}
        {activeTab === "mic" && type === "mic" && (
          <MicTab cue={cueData as MicCueData} onSave={save} />
        )}
        {activeTab === "timecode" && type === "timecode" && (
          <TimecodeTab cue={cueData as TimecodeCueData} onSave={save} />
        )}
        {activeTab === "text" && type === "text" && (
          <TextTab cue={cueData as TextCueData} onSave={save} />
        )}
        {activeTab === "triggers" && (
          <TriggersTab cue={selectedCue} onSave={onRefresh} />
        )}
      </div>

    </div>
  );
}
