//! [`TextCue`] — displays formatted text on the output surface via mpv's subtitle layer.
//!
//! Uses mpv's `sub-text` property with ASS inline tags to render styled text on top
//! of whatever is currently on screen (black idle surface, or over existing video/image).
//! The cue timer uses `osd-msg1`; `sub-text` is a separate rendering layer so they coexist.
//!
//! Styling: font family, size, hex colour, and a 9-point position grid.
//! Multi-line text is supported (newlines converted to ASS `\N`).
//! `display_duration_ms` enables auto-complete after a fixed duration.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

// ---------------------------------------------------------------------------
// TextPosition
// ---------------------------------------------------------------------------

/// 9-point position grid for the text overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextPosition {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    #[default]
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl TextPosition {
    /// ASS `\an` alignment code (1–9).
    fn to_ass_alignment(self) -> u8 {
        match self {
            TextPosition::BottomLeft   => 1,
            TextPosition::BottomCenter => 2,
            TextPosition::BottomRight  => 3,
            TextPosition::MiddleLeft   => 4,
            TextPosition::Center       => 5,
            TextPosition::MiddleRight  => 6,
            TextPosition::TopLeft      => 7,
            TextPosition::TopCenter    => 8,
            TextPosition::TopRight     => 9,
        }
    }
}

// ---------------------------------------------------------------------------
// ASS helpers
// ---------------------------------------------------------------------------

/// Parse `"#RRGGBB"` into `(r, g, b)`.
fn parse_hex_color(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.trim_start_matches('#');
    if hex.len() != 6 { return None; }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Build a fully-styled ASS override string for mpv's `sub-text` property.
///
/// Color format in ASS is `&HAABBGGRR&` — note reversed RGB and leading alpha byte
/// where `00` = fully opaque.
fn build_ass_text(text: &str, font: &str, size: u32, color: &str, position: TextPosition) -> String {
    let an = position.to_ass_alignment();
    let (r, g, b) = parse_hex_color(color).unwrap_or((255, 255, 255));
    // ASS primary colour: &H00BBGGRR& (opaque)
    let ass_color = format!("&H00{b:02X}{g:02X}{r:02X}&");
    // Black border + shadow for readability on any background.
    let tags = format!(
        "{{\\an{an}\\fn{font}\\fs{size}\\c{ass_color}\\bord2\\shad1\\3c&H00000000&\\4c&H00000000&}}"
    );
    // ASS line breaks are \N, not \n.
    let body = text.replace('\n', "\\N");
    format!("{tags}{body}")
}

// ---------------------------------------------------------------------------
// TextCue
// ---------------------------------------------------------------------------

/// A cue that displays styled text on the output surface.
pub struct TextCue {
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,

    state: CueState,

    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    action_started_at: Option<Instant>,

    continue_mode: ContinueMode,

    /// The text to display.  Newlines are supported.
    pub text: String,
    /// Font family name passed to mpv (ASS `\fn`).
    pub font: String,
    /// Font size in mpv OSD / ASS points.
    pub font_size: u32,
    /// Text colour as `"#RRGGBB"`.
    pub text_color: String,
    /// Position on the output surface (9-point grid).
    pub position: TextPosition,
    /// Target monitor index.  `None` = use the workspace display setting.
    pub screen_index: Option<u32>,
    /// Auto-complete after this duration.  `None` = hold until stopped.
    pub display_duration_ms: Option<u64>,

    is_disabled: bool,

    in_pre_wait: bool,
    play_generation: u64,
    auto_continue_fired: bool,
}

impl TextCue {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Text Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Cyan,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            continue_mode: ContinueMode::DoNotContinue,
            text: String::new(),
            font: String::from("Arial"),
            font_size: 60,
            text_color: String::from("#FFFFFF"),
            position: TextPosition::Center,
            screen_index: None,
            display_duration_ms: None,
            is_disabled: false,
            in_pre_wait: false,
            play_generation: 0,
            auto_continue_fired: false,
        }
    }

    fn start_text_action(&mut self, context: &CueContext) -> Result<()> {
        if self.text.trim().is_empty() {
            // No text — complete instantly so the sequence can advance.
            context.emit(CueEvent::ActionStarted { cue_id: self.id });
            self.state = CueState::Completed;
            context.emit(CueEvent::ActionCompleted { cue_id: self.id });
            return Ok(());
        }

        let ass_text = build_ass_text(
            &self.text,
            &self.font,
            self.font_size,
            &self.text_color,
            self.position,
        );
        let screen = self.screen_index.or(context.output_screen);
        context.output_engine.show_text_overlay(&ass_text, screen);

        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;
        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }
}

