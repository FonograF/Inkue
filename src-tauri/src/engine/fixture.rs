//! Fixture model for the lighting patch.
//!
//! A [`PatchedFixture`] is a named lighting instrument placed at a DMX address
//! in a universe.  It embeds its [`FixtureType`] (the channel layout) so the
//! workspace is fully self-contained — no cross-references to resolve and no
//! external library to ship.  [`builtin_fixture_types`] provides ready-made
//! templates the operator picks from when patching; once patched, each
//! fixture's parameters can be tweaked independently.
//!
//! A Light Cue references a fixture by ID and a parameter by index; at GO it
//! resolves `(universe, channel, width)` from the patch and submits a fade to
//! the [`super::dmx_engine::DmxEngine`].

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::dmx_engine::ChannelWidth;

/// Number of DMX slots a [`ChannelWidth`] occupies.
pub fn width_channels(width: ChannelWidth) -> u16 {
    match width {
        ChannelWidth::Bit8 => 1,
        ChannelWidth::Bit16 => 2,
    }
}

/// What a fixture parameter controls.  Drives the patch UI grouping, the
/// identify ("test fixture") value, and future colour-mixing helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// Master dimmer / intensity.
    Intensity,
    Red,
    Green,
    Blue,
    White,
    Amber,
    /// Ultraviolet.
    Uv,
    /// Horizontal position of a moving head.
    Pan,
    /// Vertical position of a moving head.
    Tilt,
    /// Anything else (gobo, strobe, focus, …) — set by raw value.
    Generic,
}

impl ParamKind {
    /// The normalised value to drive this parameter to when identifying the
    /// fixture (the "test fixture" button): colour + intensity go full, moving
    /// heads centre so the beam is visible, everything else stays dark.
    pub fn identify_value(self) -> f64 {
        match self {
            ParamKind::Intensity
            | ParamKind::Red
            | ParamKind::Green
            | ParamKind::Blue
            | ParamKind::White
            | ParamKind::Amber
            | ParamKind::Uv => 1.0,
            ParamKind::Pan | ParamKind::Tilt => 0.5,
            ParamKind::Generic => 0.0,
        }
    }
}

/// One controllable parameter of a fixture, at a fixed offset from the
/// fixture's base address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureParam {
    /// What this parameter controls.
    pub kind: ParamKind,
    /// Human-readable label shown in the inspector (e.g. `"Dimmer"`, `"Red"`).
    pub name: String,
    /// Zero-based offset of this parameter's (coarse) channel from the
    /// fixture's base address.
    pub channel_offset: u16,
    /// Whether this parameter is 8- or 16-bit on the wire.
    pub width: ChannelWidth,
    /// Default normalised value `[0, 1]`.
    pub default: f64,
}

/// The channel layout of a kind of lighting instrument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureType {
    /// Display name (e.g. `"RGB PAR"`, `"Moving Head 16-bit"`).
    pub name: String,
    /// The parameters, in display order.
    pub parameters: Vec<FixtureParam>,
}

impl FixtureType {
    /// Number of DMX channels this fixture occupies, i.e. the highest
    /// `channel_offset + width` across its parameters.
    pub fn footprint(&self) -> u16 {
        self.parameters
            .iter()
            .map(|p| p.channel_offset + width_channels(p.width))
            .max()
            .unwrap_or(0)
    }
}

/// A fixture placed at a DMX address in the workspace patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchedFixture {
    /// Unique identifier, referenced by Light Cue targets.
    pub id: Uuid,
    /// Operator-facing label (e.g. `"Front wash L"`).
    pub label: String,
    /// Logical universe this fixture lives in.
    pub universe: u16,
    /// One-based DMX start address of the fixture's first channel.
    pub base_address: u16,
    /// The embedded channel layout.
    pub fixture_type: FixtureType,
}

impl PatchedFixture {
    /// Create a fixture from a type at the given address with a fresh UUID.
    pub fn new(label: impl Into<String>, universe: u16, base_address: u16, fixture_type: FixtureType) -> Self {
        Self {
            id: Uuid::new_v4(),
            label: label.into(),
            universe,
            base_address,
            fixture_type,
        }
    }

