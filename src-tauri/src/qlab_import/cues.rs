//! QLab cue → Inkue cue JSON.
//!
//! A port of the reference implementation in `qlab2inkue/qlab_to_inkue.py`,
//! which is where a new QLab property should be decoded and proven first (it
//! can decode an unknown `.qlab5` offline, and its `test_mapping.py` runs
//! against a fixture holding one cue of every type).
//!
//! Two rules the whole module follows:
//!
//! - **Nothing is ever dropped silently.** A cue with no Inkue counterpart
//!   becomes a clearly-named Memo carrying whatever it can, so the operator
//!   sees what needs rebuilding and where.
//! - **Nothing is pretended.** A QLab Script cue holds AppleScript, which has
//!   no command-line equivalent; it is imported disarmed with the script in
//!   the notes rather than as a cue that would silently do nothing.

use serde_json::{json, Map, Value};

use super::patches::Patches;

/// QLab `continueMode` → Inkue.
fn continue_mode(cue: &Value) -> &'static str {
    match cue.get("continueMode").and_then(Value::as_i64).unwrap_or(0) {
        1 => "auto_continue",
        2 => "auto_follow",
        _ => "do_not_continue",
    }
}

const COLORS: [&str; 11] = [
    "none", "red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white", "black",
];

fn color(cue: &Value) -> String {
    let name = cue
        .get("colorName")
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_lowercase();
    if COLORS.contains(&name.as_str()) { name } else { "none".into() }
}

/// Seconds → milliseconds, treating 0 and absent alike (QLab writes both).
fn ms(cue: &Value, key: &str) -> Option<u64> {
    let seconds = cue.get(key).and_then(Value::as_f64)?;
    if seconds == 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn ms_or_zero(cue: &Value, key: &str) -> u64 {
    ms(cue, key).unwrap_or(0)
}

/// Reuse QLab's `uniqueID` as the Inkue cue id — it is already a UUID, which
/// is what makes every cue target resolve without a lookup table.
fn cue_id(cue: &Value) -> String {
    cue.get("uniqueID")
        .and_then(Value::as_str)
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(uuid::Uuid::new_v4)
        .to_string()
}

/// `cueTargetUniqueID` as Inkue's target list.
fn target_ids(cue: &Value) -> Vec<String> {
    match cue.get("cueTargetUniqueID").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => vec![uuid::Uuid::parse_str(id)
            .unwrap_or_else(|_| uuid::Uuid::new_v4())
            .to_string()],
        _ => Vec::new(),
    }
}

fn loop_count(cue: &Value) -> u64 {
    if cue.get("infiniteLoop").and_then(Value::as_bool).unwrap_or(false) {
        return u32::MAX as u64;
    }
    let plays = cue.get("playCount").and_then(Value::as_i64).unwrap_or(1);
    plays.saturating_sub(1).max(0) as u64
}

/// The fields every cue type shares.
fn common(cue: &Value, cue_type: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("type".into(), json!(cue_type));
    out.insert("cue_type".into(), json!(cue_type));
    out.insert("id".into(), json!(cue_id(cue)));
    let number = cue.get("number").and_then(Value::as_str).filter(|s| !s.is_empty());
    out.insert("number".into(), json!(number));
    out.insert("name".into(), json!(cue.get("name").and_then(Value::as_str).unwrap_or("")));
    out.insert("notes".into(), json!(cue.get("notes").and_then(Value::as_str).unwrap_or("")));
    out.insert("color".into(), json!(color(cue)));
    out.insert("pre_wait_ms".into(), json!(ms_or_zero(cue, "preWait")));
    out.insert("post_wait_ms".into(), json!(ms_or_zero(cue, "postWait")));
    out.insert("continue_mode".into(), json!(continue_mode(cue)));
    // QLab arms a cue; Inkue disables it.
    let armed = cue.get("armed").and_then(Value::as_bool).unwrap_or(true);
    out.insert("is_disabled".into(), json!(!armed));
    out
}

/// Append an explanatory line to a cue's notes without losing what is there.
fn add_note(cue_json: &mut Map<String, Value>, extra: &str) {
    let existing = cue_json.get("notes").and_then(Value::as_str).unwrap_or("").to_string();
    let combined = if existing.is_empty() {
        extra.to_string()
    } else {
        format!("{existing}\n\n{extra}")
    };
    cue_json.insert("notes".into(), json!(combined));
}