impl Default for TextCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for TextCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Text }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn is_disabled(&self) -> bool { self.is_disabled }
    fn set_disabled(&mut self, d: bool) { self.is_disabled = d; }
    fn state(&self) -> CueState { self.state }

    fn load(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            return Ok(());
        }
        self.play_generation = self.play_generation.wrapping_add(1);
        self.auto_continue_fired = false;
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());

        if !self.pre_wait.is_zero() {
            self.in_pre_wait = true;
            return Ok(());
        }

        self.start_text_action(context)
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;
        context.output_engine.clear_text_overlay();
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            self.state = CueState::Paused;
        }
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Paused {
            self.state = CueState::Running;
        }
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            if let Err(e) = self.start_text_action(context) {
                log::warn!("TextCue '{}' failed to start action: {e}", self.name);
                self.state = CueState::Standby;
            }
        }
        Ok(())
    }

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> {
        self.display_duration_ms.map(Duration::from_millis)
    }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        self.action_started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    /// Text overlays stop on the next GO (same as Image/Video visual cues).
    fn stop_on_next_go(&self) -> bool { true }

    fn play_generation(&self) -> u64 { self.play_generation }
    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state,
            voice_id: None,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: RuntimeState) {
        self.state = snap.state;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        self.in_pre_wait = snap.state == CueState::Running && snap.action_started_at.is_none();
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "text",
            "cue_type": "text",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "text": self.text,
            "font": self.font,
            "font_size": self.font_size,
            "text_color": self.text_color,
            "position": self.position,
            "screen_index": self.screen_index,
            "display_duration_ms": self.display_duration_ms,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`TextCue`].
pub struct TextCueFactory;

