// Typed wrappers around Tauri invoke() calls.
// All backend communication goes through this file.

import { invoke } from "@tauri-apps/api/core";
import type {
  AppPreferences,
  AudioCueData,
  AudioPreferences,
  CollectReport,
  MachineAudioConfig,
  CueId,
  CueListSummary,
  CueSummary,
  CueType,
  DeviceInfo,
  DisplayPreferences,
  DmxUniverseSnapshot,
  FixtureConflict,
  FixtureGroup,
  FixtureType,
  GeneralPreferences,
  CueListTcConfig,
  GroupMode,
  InputPatch,
  OscPatch,
  TcMachineConfig,
  TcPosition,
  TcTrigger,
  OscReceiveConfig,
  OutputPatch,
  ParamTarget,
  PatchedFixture,
  ScreenInfo,
  UniverseOutput,
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
export const getNormalizeDb = (cueId: CueId) =>
  invoke<number>("get_normalize_db", { cueId });
export const listVideoScreens = () => invoke<ScreenInfo[]>("list_video_screens");
export const listSystemFonts  = () => invoke<string[]>("list_system_fonts");
export const previewOutputTimer = (
  font: string, fontSize: number, position: string, margin: number, text: string | null,
) => invoke<void>("preview_output_timer", { font, fontSize: fontSize, position, margin, text });
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
export const collectAndSave = (targetDir: string) =>
  invoke<CollectReport>("collect_and_save_workspace", { targetDir });

// ---------------------------------------------------------------------------
// Cue Lists
// ---------------------------------------------------------------------------

export const getCueLists = () => invoke<CueListSummary[]>("get_cue_lists");
export const addCueList = (name: string) =>
  invoke<string>("add_cue_list", { name });
export const removeCueList = (id: string) =>
  invoke<void>("remove_cue_list", { id });
export const renameCueList = (id: string, name: string) =>
  invoke<void>("rename_cue_list", { id, name });
export const setActiveCueList = (id: string) =>
  invoke<void>("set_active_cue_list", { id });

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
export const getMachineAudioConfig = () =>
  invoke<MachineAudioConfig>("get_machine_audio_config");
export const updateMachineAudioConfig = (config: MachineAudioConfig) =>
  invoke<void>("update_machine_audio_config", { config });
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
export const openPreferencesWindow = () => invoke<void>("open_preferences_window");

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

// ---------------------------------------------------------------------------
// Timecode
// ---------------------------------------------------------------------------

export const listTcMidiInputPorts = () => invoke<string[]>("list_tc_midi_input_ports");
export const getTcConfig = () => invoke<TcMachineConfig>("get_tc_config");
export const setTcConfig = (config: TcMachineConfig) => invoke<void>("set_tc_config", { config });
export const getTcPosition = () => invoke<TcPosition | null>("get_tc_position");
export const getCueTcTrigger = (cueId: string) => invoke<TcTrigger | null>("get_cue_tc_trigger", { cueId });
export const setCueTcTrigger = (
  cueId: string,
  positionStr: string | null,
  rateStr: string | null,
  realTime: boolean,
) => invoke<void>("set_cue_tc_trigger", { cueId, positionStr, rateStr, realTime });
export const getCuelistTcConfig = () => invoke<CueListTcConfig | null>("get_cuelist_tc_config");
export const setCuelistTcConfig = (config: CueListTcConfig) =>
  invoke<void>("set_cuelist_tc_config", { config });

// ---------------------------------------------------------------------------
// Audio inputs + Input Patches (Mic Cues)
// ---------------------------------------------------------------------------

export const listInputDevices = () => invoke<DeviceInfo[]>("list_input_devices");
export const listInputPatches = () => invoke<InputPatch[]>("list_input_patches");
export const addInputPatch = (name: string, deviceId: string, channels: number[]) =>
  invoke<InputPatch>("add_input_patch", { name, deviceId, channels });
export const updateInputPatch = (patch: InputPatch) =>
  invoke<void>("update_input_patch", { patch });
export const removeInputPatch = (patchId: string) =>
  invoke<void>("remove_input_patch", { patchId });

// ---------------------------------------------------------------------------
// OSC Patches
// ---------------------------------------------------------------------------

export const listOscPatches = () => invoke<OscPatch[]>("list_osc_patches");
export const addOscPatch = (name: string, ip: string, port: number) =>
  invoke<OscPatch>("add_osc_patch", { name, ip, port });
export const updateOscPatch = (patch: OscPatch) =>
  invoke<void>("update_osc_patch", { patch });
export const removeOscPatch = (patchId: string) =>
  invoke<void>("remove_osc_patch", { patchId });

// ---------------------------------------------------------------------------
// OSC Receive Config
// ---------------------------------------------------------------------------

export const getOscConfig = () => invoke<OscReceiveConfig>("get_osc_config");
export const setOscConfig = (config: OscReceiveConfig) =>
  invoke<void>("set_osc_config", { config });
export const sendOscTest = (patchId: string, message: import("./types").OscMessage) =>
  invoke<string>("send_osc_test", { patchId, message });

// ---------------------------------------------------------------------------
// MIDI
// ---------------------------------------------------------------------------

export const listMidiOutputPorts = () => invoke<string[]>("list_midi_output_ports");
export const sendMidiTest = (
  portName: string,
  messageType: string,
  channel: number,
  data1: number,
  data2: number,
) => invoke<void>("send_midi_test", { portName, messageType, channel, data1, data2 });

// ---------------------------------------------------------------------------
// DMX / Lighting
// ---------------------------------------------------------------------------

export const dmxSetOutputs = (outputs: UniverseOutput[]) =>
  invoke<void>("dmx_set_outputs", { outputs });
export const dmxGetOutputs = () => invoke<UniverseOutput[]>("dmx_get_outputs");
export const dmxSetChannel = (universe: number, address: number, value: number) =>
  invoke<void>("dmx_set_channel", { universe, address, value });
export const dmxSetBlackout = (on: boolean) => invoke<void>("dmx_set_blackout", { on });
export const dmxGetBlackout = () => invoke<boolean>("dmx_get_blackout");
export const dmxGetSnapshot = () => invoke<DmxUniverseSnapshot[]>("dmx_get_snapshot");

// ---------------------------------------------------------------------------
// DMX / Fixtures
// ---------------------------------------------------------------------------

export const listBuiltinFixtureTypes = () =>
  invoke<FixtureType[]>("list_builtin_fixture_types");
export const listFixtures = () => invoke<PatchedFixture[]>("list_fixtures");
export const addFixture = (
  label: string,
  universe: number,
  baseAddress: number,
  fixtureType: FixtureType,
) => invoke<PatchedFixture>("add_fixture", { label, universe, baseAddress, fixtureType });
export const updateFixture = (fixture: PatchedFixture) =>
  invoke<void>("update_fixture", { fixture });
export const removeFixture = (fixtureId: string) =>
  invoke<void>("remove_fixture", { fixtureId });
export const getFixtureConflicts = () =>
  invoke<FixtureConflict[]>("get_fixture_conflicts");
export const dmxTestFixture = (fixtureId: string, on: boolean) =>
  invoke<void>("dmx_test_fixture", { fixtureId, on });

// Live Dashboard
export const dmxSetFixtureParam = (fixtureId: string, paramIndex: number, value: number) =>
  invoke<void>("dmx_set_fixture_param", { fixtureId, paramIndex, value });
export const dmxClearFixtures = () => invoke<void>("dmx_clear_fixtures");
export const captureLiveTargets = () => invoke<ParamTarget[]>("capture_live_targets");

// Fixture groups
export const listFixtureGroups = () => invoke<FixtureGroup[]>("list_fixture_groups");
export const addFixtureGroup = (label: string, fixtureIds: string[]) =>
  invoke<FixtureGroup>("add_fixture_group", { label, fixtureIds });
export const updateFixtureGroup = (group: FixtureGroup) =>
  invoke<void>("update_fixture_group", { group });
export const removeFixtureGroup = (groupId: string) =>
  invoke<void>("remove_fixture_group", { groupId });