// ---------------------------------------------------------------------------
// Media paths
// ---------------------------------------------------------------------------

/// Bundle-relative media path, across QLab 4 and 5.
///
/// QLab 5 wraps the target in an `F53Alias` carrying `relativePath`; QLab 4
/// uses an `NSURL`. Either way a bundle-relative path is what we want: the
/// caller resolves it against the workspace folder.
pub fn media_path(cue: &Value) -> Option<String> {
    let target = cue.get("fileTarget")?;
    if let Some(rel) = target.get("relativePath").and_then(Value::as_str) {
        if !rel.is_empty() {
            return Some(rel.to_string());
        }
    }
    if target.get("__class__").and_then(Value::as_str) == Some("NSURL") {
        if let Some(rel) = target.get("relative").and_then(Value::as_str) {
            return Some(rel.replace("file://", ""));
        }
    }
    // Last resort: keep from the last media folder of an absolute path.
    let last_known = target.get("lastKnownPath").and_then(Value::as_str)?;
    Some(relative_from_absolute(last_known))
}

fn relative_from_absolute(path: &str) -> String {
    let normalised = path.replace('\\', "/");
    let lower = normalised.to_lowercase();
    for folder in ["/audio/", "/video/", "/midi file/", "/midi/"] {
        if let Some(index) = lower.rfind(folder) {
            return normalised[index + 1..].to_string();
        }
    }
    normalised.rsplit('/').next().unwrap_or(&normalised).to_string()
}

// ---------------------------------------------------------------------------
// Audio levels
// ---------------------------------------------------------------------------

/// Linear gain at a crosspoint of an `AudioLevelMatrix`.
fn crosspoint(matrix: &Value, row: i64, column: i64) -> Option<f64> {
    let entries = matrix.get("entries")?;
    let values: Vec<&Value> = match entries {
        Value::Object(map) => map.values().collect(),
        Value::Array(items) => items.iter().collect(),
        _ => return None,
    };
    for entry in values {
        if entry.get("row").and_then(Value::as_i64) == Some(row)
            && entry.get("column").and_then(Value::as_i64) == Some(column)
        {
            let initial = entry.get("initialLevel").and_then(Value::as_f64).unwrap_or(1.0);
            let trim = entry.get("trimLevel").and_then(Value::as_f64).unwrap_or(1.0);
            return Some(initial * trim);
        }
    }
    None
}

/// Master level in dB. QLab silence (0.0 linear) maps to Inkue's -60 floor; a
/// missing matrix means unity. This is the level a following Fade ramps *from*,
/// so a wrong value here silently breaks fade-ins.
pub fn master_db(cue: &Value) -> f64 {
    let Some(matrix) = cue.get("levels") else { return 0.0 };
    match crosspoint(matrix, 0, 0) {
        None => 0.0,
        Some(gain) if gain <= 0.0 => -60.0,
        Some(gain) => (20.0 * gain.log10()).max(-60.0),
    }
}

// ---------------------------------------------------------------------------
// Per-type mappings
// ---------------------------------------------------------------------------

pub fn audio(cue: &Value, pan_starts: &std::collections::HashMap<String, f64>) -> Value {
    let mut out = common(cue, "audio");
    out.insert("file_path".into(), json!(media_path(cue)));
    out.insert("volume_db".into(), json!(master_db(cue)));
    // A following pan fade's start position wins over the cue's own matrix.
    out.insert("pan".into(), json!(start_pan(cue, pan_starts)));
    out.insert("fade_in_ms".into(), Value::Null);
    out.insert("fade_in_curve".into(), Value::Null);
    out.insert("fade_out_ms".into(), Value::Null);
    out.insert("fade_out_curve".into(), Value::Null);
    out.insert("start_time_ms".into(), json!(ms(cue, "startTime")));
    out.insert("end_time_ms".into(), json!(ms(cue, "endTime")));
    out.insert("loop_count".into(), json!(loop_count(cue)));
    out.insert("output_patch_id".into(), Value::Null);
    out.insert("rate".into(), json!(cue.get("rate").and_then(Value::as_f64).unwrap_or(1.0)));
    Value::Object(out)
}

