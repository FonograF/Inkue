// TypeScript types mirroring the Rust backend serialised structs.

export type CueId = string; // UUID as string

export type CueType = "audio" | "memo" | "wait" | "group" | "fade" | "stop" | "devamp" | "video" | "image" | "osc" | "midi" | "light" | "mic" | "timecode" | "text" | "camera" | CommandCueType;

/** Cues whose action is performed on *other* cues (QLab's control cues).
 *  Distinct types — own colour, own row label, 1:1 with QLab on import — over
 *  one shared Rust implementation. Grouped behind the toolbar's "+ Command". */
export type CommandCueType = "start" | "pause" | "resume" | "load" | "reset" | "goto" | "arm" | "disarm";

/** Hints are kept short enough to stay on one line in the toolbar dropdown —
 *  a wrapped row breaks the menu's rhythm. The inspector carries the longer
 *  explanation. */
export const COMMAND_CUE_TYPES: { type: CommandCueType; label: string; hint: string }[] = [
  { type: "start",  label: "Start",  hint: "Trigger the targets" },
  { type: "pause",  label: "Pause",  hint: "Pause the targets" },
  { type: "resume", label: "Resume", hint: "Resume the targets" },
  { type: "load",   label: "Load",   hint: "Bring them up paused" },
  { type: "reset",  label: "Reset",  hint: "Return them to standby" },
  { type: "goto",   label: "Goto",   hint: "Move the Playhead there" },
  { type: "arm",    label: "Arm",    hint: "Enable the targets" },
  { type: "disarm", label: "Disarm", hint: "Disable the targets" },
];

export const isCommandCueType = (t: CueType): t is CommandCueType =>
  COMMAND_CUE_TYPES.some((c) => c.type === t);

/** Accent color per cue type — the single source of truth for the toolbar
 *  buttons (App.tsx) and the cue-list context menu (CueListView.tsx). */
export const CUE_TYPE_COLORS: Record<CueType, string> = {
  audio:    "#3b82f6",          // blue
  video:    "#a78bfa",
  image:    "#86efac",
  stop:     "#ef4444",
  fade:     "#ec4899",
  wait:     "#fb923c",
  group:    "#fde047",
  midi:     "var(--wc-accent)", // the green audio used to be
  osc:      "#06b6d4",
  light:    "#fbbf24",
  mic:      "#22d3ee",          // cyan
  timecode: "#ff2d9c",          // hot pink
  text:     "#e2e8f0",
  memo:     "#e2e8f0",
  camera:   "#34d399",          // emerald
  devamp:   "#facc15",          // amber — the "release the vamp" action
  // Command cues: one hue family so they read as a group in the cue list,
  // shaded by what they do — go, hold, restore, arm.
  start:    "#4ade80",
  resume:   "#4ade80",
  pause:    "#facc15",
  load:     "#eab308",
  reset:    "#38bdf8",
  goto:     "#0ea5e9",
  arm:      "#fb923c",
  disarm:   "#f97316",
};

export type CueState = "standby" | "running" | "paused" | "completed";

export type ContinueMode = "do_not_continue" | "auto_continue" | "auto_follow";

export type CueColor =
  | "none"
  | "red"
  | "orange"
  | "yellow"
  | "green"
  | "cyan"
  | "blue"
  | "purple"
  | "pink"
  | "white"
  | "black";

export type FadeCurve = "linear" | "s_curve" | "exponential";

export type GroupMode = "simultaneous" | "sequential" | "playlist" | "start_random";

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
  notes: string;
  state: CueState;
  continue_mode: ContinueMode;
  color: CueColor;
  pre_wait_ms: number;
  post_wait_ms: number;
  duration_ms: number | null;
  file_path: string | null;
  /** True while the audio file is being decoded in a background thread. */
  is_loading: boolean;
  /** True when this cue is disabled — skipped by the transport on GO. */
  is_disabled: boolean;
  /** True when this cue's media file was assigned but is now missing from disk. */
  is_broken: boolean;
  /** True for non-critical problems (no file assigned, zero duration, empty group). */
  is_warning: boolean;
  /** Human-readable warning description, present when is_warning is true. */
  warning_message?: string;
  /** Output Patch name this cue plays through (explicit or workspace default).
   *  Absent for cue types with no audio output. */
  output_patch_name?: string;
  /** Duration of one loop iteration in ms (raw file duration, no loop multiplier). null for non-media cues. */
  file_duration_ms: number | null;
  /** For Group cues: direct child cue summaries (recursive). */
  children?: CueSummary[];
  /** For Group cues: playback mode. */
  group_mode?: GroupMode;
  /** For Playlist Group cues: whether the playlist loops (wraps last → first). */
  playlist_loop?: boolean;
  /** For running Sequential/Playlist Group cues: ID of the currently active child. */
  active_child_id?: string;
}

