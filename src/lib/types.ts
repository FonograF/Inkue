// TypeScript types mirroring the Rust backend serialised structs.

export type CueId = string; // UUID as string

export type CueType = "audio" | "memo" | "wait" | "group" | "fade";

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
// Preferences
// ---------------------------------------------------------------------------

export type AudioBackend = "wasapi_shared" | "wasapi_exclusive" | "asio";

export interface AudioPreferences {
  buffer_size: number;
  backend: AudioBackend;
  device_id: string | null;
  default_volume_db: number;
  default_fade_out_ms: number;
  default_fade_curve: FadeCurve;
  /** ASIO output pair index (0 = Out 1-2, 1 = Out 3-4, …). */
  asio_out_pair: number;
}

export interface AppPreferences {
  audio: AudioPreferences;
  general: Record<string, never>;
  network: Record<string, never>;
  display: Record<string, never>;
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