pub fn video(cue: &Value) -> Value {
    let mut out = common(cue, "video");
    out.insert("file_path".into(), json!(media_path(cue)));
    out.insert("volume_db".into(), json!(0.0));
    out.insert("start_time_ms".into(), json!(ms(cue, "startTime")));
    out.insert("end_time_ms".into(), json!(ms(cue, "endTime")));
    out.insert("loop_count".into(), json!(loop_count(cue)));
    out.insert("output_patch_id".into(), Value::Null);
    Value::Object(out)
}

pub fn camera(cue: &Value, patches: &Patches) -> Value {
    let mut out = common(cue, "camera");
    let device = patches.camera_device(cue.get("cameraPatchID").and_then(Value::as_str));
    out.insert(
        "source".into(),
        json!({ "kind": "device", "id": device, "name": device }),
    );
    if !device.is_empty() {
        add_note(
            &mut out,
            &format!("[QLab camera patch: \"{device}\" — a macOS capture device name; \
                      re-pick the camera on Windows or Linux]"),
        );
    }
    Value::Object(out)
}

pub fn memo(cue: &Value, text: &str) -> Value {
    let mut out = common(cue, "memo");
    out.insert("memo_text".into(), json!(text));
    Value::Object(out)
}

pub fn wait(cue: &Value) -> Value {
    let mut out = common(cue, "wait");
    out.insert("wait_duration_ms".into(), json!(ms_or_zero(cue, "duration")));
    Value::Object(out)
}

/// Start / Pause / Load / Reset / Goto / Arm / Disarm — one Inkue type each.
pub fn control(cue: &Value, cue_type: &str) -> Value {
    let mut out = common(cue, cue_type);
    out.insert("target_cue_ids".into(), json!(target_ids(cue)));
    out.insert("target_cue_numbers".into(), json!(Vec::<String>::new()));
    Value::Object(out)
}

pub fn stop(cue: &Value) -> Value {
    let mut out = common(cue, "stop");
    out.insert("target_cue_ids".into(), json!(target_ids(cue)));
    out.insert("target_cue_numbers".into(), json!(Vec::<String>::new()));
    // QLab's Stop uses the target's own fade time; Inkue's soft stop matches.
    out.insert("hard_stop_mode".into(), json!(false));
    Value::Object(out)
}

pub fn devamp(cue: &Value) -> Value {
    let mut out = common(cue, "devamp");
    out.insert("target_cue_ids".into(), json!(target_ids(cue)));
    out.insert("target_cue_numbers".into(), json!(Vec::<String>::new()));
    out.insert(
        "stop_at_end".into(),
        json!(cue.get("stopTargetWhenDone").and_then(Value::as_bool).unwrap_or(false)),
    );
    Value::Object(out)
}

pub fn script(cue: &Value) -> Value {
    let mut out = common(cue, "script");
    out.insert("command".into(), json!(""));
    out.insert("args".into(), json!(Vec::<String>::new()));
    out.insert("working_dir".into(), Value::Null);
    out.insert("timeout_ms".into(), json!(30_000));
    // Disarmed on purpose: a cue that silently did nothing would be worse.
    out.insert("is_disabled".into(), json!(true));
    let source = cue.get("source").and_then(Value::as_str).unwrap_or("");
    add_note(
        &mut out,
        &format!(
            "[QLab AppleScript — no command-line equivalent; rewrite this as an \
             executable + arguments, then re-arm the cue]\n\n{source}"
        ),
    );
    Value::Object(out)
}

/// Type an OSC argument the way QLab's single message string implies.
fn osc_arg(token: &str) -> Value {
    if let Ok(int) = token.parse::<i64>() {
        return json!({ "type": "int", "value": int });
    }
    if let Ok(float) = token.parse::<f64>() {
        return json!({ "type": "float", "value": float });
    }
    json!({ "type": "str", "value": token })
}

