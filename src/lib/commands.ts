// Typed wrappers around Tauri invoke() calls.
// All backend communication goes through this file.

import { invoke } from "@tauri-apps/api/core";
import type {
  AppPreferences,
  AudioCueData,
  AudioPreferences,
  CueId,
  CueSummary,
  CueType,
  DeviceInfo,
  DisplayPreferences,
  GeneralPreferences,
  GroupMode,
  OutputPatch,
  ScreenInfo,
  VideoCueData,
  WaveformData,
  WorkspaceInfo,
} from "./types";

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

export const go = () => invoke<void>("go");
export const setMasterVolume = (db: number) => invoke<void>("set_master_volume", { db });
export const stopAll = () => invoke<void>("stop_all");
export const hardStopAll = () => invoke<void>("hard_stop_all");
export const stopCue = (cueId: CueId) => invoke<void>("stop_cue", { cueId });
export const pauseCue = (cueId: CueId) => invoke<void>("pause_cue", { cueId });
export const seekCue = (cueId: CueId, positionMs: number) =>
  invoke<void>("seek_cue", { cueId, positionMs });
export const resumeCue = (cueId: CueId) =>
  invoke<void>("resume_cue", { cueId });

// ---------------------------------------------------------------------------
// Cue management
// ---------------------------------------------------------------------------

export const getAllCues = () => invoke<CueSummary[]>("get_all_cues");
export const getCue = (cueId: CueId) =>
  invoke<AudioCueData | VideoCueData>("get_cue", { cueId });
export const addCue = (cueType: CueType, position = -1) =>
  invoke<CueId>("add_cue", { cueType, position });
export const removeCue = (cueId: CueId) =>
  invoke<void>("remove_cue", { cueId });
export const removeCues = (ids: CueId[]) =>
  invoke<void>("remove_cues", { ids });
export const moveCue = (cueId: CueId, newPosition: number) =>
  invoke<void>("move_cue", { cueId, newPosition });
export const moveCues = (ids: CueId[], beforeId: CueId | null) =>
  invoke<void>("move_cues", { ids, beforeId });
export const groupCues = (ids: CueId[]) =>
  invoke<CueId>("group_cues", { ids });
export const ungroup = (groupId: CueId) =>
  invoke<void>("ungroup", { groupId });
export const setGroupMode = (groupId: CueId, mode: GroupMode) =>
  invoke<void>("set_group_mode", { groupId, mode });
export const addCueToGroup = (cueId: CueId, groupId: CueId, position = -1) =>
  invoke<void>("add_cue_to_group", { cueId, groupId, position });
export const removeCueFromGroup = (groupId: CueId, cueId: CueId) =>
  invoke<void>("remove_cue_from_group", { groupId, cueId });
export const moveToTopLevel = (cueId: CueId, beforeId: CueId | null) =>
  invoke<void>("move_to_top_level", { cueId, beforeId });
export const duplicateCue = (cueId: CueId) =>
  invoke<CueId>("duplicate_cue", { cueId });
export const duplicateCues = (ids: CueId[]) =>
  invoke<CueId[]>("duplicate_cues", { ids });

// ---------------------------------------------------------------------------
// Undo / Redo / Copy / Paste
// ---------------------------------------------------------------------------

export const undo = () => invoke<void>("undo");
export const redo = () => invoke<void>("redo");
export const canUndo = () => invoke<boolean>("can_undo");
export const canRedo = () => invoke<boolean>("can_redo");
export const copyCue = (cueId: CueId) => invoke<void>("copy_cue", { cueId });
export const pasteCue = (afterCueId?: CueId | null) =>
  invoke<CueId>("paste_cue", { afterCueId: afterCueId ?? null });
export const updateCue = (cueId: CueId, properties: Partial<AudioCueData>) =>
  invoke<void>("update_cue", { cueId, properties });
export const setAudioFile = (cueId: CueId, filePath: string) =>
  invoke<void>("set_audio_file", { cueId, filePath });
export const setVideoFile = (cueId: CueId, filePath: string) =>
  invoke<void>("set_video_file", { cueId, filePath });
export const getWaveformPeaks = (cueId: CueId, bins: number) =>
  invoke<WaveformData>("get_waveform_peaks", { cueId, bins });
export const listVideoScreens = () => invoke<ScreenInfo[]>("list_video_screens");
export const setImageFile = (cueId: CueId, filePath: string) =>
  invoke<void>("set_image_file", { cueId, filePath });

export const previewCue = (cueId: CueId, startMs?: number, endMs?: number) =>
  invoke<string>("preview_cue", {
    cueId,
    startMs: startMs != null ? Math.round(startMs) : null,
    endMs: endMs != null ? Math.round(endMs) : null,
  });

export const stopPreview = (voiceId: string) =>
  invoke<void>("stop_preview", { voiceId });

// ---------------------------------------------------------------------------
// Playhead
// ---------------------------------------------------------------------------

export const setPlayhead = (cueId: CueId | null) =>
  invoke<void>("set_playhead", { cueId });
export const getPlayhead = () => invoke<CueId | null>("get_playhead");

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

export const newWorkspace = () => invoke<void>("new_workspace");
export const saveWorkspace = (path: string) =>
  invoke<void>("save_workspace", { path });
export const loadWorkspace = (path: string) =>
  invoke<void>("load_workspace", { path });
export const getWorkspaceInfo = () => invoke<WorkspaceInfo>("get_workspace_info");

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

export const getPreferences = () => invoke<AppPreferences>("get_preferences");
export const getAvailableBackends = () =>
  invoke<string[]>("get_available_backends");
export const getAsioOutputPairs = () =>
  invoke<number>("get_asio_output_pairs");
export const updateAudioPreferences = (prefs: AudioPreferences) =>
  invoke<void>("update_audio_preferences", { prefs });
export const updateGeneralPreferences = (prefs: GeneralPreferences) =>
  invoke<void>("update_general_preferences", { prefs });
export const updateDisplayPreferences = (prefs: DisplayPreferences) =>
  invoke<void>("update_display_preferences", { prefs });
export async function listAudioDevices(backend?: string): Promise<DeviceInfo[]> {
  return invoke<DeviceInfo[]>("list_audio_devices", { backend: backend ?? null });
}
export const testAudioDevice = (deviceId: string, backend: string) =>
  invoke<void>("test_audio_device", { deviceId, backend });
export const getOutputScreen = () =>
  invoke<number | null>("get_output_screen");
export const setOutputScreen = (screen: number | null) =>
  invoke<void>("set_output_screen", { screen });
export const toggleOutputWindow = () => invoke<void>("toggle_output_window");
export const getOutputWindowVisible = () => invoke<boolean>("get_output_window_visible");

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

export const listOutputDevices = () =>
  invoke<DeviceInfo[]>("list_output_devices");
export const getOutputPatches = () => invoke<OutputPatch[]>("get_output_patches");
export const setOutputPatch = (
  patchId: string | null,
  name: string,
  deviceId: string,
  channels: number[]
) => invoke<string>("set_output_patch", { patchId, name, deviceId, channels });
export const refreshDevices = () => invoke<void>("refresh_devices");