/** Full cue data returned by get_cue. */
/** Play count meaning "loop this slice forever" (a vamp) — u32::MAX. */
export const PLAY_COUNT_INFINITE = 4294967295;

/** QLab-style slices: markers split the clip into segments, each with a play
 *  count (PLAY_COUNT_INFINITE = vamp until a Devamp Cue releases it). */
export interface SliceList {
  /** Marker positions in ms from file start, sorted ascending. */
  markers: number[];
  /** Play count per segment (markers.length + 1 entries). */
  play_counts: number[];
}

export const EMPTY_SLICES: SliceList = { markers: [], play_counts: [1] };

export interface AudioCueData extends CueSummary {
  notes: string;
  volume_db: number;
  pan: number;
  /** Crosspoint levels in dB, `[input channel][patch channel]`. `null` = the
   *  cue routes with Pan instead. Replaces pan when set. */
  level_matrix?: number[][] | null;
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
  start_time_ms: number | null;
  end_time_ms: number | null;
  loop_count: number;
  output_patch_id: string | null;
  rate: number;
  slices: SliceList;
}

/** How the source frame is mapped onto the output surface. */
export type FitMode = "fit" | "fill" | "stretch";

/** Per-cue visual geometry for Video and Image cues. */
export interface VideoGeometry {
  fit_mode: FitMode;
  /** Horizontal offset as a fraction of the scaled video width (−1..1). */
  pan_x: number;
  /** Vertical offset as a fraction of the scaled video height (−1..1). */
  pan_y: number;
  /** Linear scale factor (1.0 = 100%). */
  scale: number;
  /** Clockwise rotation in degrees (0–359). */
  rotation: number;
  /** Crop per edge as a fraction of the source size (0..0.45 each). */
  crop_left: number;
  crop_right: number;
  crop_top: number;
  crop_bottom: number;
}

/** Neutral geometry (engine defaults). */
export const DEFAULT_GEOMETRY: VideoGeometry = {
  fit_mode: "fit",
  pan_x: 0,
  pan_y: 0,
  scale: 1,
  rotation: 0,
  crop_left: 0,
  crop_right: 0,
  crop_top: 0,
  crop_bottom: 0,
};

/** How a visual cue's pixels combine with the layers below it. */
export type BlendMode =
  | "normal" | "add" | "multiply" | "screen" | "overlay"
  | "soft_light" | "hard_light" | "darken" | "lighten"
  | "color_dodge" | "color_burn" | "difference" | "exclusion" | "subtract";

/** Compositing properties of a visual cue (QLab layer model). */
export interface LayerStyle {
  /** Stacking order 1–1000 (higher = closer to the viewer). null = automatic (newest on top). */
  layer: number | null;
  /** Base opacity 0.0–1.0. */
  opacity: number;
  blend_mode: BlendMode;
}

/** Default compositing (automatic layer, fully opaque, normal blending). */
export const DEFAULT_LAYER_STYLE: LayerStyle = {
  layer: null,
  opacity: 1,
  blend_mode: "normal",
};