pub fn osc(cue: &Value, patches: &Patches) -> Value {
    let mut out = common(cue, "osc");
    let raw = cue.get("oscString").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let patch_id = patches.osc_patch(cue.get("networkPatchID").and_then(Value::as_str));
    let mut parts = raw.split_whitespace();
    let messages = match parts.next() {
        Some(address) => json!([{
            "patch_id": patch_id,
            "address": address,
            "args": parts.map(osc_arg).collect::<Vec<_>>(),
        }]),
        None => json!([]),
    };
    out.insert("messages".into(), messages);
    if cue.get("plainTextString").and_then(Value::as_str).is_some()
        || cue.get("hexCodesString").and_then(Value::as_str).is_some()
    {
        add_note(
            &mut out,
            "[QLab sent raw UDP/hex here — Inkue's Network cue is OSC only, so \
             that payload was not converted]",
        );
    }
    Value::Object(out)
}

/// QLab's voice-message index follows the MIDI status nibbles.
fn midi_message_type(status: i64) -> Option<&'static str> {
    match status {
        0 => Some("note_off"),
        1 => Some("note_on"),
        3 => Some("control_change"),
        4 => Some("program_change"),
        _ => None,
    }
}

pub fn midi(cue: &Value, patches: &Patches) -> Value {
    let mut out = common(cue, "midi");
    let status = cue.get("status").and_then(Value::as_i64).unwrap_or(-1);
    let Some(kind) = midi_message_type(status) else {
        out.insert("messages".into(), json!([]));
        add_note(
            &mut out,
            &format!(
                "[QLab MIDI message type {status} (MSC / SysEx / pressure / pitch bend) \
                 has no Inkue equivalent — not converted]"
            ),
        );
        return Value::Object(out);
    };
    let mut data1 = cue.get("byte1").and_then(Value::as_i64).unwrap_or(0);
    let mut data2 = cue.get("byte2").and_then(Value::as_i64).unwrap_or(0);
    if kind == "control_change" && data1 == 0 {
        if let Some(number) = cue.get("controlNumber").and_then(Value::as_i64) {
            if number != 0 {
                data1 = number;
                data2 = cue.get("controlValue").and_then(Value::as_i64).unwrap_or(0);
            }
        }
    }
    out.insert(
        "messages".into(),
        json!([{
            "port_name": patches.midi_port(cue.get("midiPatchID").and_then(Value::as_str)),
            "message_type": kind,
            "channel": cue.get("channel").and_then(Value::as_i64).unwrap_or(1).clamp(1, 16),
            "data1": data1.clamp(0, 127),
            "data2": data2.clamp(0, 127),
        }]),
    );
    Value::Object(out)
}

pub fn midi_file(cue: &Value, patches: &Patches) -> Value {
    let mut out = common(cue, "midi_file");
    out.insert("file_path".into(), json!(media_path(cue)));
    out.insert(
        "port_name".into(),
        json!(patches.midi_port(cue.get("midiPatchID").and_then(Value::as_str))),
    );
    out.insert(
        "playback_rate".into(),
        json!(cue.get("playbackRate").and_then(Value::as_f64).unwrap_or(1.0)),
    );
    Value::Object(out)
}

pub fn titles(cue: &Value) -> Value {
    let mut out = common(cue, "text");
    let font = cue
        .get("lastSeenFontNames")
        .and_then(Value::as_array)
        .and_then(|fonts| fonts.first())
        .and_then(Value::as_str)
        .unwrap_or("Helvetica");
    out.insert(
        "text".into(),
        json!(cue.get("titlesAttributedString").and_then(Value::as_str).unwrap_or("")),
    );
    out.insert("font".into(), json!(font));
    out.insert("font_size".into(), json!(48));
    out.insert("text_color".into(), json!("#FFFFFF"));
    out.insert("position".into(), json!("center"));
    out.insert("screen_index".into(), json!(0));
    out.insert("display_duration_ms".into(), json!(ms(cue, "duration")));
    Value::Object(out)
}

