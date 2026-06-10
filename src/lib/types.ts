// TypeScript types mirroring the Rust backend serialised structs.

export type CueId = string; // UUID as string

export type CueType = "audio" | "memo" | "wait" | "group" | "fade" | "stop" | "video" | "image" | "osc";

export type CueState = "standby" | "running" | "paused" | "completed";

export type ContinueMode = "do_not_continue" | "auto_continue" | "auto_follow";

export type CueColor =
  | "none"
  | "red"
  | "orange"
  | "yellow"
  | "green"
  | "blue"
  | "purple"
  | "pink"
  | "white"
  | "black";

export type FadeCurve = "linear" | "s_curve" | "exponential";

export type GroupMode = "simultaneous" | "sequential";

export interface FadeSpec {
  duration_ms: number;
  curve: FadeCurve;
}

/** Compact row data used to render the cue list table. */
export interface CueSummary {
  id: CueId;
  cue_type: CueType;
  name: string;
  number: string | null;
  state: CueState;
  continue_mode: ContinueMode;
  color: CueColor;
  pre_wait_ms: number;
  post_wait_ms: number;
  duration_ms: number | null;
  file_path: string | null;
  /** True while the audio file is being decoded in a background thread. */
  is_loading: boolean;
  /** For Group cues: direct child cue summaries (recursive). */
  children?: CueSummary[];
  /** For Group cues: playback mode. */
  group_mode?: GroupMode;
  /** For running Sequential Group cues: ID of the currently active child. */
  active_child_id?: string;
}

/** Full cue data returned by get_cue. */
export interface AudioCueData extends CueSummary {
  notes: string;
  volume_db: number;
  pan: number;
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
  start_time_ms: number | null;
  end_time_ms: number | null;
  loop_count: number;
  output_patch_id: string | null;
  rate: number;
}

/** Full cue data returned by get_cue for a Video Cue. */
export interface VideoCueData extends CueSummary {
  notes: string;
  volume_db: number;
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
  start_time_ms: number | null;
  end_time_ms: number | null;
  loop_count: number;
  output_surface_id: string | null;
}

/** Full cue data returned by get_cue for an Image Cue. */
export interface ImageCueData extends CueSummary {
  notes: string;
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
}

/** Full cue data returned by get_cue for a Wait Cue. */
export interface WaitCueData extends CueSummary {
  notes: string;
  /** The configured wait duration in milliseconds. */
  wait_duration_ms: number;
}

/** Information about a connected monitor. */
export interface ScreenInfo {
  index: number;
  width: number;
  height: number;
  x: number;
  y: number;
  is_primary: boolean;
}

export interface DeviceInfo {
  id: string;
  name: string;
  channels: number;
  sample_rate: number;
}

export interface OutputPatch {
  id: string;
  name: string;
  device_id: string;
  channels: number[];
}

export interface WorkspaceInfo {
  name: string;
  is_modified: boolean;
  file_path: string | null;
}

export interface WaveformData {
  peaks: number[];
  file_duration_s: number;
}

// ---------------------------------------------------------------------------
// OSC types
// ---------------------------------------------------------------------------

export type OscArgType = "int" | "float" | "str" | "bool";

export type OscArg =
  | { type: "int";   value: number }
  | { type: "float"; value: number }
  | { type: "str";   value: string }
  | { type: "bool";  value: boolean };

export interface OscMessage {
  patch_id: string;
  address: string;
  args: OscArg[];
}

export interface OscPatch {
  id: string;
  name: string;
  ip: string;
  port: number;
}

/** Full cue data returned by get_cue for an OSC Cue. */
export interface OscCueData extends CueSummary {
  notes: string;
  messages: OscMessage[];
}

export interface OscReceiveConfig {
  enabled: boolean;
  port: number;
  allowed_ips: string[];
  feedback_enabled: boolean;
  feedback_host: string;
  feedback_port: number;
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

export type AudioBackend = "wasapi_shared" | "wasapi_exclusive" | "asio";

/** Hardware-specific settings — stored in %APPDATA%\WinCue\audio.json, not in the workspace. */
export interface MachineAudioConfig {
  backend: AudioBackend;
  device_id: string | null;
  /** Samples. Only applied for WASAPI Exclusive. */
  buffer_size: number;
  /** ASIO output pair index (0 = Out 1-2, 1 = Out 3-4, …). */
  asio_out_pair: number;
}

export const DEFAULT_MACHINE_AUDIO_CONFIG: MachineAudioConfig = {
  backend: "wasapi_shared",
  device_id: null,
  buffer_size: 256,
  asio_out_pair: 0,
};

/** Show-specific audio defaults — travel with the .wincue workspace file. */
export interface AudioPreferences {
  default_volume_db: number;
  default_fade_out_ms: number;
  default_fade_curve: FadeCurve;
}

export type CueRowHeight = "compact" | "normal" | "tall";

export interface GeneralPreferences {
  double_go_protection_ms: number;
  confirm_before_delete: boolean;
  auto_scroll_to_playhead: boolean;
  cue_row_height: CueRowHeight;
}

export const DEFAULT_GENERAL_PREFS: GeneralPreferences = {
  double_go_protection_ms: 500,
  confirm_before_delete: false,
  auto_scroll_to_playhead: true,
  cue_row_height: "normal",
};

export type TimerPosition = "center" | "top_left" | "top_right" | "bottom_left" | "bottom_right";

export interface DisplayPreferences {
  /** Monitor index for the unified output surface. null = floating window. */
  output_screen: number | null;
  /** When true, a countdown timer is shown on the output window. */
  show_output_timer: boolean;
  /** When true the timer counts down (remaining); when false it counts up (elapsed). */
  timer_count_down: boolean;
  /** Font family for the OSD timer (e.g. "Arial"). */
  timer_font: string;
  /** Font size in mpv OSD points for the timer (default 120). */
  timer_font_size: number;
  /** Position of the timer on the output window. */
  timer_position: TimerPosition;
  /** When true, milliseconds are shown (e.g. 00:00.000). */
  timer_show_ms: boolean;
  /** Margin in pixels from the edge for corner positions. */
  timer_margin: number;
  /** When true (and show_output_timer is true), show timer as floating Win32 window instead of OSD overlay. */
  timer_floating: boolean;
  bg_app: string;
  bg_surface: string;
  bg_panel: string;
  accent: string;
  text_primary: string;
}

export const DEFAULT_DISPLAY_PREFS: Pick<DisplayPreferences, "bg_app" | "bg_surface" | "bg_panel" | "accent" | "text_primary"> = {
  bg_app:       "#020617",
  bg_surface:   "#0f172a",
  bg_panel:     "#1e293b",
  accent:       "#3b82f6",
  text_primary: "#e2e8f0",
};

export interface AppPreferences {
  audio: AudioPreferences;
  general: GeneralPreferences;
  network: Record<string, never>;
  display: DisplayPreferences;
}

// ---------------------------------------------------------------------------
// Events emitted by the backend
export interface CueStateChangedEvent {
  cue_id: CueId;
  old_state: CueState;
  new_state: CueState;
}

export interface PlayheadMovedEvent {
  cue_id: CueId | null;
}

export interface WorkspaceModifiedEvent {
  /* empty */
}

export interface DeviceChangedEvent {
  devices: DeviceInfo[];
}

export interface CueTimeUpdateEvent {
  cue_id: CueId;
  elapsed_ms: number;
  action_elapsed_ms: number;
  remaining_ms: number;
}