/** Full cue data returned by get_cue for a Video Cue. */
export interface VideoCueData extends CueSummary {
  notes: string;
  volume_db: number;
  /** Crosspoint levels in dB for the video's audio track. See AudioCueData. */
  level_matrix?: number[][] | null;
  /** Audio track fade-in. */
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  /** Audio track fade-out. */
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
  /** Visual (GL overlay) fade-in — independent from audio. */
  video_fade_in_ms: number | null;
  video_fade_in_curve: FadeCurve | null;
  /** Visual (GL overlay) fade-out — independent from audio. */
  video_fade_out_ms: number | null;
  video_fade_out_curve: FadeCurve | null;
  start_time_ms: number | null;
  end_time_ms: number | null;
  loop_count: number;
  output_surface_id: string | null;
  output_patch_id: string | null;
  /** Freeze on the last frame at natural EOF instead of cutting to black. */
  hold_last_frame: boolean;
  geometry: VideoGeometry;
  layer_style: LayerStyle;
  slices: SliceList;
  /** Full media duration (serialized form only — summaries carry
   *  file_duration_ms instead). */
  cached_duration_ms?: number | null;
}

/** 9-point position grid for TextCue. */
export type TextPosition =
  | "top_left" | "top_center" | "top_right"
  | "middle_left" | "center" | "middle_right"
  | "bottom_left" | "bottom_center" | "bottom_right";

/** Full cue data returned by get_cue for a Text Cue. */
export interface TextCueData extends CueSummary {
  notes: string;
  /** Text content to display (multi-line supported). */
  text: string;
  /** Font family name. */
  font: string;
  /** Font size in mpv OSD/ASS points. */
  font_size: number;
  /** Text colour as "#RRGGBB". */
  text_color: string;
  /** Position on the output surface. */
  position: TextPosition;
  /** Target monitor index. null = use workspace display setting. */
  screen_index: number | null;
  /** Auto-complete after this duration in ms. null = hold until stopped. */
  display_duration_ms: number | null;
}

/** Full cue data returned by get_cue for an Image Cue. */
export interface ImageCueData extends CueSummary {
  notes: string;
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
  /** How long the image stays on screen in ms. null = infinite (hold until stopped). */
  display_duration_ms: number | null;
  geometry: VideoGeometry;
  layer_style: LayerStyle;
}

// ---------------------------------------------------------------------------
// Camera types
// ---------------------------------------------------------------------------

/** Where a Camera Cue's live feed comes from (matches the Rust enum tagging). */
export type CameraSource =
  | { kind: "device"; id: string; name: string }
  | { kind: "url"; url: string };

/** One connected capture device (from list_camera_devices). */
export interface CameraDeviceInfo {
  id: string;
  name: string;
}

/** Full cue data returned by get_cue for a Camera Cue. */
export interface CameraCueData extends CueSummary {
  notes: string;
  source: CameraSource;
  /** Visual (GL overlay) fade-in from black. */
  video_fade_in_ms: number | null;
  video_fade_in_curve: FadeCurve | null;
  /** Visual (GL overlay) fade-out to black on stop. */
  video_fade_out_ms: number | null;
  video_fade_out_curve: FadeCurve | null;
  geometry: VideoGeometry;
  layer_style: LayerStyle;
}

// ---------------------------------------------------------------------------
// MIDI types
// ---------------------------------------------------------------------------

export type MidiMessageType = "note_on" | "note_off" | "control_change" | "program_change";

export interface MidiMessage {
  port_name: string;
  message_type: MidiMessageType;
  /** MIDI channel 1–16 */
  channel: number;
  /** Note / CC number / program (0–127) */
  data1: number;
  /** Velocity / CC value (0–127); unused for program_change */
  data2: number;
}

/** Full cue data returned by get_cue for a MIDI Cue. */
export interface MidiCueData extends CueSummary {
  notes: string;
  messages: MidiMessage[];
}

/** Full cue data returned by get_cue for a Fade Cue. */
export interface FadeCueData extends CueSummary {
  notes: string;
  /** UUIDs of cues to fade (empty = no-op). */
  target_cue_ids: string[];
  /** Display labels kept in sync with target_cue_ids. */
  target_cue_numbers: string[];
  /** Target audio volume in dB (−60 = silence, 0 = unity). */
  target_volume_db: number;
  /** Target visual brightness in percent (0 = black, 100 = fully visible). Independent from volume. */
  target_brightness_pct: number;
  /** Target stereo pan (-1 = left, +1 = right); null = leave pan untouched. */
  target_pan: number | null;
  /** When true (default) the fade drives volume toward target_volume_db; false = pan-only. */
  fade_volume: boolean;
  /** Fade duration in milliseconds. */
  fade_duration_ms: number;
  /** Fade curve shape. */
  fade_curve: FadeCurve;
  /** Stop the target cue(s) once the fade completes. */
  stop_at_end: boolean;
}