pub fn mic(cue: &Value) -> Value {
    let mut out = common(cue, "mic");
    let count = cue.get("channels").and_then(Value::as_i64).unwrap_or(1).max(1);
    let offset = cue.get("channelOffset").and_then(Value::as_i64).unwrap_or(0);
    let channels: Vec<i64> = (offset..offset + count).collect();
    out.insert("input_patch_id".into(), Value::Null);
    out.insert("input_channels".into(), json!(channels));
    out.insert("output_patch_id".into(), Value::Null);
    out.insert("volume_db".into(), json!(master_db(cue)));
    out.insert("pan".into(), json!(0.0));
    out.insert("fade_in_ms".into(), Value::Null);
    out.insert("fade_in_curve".into(), Value::Null);
    out.insert("fade_out_ms".into(), Value::Null);
    out.insert("fade_out_curve".into(), Value::Null);
    add_note(
        &mut out,
        "[Input Patch not set — QLab's audio input patch has no Inkue equivalent \
         to import; pick one in the Mic tab]",
    );
    Value::Object(out)
}

// ---------------------------------------------------------------------------
// Fade
// ---------------------------------------------------------------------------

/// The `live` fade entries a QLab Fade cue actually animates.
fn live_fade_entries(cue: &Value) -> Vec<&Value> {
    let Some(entries) = cue.pointer("/fade/entries") else { return Vec::new() };
    let values: Vec<&Value> = match entries {
        Value::Object(map) => map.values().collect(),
        Value::Array(items) => items.iter().collect(),
        _ => return Vec::new(),
    };
    values
        .into_iter()
        .filter(|e| e.get("live").and_then(Value::as_bool).unwrap_or(false))
        .collect()
}

fn linear_to_db(gain: f64) -> f64 {
    if gain <= 0.0 { -60.0 } else { (20.0 * gain.log10()).max(-60.0) }
}

/// Target master level in dB, or `None` when the fade does not touch the
/// master crosspoint (a pan or geometry fade).
fn fade_target_db(cue: &Value) -> Option<f64> {
    live_fade_entries(cue)
        .into_iter()
        .find(|e| {
            e.get("row").and_then(Value::as_i64) == Some(0)
                && e.get("column").and_then(Value::as_i64) == Some(0)
        })
        .map(|e| linear_to_db(e.get("endValue").and_then(Value::as_f64).unwrap_or(1.0)))
}

/// Inkue pan (-1 left … +1 right) from a linear left/right output pair.
fn pan_from_lr(left: f64, right: f64) -> f64 {
    let total = left + right;
    if total <= 0.0 { 0.0 } else { ((right - left) / total).clamp(-1.0, 1.0) }
}

/// The live row-0 crosspoints on *output* columns, sorted left to right.
///
/// A pan fade animates those rather than the master, so the lowest active
/// column is L and the highest is R.
fn pan_entries(cue: &Value) -> Vec<&Value> {
    let mut entries: Vec<&Value> = live_fade_entries(cue)
        .into_iter()
        .filter(|e| {
            e.get("row").and_then(Value::as_i64) == Some(0)
                && e.get("column").and_then(Value::as_i64).unwrap_or(0) > 0
        })
        .collect();
    entries.sort_by_key(|e| e.get("column").and_then(Value::as_i64).unwrap_or(0));
    entries
}

/// Destination pan of a QLab pan fade, or `None` if it is not one.
fn fade_target_pan(cue: &Value) -> Option<f64> {
    let entries = pan_entries(cue);
    if entries.len() < 2 {
        return None;
    }
    let value = |e: &Value| e.get("endValue").and_then(Value::as_f64).unwrap_or(0.0);
    Some(pan_from_lr(value(entries[0]), value(entries[entries.len() - 1])))
}

/// Starting pan of a QLab pan fade.
///
/// QLab applies these at the fade's onset, which is how a cue "begins in the
/// left speaker" even though its own matrix is centred. Inkue's fade reads the
/// voice's *current* pan at GO, so this is lifted onto the target cue's initial
/// pan by [`collect_pan_starts`].
fn fade_start_pan(cue: &Value) -> Option<f64> {
    let entries = pan_entries(cue);
    if entries.len() < 2 {
        return None;
    }
    let value = |e: &Value| e.get("startValue").and_then(Value::as_f64).unwrap_or(0.0);
    Some(pan_from_lr(value(entries[0]), value(entries[entries.len() - 1])))
}