impl CueFactory for TextCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(TextCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = TextCue::new();

        if let Some(s) = value.get("id").and_then(|v| v.as_str()) {
            cue.id = s.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(s) = value.get("name").and_then(|v| v.as_str()) {
            cue.name = s.to_string();
        }
        if let Some(s) = value.get("number").and_then(|v| v.as_str()) {
            cue.number = Some(s.to_string());
        }
        if let Some(s) = value.get("notes").and_then(|v| v.as_str()) {
            cue.notes = s.to_string();
        }
        if let Some(col) = value.get("color") {
            if let Ok(c) = serde_json::from_value(col.clone()) {
                cue.color = c;
            }
        }
        if let Some(ms) = value.get("pre_wait_ms").and_then(|v| v.as_u64()) {
            cue.pre_wait = Duration::from_millis(ms);
        }
        if let Some(ms) = value.get("post_wait_ms").and_then(|v| v.as_u64()) {
            cue.post_wait = Duration::from_millis(ms);
        }
        if let Some(cm) = value.get("continue_mode") {
            if let Ok(mode) = serde_json::from_value(cm.clone()) {
                cue.continue_mode = mode;
            }
        }
        if let Some(s) = value.get("text").and_then(|v| v.as_str()) {
            cue.text = s.to_string();
        }
        if let Some(s) = value.get("font").and_then(|v| v.as_str()) {
            cue.font = s.to_string();
        }
        if let Some(n) = value.get("font_size").and_then(|v| v.as_u64()) {
            cue.font_size = n as u32;
        }
        if let Some(s) = value.get("text_color").and_then(|v| v.as_str()) {
            cue.text_color = s.to_string();
        }
        if let Some(pos) = value.get("position") {
            if let Ok(p) = serde_json::from_value(pos.clone()) {
                cue.position = p;
            }
        }
        if let Some(idx) = value.get("screen_index").and_then(|v| v.as_u64()) {
            cue.screen_index = Some(idx as u32);
        }
        if let Some(ms) = value.get("display_duration_ms").and_then(|v| v.as_u64()) {
            cue.display_duration_ms = Some(ms);
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }

        Ok(Box::new(cue))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_color_is_cyan() {
        assert_eq!(TextCue::new().color(), CueColor::Cyan);
    }

    #[test]
    fn cue_type_is_text() {
        assert_eq!(TextCue::new().cue_type(), CueType::Text);
    }

    #[test]
    fn stop_on_next_go_true() {
        assert!(TextCue::new().stop_on_next_go());
    }

    #[test]
    fn duration_none_by_default() {
        assert!(TextCue::new().duration().is_none());
    }

    #[test]
    fn duration_from_display_duration_ms() {
        let mut cue = TextCue::new();
        cue.display_duration_ms = Some(5000);
        assert_eq!(cue.duration(), Some(Duration::from_millis(5000)));
    }

    #[test]
    fn position_ass_alignment() {
        assert_eq!(TextPosition::Center.to_ass_alignment(), 5);
        assert_eq!(TextPosition::TopLeft.to_ass_alignment(), 7);
        assert_eq!(TextPosition::BottomRight.to_ass_alignment(), 3);
        assert_eq!(TextPosition::BottomCenter.to_ass_alignment(), 2);
        assert_eq!(TextPosition::TopRight.to_ass_alignment(), 9);
    }

    #[test]
    fn parse_hex_color_values() {
        assert_eq!(parse_hex_color("#FFFFFF"), Some((255, 255, 255)));
        assert_eq!(parse_hex_color("#FF0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("#0000FF"), Some((0, 0, 255)));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn build_ass_text_center_white() {
        let ass = build_ass_text("Hello", "Arial", 60, "#FFFFFF", TextPosition::Center);
        assert!(ass.contains("\\an5"));
        assert!(ass.contains("\\fnArial"));
        assert!(ass.contains("\\fs60"));
        assert!(ass.contains("Hello"));
        // White in ASS BGR: r=FF g=FF b=FF → &H00FFFFFF&
        assert!(ass.contains("&H00FFFFFF&"));
    }

    #[test]
    fn build_ass_text_red_color() {
        // Red = #FF0000 → r=FF g=00 b=00 → ASS &H000000FF&
        let ass = build_ass_text("Red", "Arial", 40, "#FF0000", TextPosition::TopLeft);
        assert!(ass.contains("\\an7"));
        assert!(ass.contains("&H000000FF&"));
    }

    #[test]
    fn build_ass_text_multiline() {
        let ass = build_ass_text("Line 1\nLine 2", "Arial", 60, "#FFFFFF", TextPosition::Center);
        assert!(ass.contains("Line 1\\NLine 2"));
    }

    #[test]
    fn serialize_roundtrip() {
        let factory = TextCueFactory;
        let mut cue = TextCue::new();
        cue.set_name("Test Text".to_string());
        cue.text = "Hello, World!".to_string();
        cue.font_size = 80;
        cue.text_color = "#FF0000".to_string();
        cue.position = TextPosition::TopCenter;
        cue.display_duration_ms = Some(3000);

        let json_val = cue.serialize();
        assert_eq!(json_val["type"], "text");
        assert_eq!(json_val["name"], "Test Text");
        assert_eq!(json_val["text"], "Hello, World!");
        assert_eq!(json_val["font_size"], 80);
        assert_eq!(json_val["display_duration_ms"], 3000);

        let rebuilt = factory.from_json(json_val).expect("should deserialise");
        assert_eq!(rebuilt.name(), "Test Text");
        assert_eq!(rebuilt.cue_type(), CueType::Text);
        assert_eq!(rebuilt.duration(), Some(Duration::from_millis(3000)));
    }
}