/** Full cue data returned by get_cue for a Wait Cue. */
export interface WaitCueData extends CueSummary {
  notes: string;
  /** The configured wait duration in milliseconds. */
  wait_duration_ms: number;
}

/** Full cue data returned by get_cue for a Stop Cue. */
export interface StopCueData extends CueSummary {
  notes: string;
  /** UUIDs of cues to stop. Empty = stop all running cues. */
  target_cue_ids: string[];
  /** Display labels kept in sync with target_cue_ids. */
  target_cue_numbers: string[];
  /** true = immediate cut (no fade); false = soft stop with workspace fade-out. */
  hard_stop_mode: boolean;
}

export interface DevampCueData extends CueSummary {
  notes: string;
  /** UUIDs of the cues to devamp. */
  target_cue_ids: string[];
  /** Display labels kept in sync with target_cue_ids. */
  target_cue_numbers: string[];
  /** true = the target stops at the end of its current slice. */
  stop_at_end: boolean;
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
  /** Mixer fader in dB (0 = unity). */
  gain_db: number;
}

/** A named live-audio input mapping (Mic Cues). Mirror of OutputPatch, stored in the workspace. */
export interface InputPatch {
  id: string;
  name: string;
  device_id: string;
  /** Zero-based input device channel indices this patch exposes. */
  channels: number[];
}

/** Full cue data returned by get_cue for a Mic Cue. */
export interface MicCueData extends CueSummary {
  notes: string;
  input_patch_id: string | null;
  /** Device channel indices to take (empty = use the patch's own channels). */
  input_channels: number[];
  output_patch_id: string | null;
  volume_db: number;
  pan: number;
  fade_in_ms: number | null;
  fade_in_curve: FadeCurve | null;
  fade_out_ms: number | null;
  fade_out_curve: FadeCurve | null;
}

export type CueListMode = "sequential" | "cart";

export interface CueListSummary {
  id: string;
  name: string;
  mode: CueListMode;
}

export interface WorkspaceInfo {
  name: string;
  is_modified: boolean;
  file_path: string | null;
}

/** Metadata for an unsaved-work snapshot left by an abnormally-terminated session. */
export interface RecoveryInfo {
  name: string;
  original_path: string | null;
  modified_at: string | null;
}

// --- Preflight (Check Workspace) -------------------------------------------

export type Severity = "error" | "warning";

export interface CueIssue {
  severity: Severity;
  message: string;
}

/** One cue's preflight result (only cues with at least one issue are returned). */
export interface CueValidation {
  cue_id: CueId;
  cue_number: string | null;
  cue_name: string;
  cue_type: CueType;
  issues: CueIssue[];
  /** The unresolved media path when the problem is a missing file (drives relink). */
  missing_file: string | null;
}

export interface RelinkResult {
  relinked: number;
}

// --- Logs ------------------------------------------------------------------

export interface LogLine {
  ts: string;
  level: string;
  target: string;
  message: string;
}

// --- Runtime health (device/network faults) --------------------------------

export type HealthLevel = "error" | "warning" | "info";

export interface HealthAlert {
  key: string;
  level: HealthLevel;
  message: string;
  /** Action id the banner maps to a command (e.g. "restore_audio_device"). */
  action: string | null;
  action_label: string | null;
}

export interface CollectReport {
  workspace_path: string;
  files_copied: number;
  files_skipped: number;
  files_missing: string[];
}

export interface WaveformData {
  peaks: number[];
  /** RMS per bin — the "body" of the sound, drawn inside the peak envelope. */
  rms: number[];
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
  /** Send rate (Hz) for /inkue/cue/{i}/progress|elapsed|remaining|duration.
   *  0 disables progress feedback. */
  feedback_progress_hz: number;
}