/// Walk the cue tree recording each pan fade's start pan against its target,
/// so an Audio cue can inherit the position its following pan fade begins at.
pub fn collect_pan_starts(cue: &Value, out: &mut std::collections::HashMap<String, f64>) {
    if cue.get("__class__").and_then(Value::as_str) == Some("FadeCue") {
        if let (Some(pan), Some(target)) = (fade_start_pan(cue), cue.get("cueTargetUniqueID")) {
            if let Some(id) = target.as_str().filter(|s| !s.is_empty()) {
                out.entry(normalise_id(id)).or_insert(pan);
            }
        }
    }
    for child in cue.get("cues").and_then(Value::as_array).unwrap_or(&Vec::new()) {
        collect_pan_starts(child, out);
    }
}

fn normalise_id(id: &str) -> String {
    uuid::Uuid::parse_str(id)
        .unwrap_or_else(|_| uuid::Uuid::new_v4())
        .to_string()
}

/// A QLab Fade cue. A pan-only fade must not move the level, hence
/// `fade_volume: false` when the master crosspoint is untouched.
pub fn fade(cue: &Value) -> Value {
    let mut out = common(cue, "fade");
    let target_db = fade_target_db(cue);
    let target_pan = fade_target_pan(cue);
    out.insert("target_cue_ids".into(), json!(target_ids(cue)));
    out.insert("target_cue_numbers".into(), json!(Vec::<String>::new()));
    out.insert("target_volume_db".into(), json!(target_db.unwrap_or(0.0)));
    out.insert("target_brightness_pct".into(), json!(100.0));
    out.insert("target_pan".into(), json!(target_pan));
    out.insert("fade_volume".into(), json!(target_db.is_some()));
    out.insert("fade_duration_ms".into(), json!(ms_or_zero(cue, "duration")));
    // QLab's default shape. Reading upShape/downShape is still open.
    out.insert("fade_curve".into(), json!("s_curve"));
    out.insert(
        "stop_at_end".into(),
        json!(cue.get("stopTargetWhenDone").and_then(Value::as_bool).unwrap_or(false)),
    );
    if target_db.is_none() && target_pan.is_none() {
        add_note(
            &mut out,
            "[QLab geometry fade — Inkue fades level, pan and visual opacity, \
             so this fade's target was not converted]",
        );
    }
    Value::Object(out)
}

/// The pan an Audio cue should start at, given the pan-start table.
pub fn start_pan(cue: &Value, pan_starts: &std::collections::HashMap<String, f64>) -> f64 {
    pan_starts.get(&cue_id(cue)).copied().unwrap_or(0.0)
}

/// `F53Timecode` counts ticks of 1/48,000,000 s of **real** time.
const F53_TICKS_PER_SECOND: f64 = 48_000_000.0;