    /// Inclusive one-based address range `(first, last)` this fixture occupies.
    /// Returns `None` for a fixture with no parameters.
    pub fn address_span(&self) -> Option<(u16, u16)> {
        let footprint = self.fixture_type.footprint();
        if footprint == 0 {
            return None;
        }
        Some((self.base_address, self.base_address + footprint - 1))
    }

    /// Resolve a parameter index to its `(universe, zero-based channel, width)`
    /// for the DMX engine.  Returns `None` if the index is out of range.
    pub fn resolve_channel(&self, param_index: usize) -> Option<(u16, u16, ChannelWidth)> {
        let param = self.fixture_type.parameters.get(param_index)?;
        // base_address is 1-based; the engine addresses channels 0-based.
        let channel = self.base_address.saturating_sub(1) + param.channel_offset;
        Some((self.universe, channel, param.width))
    }
}

/// A named set of fixtures driven together by a Light Cue: one colour /
/// intensity control fans out to every member.  Members are referenced by ID so
/// a group survives a fixture being re-patched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureGroup {
    /// Unique identifier, referenced by Light Cue group targets.
    pub id: Uuid,
    /// Operator-facing label (e.g. `"Front wash"`).
    pub label: String,
    /// Member fixture IDs.
    pub fixture_ids: Vec<Uuid>,
}

impl FixtureGroup {
    /// Create a group with a fresh UUID.
    pub fn new(label: impl Into<String>, fixture_ids: Vec<Uuid>) -> Self {
        Self { id: Uuid::new_v4(), label: label.into(), fixture_ids }
    }
}

/// A detected address clash between two patched fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureConflict {
    pub fixture_a: Uuid,
    pub fixture_b: Uuid,
    pub universe: u16,
    /// Human-readable description for the patch UI.
    pub message: String,
}

/// Whether two fixtures share any DMX channel in the same universe.
fn overlaps(a: &PatchedFixture, b: &PatchedFixture) -> bool {
    if a.universe != b.universe {
        return false;
    }
    let (Some((a0, a1)), Some((b0, b1))) = (a.address_span(), b.address_span()) else {
        return false;
    };
    a0 <= b1 && b0 <= a1
}

/// Find every pair of fixtures whose channel ranges overlap.
pub fn find_conflicts(fixtures: &[PatchedFixture]) -> Vec<FixtureConflict> {
    let mut conflicts = Vec::new();
    for i in 0..fixtures.len() {
        for j in (i + 1)..fixtures.len() {
            let a = &fixtures[i];
            let b = &fixtures[j];
            if overlaps(a, b) {
                conflicts.push(FixtureConflict {
                    fixture_a: a.id,
                    fixture_b: b.id,
                    universe: a.universe,
                    message: format!(
                        "“{}” and “{}” overlap on universe {}",
                        a.label, b.label, a.universe
                    ),
                });
            }
        }
    }
    conflicts
}

// ---------------------------------------------------------------------------
// Built-in templates
// ---------------------------------------------------------------------------

fn param(kind: ParamKind, name: &str, offset: u16, width: ChannelWidth, default: f64) -> FixtureParam {
    FixtureParam { kind, name: name.to_string(), channel_offset: offset, width, default }
}