/** One IPv4-capable network interface (Preferences → Network). */
export interface NetworkInterfaceInfo {
  name: string;
  ip: string;
  is_loopback: boolean;
}

/** Machine-level network interface selection. All-null = Automatic. */
export interface NetworkInterfaceConfig {
  interface_name: string | null;
  interface_ip: string | null;
}

// ---------------------------------------------------------------------------
// DMX / Lighting
// ---------------------------------------------------------------------------

export type OutputProtocol = "Sacn" | "ArtNet";

/** One workspace-level universe output mapping (matches `engine::dmx_sink::UniverseOutput`). */
export interface UniverseOutput {
  universe: number;
  protocol: OutputProtocol;
  /** Destination IP string, or null for the sACN multicast group. */
  destination: string | null;
  enabled: boolean;
}

/** Live output bytes of one universe, pushed via the `dmx-monitor` event. */
export interface DmxUniverseSnapshot {
  universe: number;
  channels: number[];
}

/** Resolution of one fixture parameter on the wire (matches `engine::dmx_engine::ChannelWidth`). */
export type ChannelWidth = "Bit8" | "Bit16";

/** What a fixture parameter controls. */
export type ParamKind =
  | "intensity"
  | "red"
  | "green"
  | "blue"
  | "white"
  | "amber"
  | "uv"
  | "pan"
  | "tilt"
  | "generic";

/** One controllable parameter of a fixture, offset from its base address. */
export interface FixtureParam {
  kind: ParamKind;
  name: string;
  channel_offset: number;
  width: ChannelWidth;
  default: number;
}

/** The channel layout of a kind of lighting instrument. */
export interface FixtureType {
  name: string;
  parameters: FixtureParam[];
}

/** A fixture placed at a DMX address in the workspace patch. */
export interface PatchedFixture {
  id: string;
  label: string;
  universe: number;
  base_address: number;
  fixture_type: FixtureType;
}

/** A detected address clash between two patched fixtures. */
export interface FixtureConflict {
  fixture_a: string;
  fixture_b: string;
  universe: number;
  message: string;
}

/** A named set of fixtures driven together by one Light Cue control. */
export interface FixtureGroup {
  id: string;
  label: string;
  fixture_ids: string[];
}

/** One thing a Light Cue drives: a fixture parameter, or a group parameter-kind. */
export type ParamTarget =
  | { kind: "fixture"; fixture_id: string; param_index: number; value: number }
  | { kind: "group"; group_id: string; param_kind: ParamKind; value: number };

/** Full cue data returned by get_cue for a Light Cue. */
export interface LightCueData extends CueSummary {
  notes: string;
  targets: ParamTarget[];
  fade: FadeSpec;
}

// ---------------------------------------------------------------------------
// Timecode
// ---------------------------------------------------------------------------

export type TcRate = "24" | "25" | "29.97" | "29.97df" | "30";

export type TcSource = "mtc" | "ltc";

export interface TcPosition {
  h: number;
  m: number;
  s: number;
  f: number;
  rate: TcRate;
}

export interface TcTrigger {
  /** SMPTE string HH:MM:SS:FF or HH:MM:SS;FF */
  position: string;
  /** true = position was entered as Real Time (ms) */
  real_time: boolean;
  rate: TcRate;
}

export type TcOnStop = "continue" | "pause" | "stop";

export interface CueListTcConfig {
  enabled: boolean;
  rate: TcRate;
  freewheel_ms: number;
  on_stop: TcOnStop;
}

export type TcOutputType = "mtc" | "ltc";

export interface TimecodeCueData extends CueSummary {
  tc_type: TcOutputType;
  midi_port: string | null;
  output_patch_id: string | null;
  rate: TcRate;
  /** SMPTE string */
  start_frame: TcPosition;
  end_frame: TcPosition | null;
}