/// `F53Timecode` → Inkue `TcPosition` JSON.
///
/// At "video speed" (the 1000/1001 pull-down) the timecode clock runs slower
/// than real time, so real seconds must be divided by 1.001 to get the label:
/// 172,972,800,000 ticks = 3603.6 s real = exactly 01:00:00:00 at 29.97.
pub fn timecode_position(tc: Option<&Value>) -> Value {
    let Some(tc) = tc else { return default_position() };
    let framerate = tc.get("F53TimecodeFramerate");
    let fps = framerate
        .and_then(|f| f.get("F53FramerateFramesPerSecond"))
        .and_then(Value::as_i64)
        .unwrap_or(30)
        .max(1);
    let drop_frame = framerate
        .and_then(|f| f.get("F53FramerateDropFrame"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let video_speed = framerate
        .and_then(|f| f.get("F53FramerateVideoSpeed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(ticks) = tc.get("F53TimecodeCommonTicks").and_then(Value::as_f64) else {
        return default_position();
    };

    let mut seconds = ticks / F53_TICKS_PER_SECOND;
    if video_speed {
        seconds /= 1.001;
    }
    let total_frames = (seconds * fps as f64).round() as i64;
    let rate = match (fps, video_speed, drop_frame) {
        (30, true, true) => "29.97df",
        (30, true, false) => "29.97",
        (24, _, _) => "24",
        (25, _, _) => "25",
        _ => "30",
    };
    json!({
        "h": (total_frames / (3600 * fps)) % 24,
        "m": (total_frames / (60 * fps)) % 60,
        "s": (total_frames / fps) % 60,
        "f": total_frames % fps,
        "rate": rate,
    })
}

fn default_position() -> Value {
    json!({ "h": 0, "m": 0, "s": 0, "f": 0, "rate": "29.97df" })
}

pub fn timecode(cue: &Value, patches: &Patches) -> Value {
    let mut out = common(cue, "timecode");
    let start = timecode_position(cue.get("startTimecode"));
    let rate = start.get("rate").cloned().unwrap_or(json!("29.97df"));
    let port = patches.midi_port(cue.get("midiPatchID").and_then(Value::as_str));
    let is_ltc = cue.get("outputType").and_then(Value::as_i64).unwrap_or(0) != 0;
    out.insert("tc_type".into(), json!(if is_ltc { "ltc" } else { "mtc" }));
    out.insert("midi_port".into(), if port.is_empty() { Value::Null } else { json!(port) });
    out.insert("output_patch_id".into(), Value::Null);
    out.insert("rate".into(), rate);
    out.insert("start_frame".into(), start);
    out.insert("end_frame".into(), Value::Null);
    Value::Object(out)
}

/// A QLab type with no Inkue counterpart, kept as an explicitly named Memo.
pub fn unconvertible(cue: &Value, label: &str, detail: &str) -> Value {
    let text = if detail.is_empty() {
        format!("[{label}]")
    } else {
        format!("[{label}] {detail}")
    };
    let mut out = match memo(cue, &text) {
        Value::Object(map) => map,
        _ => unreachable!("memo always builds an object"),
    };
    let name = cue.get("name").and_then(Value::as_str).unwrap_or("");
    out.insert("name".into(), json!(format!("[{label}] {name}").trim().to_string()));
    if !detail.is_empty() {
        add_note(&mut out, detail);
    }
    Value::Object(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_mode_maps_qlabs_integers() {
        assert_eq!(continue_mode(&json!({ "continueMode": 0 })), "do_not_continue");
        assert_eq!(continue_mode(&json!({ "continueMode": 1 })), "auto_continue");
        assert_eq!(continue_mode(&json!({ "continueMode": 2 })), "auto_follow");
        assert_eq!(continue_mode(&json!({})), "do_not_continue");
    }

    #[test]
    fn armed_becomes_not_disabled() {
        let out = common(&json!({ "armed": false }), "memo");
        assert_eq!(out["is_disabled"], json!(true));
        let out = common(&json!({ "armed": true }), "memo");
        assert_eq!(out["is_disabled"], json!(false));
    }

    #[test]
    fn a_qlab_unique_id_is_reused_as_the_cue_id() {
        // This is what makes every control cue's target resolve for free.
        let id = "68a9990a-7835-46ae-b9b8-2e231407d2fe";
        let cue = json!({ "uniqueID": id.to_uppercase() });
        assert_eq!(cue_id(&cue), id);
        assert_eq!(target_ids(&json!({ "cueTargetUniqueID": id })), vec![id]);
    }

    #[test]
    fn infinite_loop_becomes_the_sentinel() {
        assert_eq!(loop_count(&json!({ "infiniteLoop": true })), u32::MAX as u64);
        assert_eq!(loop_count(&json!({ "playCount": 3 })), 2);
        assert_eq!(loop_count(&json!({})), 0);
    }

    #[test]
    fn master_level_reads_the_matrix_and_floors_silence() {
        let unity = json!({ "levels": { "entries": {
            "0": { "row": 0, "column": 0, "initialLevel": 1.0, "trimLevel": 1.0 } } } });
        assert_eq!(master_db(&unity), 0.0);

        // QLab's Mic cue really does default to silence at the master
        // crosspoint; importing it as 0 dB would put a live mic up at unity.
        let silent = json!({ "levels": { "entries": {
            "0": { "row": 0, "column": 0, "initialLevel": 0.0, "trimLevel": 1.0 } } } });
        assert_eq!(master_db(&silent), -60.0);

        assert_eq!(master_db(&json!({})), 0.0, "no matrix means unity");
    }

    #[test]
    fn a_qlab5_alias_gives_a_bundle_relative_path() {
        let cue = json!({ "fileTarget": { "relativePath": "audio/Intro.wav" } });
        assert_eq!(media_path(&cue).as_deref(), Some("audio/Intro.wav"));
    }

    #[test]
    fn a_qlab4_nsurl_gives_a_path_too() {
        let cue = json!({ "fileTarget": { "__class__": "NSURL", "relative": "file://video/A.mov" } });
        assert_eq!(media_path(&cue).as_deref(), Some("video/A.mov"));
    }

    #[test]
    fn an_absolute_bookmark_falls_back_to_the_media_folder() {
        let cue = json!({ "fileTarget": {
            "lastKnownPath": "/Users/x/Show.qlab5/audio/Cue 1.aif" } });
        assert_eq!(media_path(&cue).as_deref(), Some("audio/Cue 1.aif"));
    }

    #[test]
    fn a_script_cue_arrives_disarmed_with_its_source() {
        let cue = json!({ "source": "tell application \"QLab\"" });
        let out = script(&cue);
        assert_eq!(out["is_disabled"], json!(true));
        assert_eq!(out["command"], json!(""));
        assert!(out["notes"].as_str().unwrap().contains("tell application"));
    }

    #[test]
    fn an_osc_string_splits_into_address_and_typed_args() {
        let patches = Patches::default();
        let out = osc(&json!({ "oscString": "/cue/1/go 2 0.5 hello" }), &patches);
        let message = &out["messages"][0];
        assert_eq!(message["address"], json!("/cue/1/go"));
        assert_eq!(message["args"][0], json!({ "type": "int", "value": 2 }));
        assert_eq!(message["args"][1], json!({ "type": "float", "value": 0.5 }));
        assert_eq!(message["args"][2], json!({ "type": "str", "value": "hello" }));
    }

    #[test]
    fn an_empty_osc_string_yields_no_message() {
        let out = osc(&json!({ "oscString": "  " }), &Patches::default());
        assert_eq!(out["messages"], json!([]));
    }

    #[test]
    fn a_midi_voice_message_maps_to_a_note() {
        let cue = json!({ "status": 1, "channel": 1, "byte1": 60, "byte2": 64 });
        let message = &midi(&cue, &Patches::default())["messages"][0];
        assert_eq!(message["message_type"], json!("note_on"));
        assert_eq!(message["data1"], json!(60));
        assert_eq!(message["data2"], json!(64));
    }

    #[test]
    fn an_unsupported_midi_type_is_reported_not_faked() {
        let out = midi(&json!({ "status": 6 }), &Patches::default());
        assert_eq!(out["messages"], json!([]));
        assert!(out["notes"].as_str().unwrap().contains("no Inkue equivalent"));
    }

    #[test]
    fn timecode_ticks_decode_through_the_pulldown() {
        // 172_972_800_000 ticks = 3603.6 s real = 01:00:00:00 at 29.97.
        let tc = json!({
            "F53TimecodeCommonTicks": 172_972_800_000i64,
            "F53TimecodeFramerate": {
                "F53FramerateFramesPerSecond": 30,
                "F53FramerateDropFrame": false,
                "F53FramerateVideoSpeed": true,
            },
        });
        let pos = timecode_position(Some(&tc));
        assert_eq!(pos, json!({ "h": 1, "m": 0, "s": 0, "f": 0, "rate": "29.97" }));
    }

    #[test]
    fn timecode_without_pulldown_is_read_at_face_value() {
        let tc = json!({
            "F53TimecodeCommonTicks": 48_000_000i64 * 90, // 90 s
            "F53TimecodeFramerate": {
                "F53FramerateFramesPerSecond": 25,
                "F53FramerateDropFrame": false,
                "F53FramerateVideoSpeed": false,
            },
        });
        let pos = timecode_position(Some(&tc));
        assert_eq!(pos, json!({ "h": 0, "m": 1, "s": 30, "f": 0, "rate": "25" }));
    }

    #[test]
    fn an_unconvertible_cue_names_itself_and_keeps_the_detail() {
        let out = unconvertible(&json!({ "name": "House" }), "QLab Light cue", "all = home");
        assert_eq!(out["cue_type"], json!("memo"));
        assert_eq!(out["name"], json!("[QLab Light cue] House"));
        assert!(out["memo_text"].as_str().unwrap().contains("all = home"));
    }

    #[test]
    fn wait_seconds_become_milliseconds() {
        assert_eq!(wait(&json!({ "duration": 1.5 }))["wait_duration_ms"], json!(1500));
    }
}