/// The built-in fixture templates offered when patching.  These are pure
/// data — picking one copies its layout into a [`PatchedFixture`].
pub fn builtin_fixture_types() -> Vec<FixtureType> {
    use ChannelWidth::{Bit16, Bit8};
    use ParamKind::*;

    vec![
        FixtureType {
            name: "Dimmer (1ch)".to_string(),
            parameters: vec![param(Intensity, "Dimmer", 0, Bit8, 0.0)],
        },
        FixtureType {
            name: "RGB (3ch)".to_string(),
            parameters: vec![
                param(Red, "Red", 0, Bit8, 0.0),
                param(Green, "Green", 1, Bit8, 0.0),
                param(Blue, "Blue", 2, Bit8, 0.0),
            ],
        },
        FixtureType {
            name: "RGBW (4ch)".to_string(),
            parameters: vec![
                param(Red, "Red", 0, Bit8, 0.0),
                param(Green, "Green", 1, Bit8, 0.0),
                param(Blue, "Blue", 2, Bit8, 0.0),
                param(White, "White", 3, Bit8, 0.0),
            ],
        },
        FixtureType {
            name: "RGBA (4ch)".to_string(),
            parameters: vec![
                param(Red, "Red", 0, Bit8, 0.0),
                param(Green, "Green", 1, Bit8, 0.0),
                param(Blue, "Blue", 2, Bit8, 0.0),
                param(Amber, "Amber", 3, Bit8, 0.0),
            ],
        },
        FixtureType {
            name: "PAR Dimmer+RGB (4ch)".to_string(),
            parameters: vec![
                param(Intensity, "Dimmer", 0, Bit8, 0.0),
                param(Red, "Red", 1, Bit8, 0.0),
                param(Green, "Green", 2, Bit8, 0.0),
                param(Blue, "Blue", 3, Bit8, 0.0),
            ],
        },
        FixtureType {
            name: "Moving Head 16-bit (8ch)".to_string(),
            parameters: vec![
                param(Pan, "Pan", 0, Bit16, 0.5),
                param(Tilt, "Tilt", 2, Bit16, 0.5),
                param(Intensity, "Dimmer", 4, Bit8, 0.0),
                param(Red, "Red", 5, Bit8, 0.0),
                param(Green, "Green", 6, Bit8, 0.0),
                param(Blue, "Blue", 7, Bit8, 0.0),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb() -> FixtureType {
        builtin_fixture_types().into_iter().find(|t| t.name.starts_with("RGB ")).unwrap()
    }

    #[test]
    fn footprint_counts_16bit_as_two() {
        let mh = builtin_fixture_types()
            .into_iter()
            .find(|t| t.name.starts_with("Moving Head"))
            .unwrap();
        // Pan(2) + Tilt(2) + Dimmer + R + G + B = 8 channels.
        assert_eq!(mh.footprint(), 8);
    }

    #[test]
    fn resolve_channel_is_zero_based() {
        let f = PatchedFixture::new("Wash", 1, 10, rgb());
        // base 10 (1-based) → channel 9 (0-based); Green offset 1 → 10.
        assert_eq!(f.resolve_channel(0), Some((1, 9, ChannelWidth::Bit8)));
        assert_eq!(f.resolve_channel(1), Some((1, 10, ChannelWidth::Bit8)));
        assert_eq!(f.resolve_channel(2), Some((1, 11, ChannelWidth::Bit8)));
        assert_eq!(f.resolve_channel(3), None);
    }

    #[test]
    fn address_span_is_inclusive() {
        let f = PatchedFixture::new("Wash", 1, 10, rgb());
        assert_eq!(f.address_span(), Some((10, 12)));
    }

    #[test]
    fn conflicts_detect_overlap_same_universe_only() {
        let a = PatchedFixture::new("A", 1, 1, rgb()); // 1..3
        let b = PatchedFixture::new("B", 1, 3, rgb()); // 3..5 → overlaps A at 3
        let c = PatchedFixture::new("C", 1, 4, rgb()); // 4..6 → overlaps B, not A
        let d = PatchedFixture::new("D", 2, 1, rgb()); // other universe → no conflict

        let fixtures = vec![a, b, c, d];
        let conflicts = find_conflicts(&fixtures);
        // A–B and B–C overlap; A–C do not (1..3 vs 4..6); universe 2 never conflicts.
        assert_eq!(conflicts.len(), 2);
        assert!(conflicts.iter().all(|c| c.universe == 1));
    }

    #[test]
    fn adjacent_fixtures_do_not_conflict() {
        let a = PatchedFixture::new("A", 1, 1, rgb()); // 1..3
        let b = PatchedFixture::new("B", 1, 4, rgb()); // 4..6
        assert!(find_conflicts(&[a, b]).is_empty());
    }
}