export interface TcMachineConfig {
  enabled: boolean;
  receiver_config: {
    source: TcSource;
    midi_port: string | null;
    ltc_device_id: string | null;
  };
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

export type AudioBackend = "wasapi_shared" | "wasapi_exclusive" | "asio";

/** Hardware-specific settings — stored in %APPDATA%\Inkue\audio.json, not in the workspace. */
export interface MachineAudioConfig {
  backend: AudioBackend;
  device_id: string | null;
  /** Friendly name of the selected output device, captured at selection time. */
  device_name: string | null;
  /** Selected audio input device for Mic Cues. null = system default input. */
  input_device_id: string | null;
  /** Samples. Only applied for WASAPI Exclusive. */
  buffer_size: number;
  /** ASIO output pair index (0 = Out 1-2, 1 = Out 3-4, …). */
  asio_out_pair: number;
}

export const DEFAULT_MACHINE_AUDIO_CONFIG: MachineAudioConfig = {
  backend: "wasapi_shared",
  device_id: null,
  device_name: null,
  input_device_id: null,
  buffer_size: 256,
  asio_out_pair: 0,
};

/** Show-specific audio defaults — travel with the .inkue workspace file. */
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
  /** When true, reordering cues rewrites all numbers to position; when false
   * (default), numbers are stable and resequencing is an explicit action. */
  auto_renumber_on_reorder: boolean;
}

export const DEFAULT_GENERAL_PREFS: GeneralPreferences = {
  double_go_protection_ms: 500,
  confirm_before_delete: false,
  auto_scroll_to_playhead: true,
  cue_row_height: "normal",
  auto_renumber_on_reorder: false,
};

export type TimerPosition = "center" | "top_left" | "top_right" | "bottom_left" | "bottom_right";

/** How a cue's colour tag is rendered in the Cue List. */
export type CueColorStyle = "stripe" | "full_row";

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
  /** UI colour theme: "dark", "light", or "system". */
  theme: "dark" | "light" | "system";
  /** How a cue's colour tag is rendered in the Cue List. */
  cue_color_style: CueColorStyle;
  /** Global projector-alignment transform, composed on top of per-cue geometry. */
  output_transform?: OutputTransform;
}

/** Global projector-alignment transform (Preferences → Display). */
export interface OutputTransform {
  /** Horizontal offset as a fraction of the output width (−1..1). */
  pan_x: number;
  /** Vertical offset as a fraction of the output height (−1..1). */
  pan_y: number;
  /** Linear scale factor (1.0 = 100%). */
  scale: number;
  /** Clockwise rotation in degrees — fractional values supported (0.1° steps). */
  rotation: number;
  /**
   * Corner-pin offsets in fractions of the output size, storage order
   * TL, TR, BL, BR; positive = right/down. Applied after scale/rotation/pan.
   */
  corners: [[number, number], [number, number], [number, number], [number, number]];
}

/** Identity transform (engine defaults). */
export const DEFAULT_OUTPUT_TRANSFORM: OutputTransform = {
  pan_x: 0,
  pan_y: 0,
  scale: 1,
  rotation: 0,
  corners: [[0, 0], [0, 0], [0, 0], [0, 0]],
};

/** Built-in calibration patterns; custom_image carries the file path. */
export type TestPatternKind =
  | "grid" | "smpte_bars" | "rgb_test" | "test_card"
  | "white" | "gray" | "black" | "custom_image";

/** Serialised form matching the Rust `TestPattern` enum (tag + content). */
export interface TestPattern {
  kind: TestPatternKind;
  /** Only for custom_image. */
  path?: string;
}

export const DEFAULT_DISPLAY_PREFS: Pick<DisplayPreferences, "theme" | "cue_color_style"> = {
  theme: "system",
  cue_color_style: "stripe",
};

export interface AppPreferences {
  audio: AudioPreferences;
  general: GeneralPreferences;
  network: Record<string, never>;
  display: DisplayPreferences;
}

// ---------------------------------------------------------------------------
// Events emitted by the backend
export interface CueListsChangedEvent {
  cue_lists: CueListSummary[];
  active_cue_list_id: string;
}

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

/** Key pressed while the output window has focus, forwarded by the backend. */
export interface OutputKeyEvent {
  key: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export interface CueTimeUpdateEvent {
  cue_id: CueId;
  elapsed_ms: number;
  action_elapsed_ms: number;
  remaining_ms: number;
}
