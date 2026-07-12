//! Blend modes for the layer compositor.
//!
//! Each mode is a per-pixel operation combining a foreground layer (a cue on a
//! higher layer) with the backdrop (the composite of everything below), QLab
//! style.  The reference implementations here are **pure Rust mirrors of the
//! GLSL** in [`GLSL_BLEND_FN`] — same structure as `warp.rs` — so the math is
//! unit-testable without a GL context; the shader is generated from the same
//! formulas (standard W3C/Photoshop separable modes).
//!
//! Compositing itself is done back-to-front with ping-pong FBOs: GL fixed-
//! function blending cannot read the backdrop, so each step samples the
//! previous composite as a texture and writes the blended result.

use serde::{Deserialize, Serialize};

/// How a visual cue's pixels combine with the layers below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Standard alpha compositing (source-over).
    #[default]
    Normal,
    /// Linear dodge: backdrop + source.
    Add,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    Difference,
    Exclusion,
    Subtract,
}

impl BlendMode {
    /// All modes, in inspector display order.
    pub const ALL: [BlendMode; 14] = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
    ];

    /// Stable integer id passed to the composite shader (`u_blend_mode`).
    pub fn shader_id(self) -> i32 {
        match self {
            BlendMode::Normal => 0,
            BlendMode::Add => 1,
            BlendMode::Multiply => 2,
            BlendMode::Screen => 3,
            BlendMode::Overlay => 4,
            BlendMode::SoftLight => 5,
            BlendMode::HardLight => 6,
            BlendMode::Darken => 7,
            BlendMode::Lighten => 8,
            BlendMode::ColorDodge => 9,
            BlendMode::ColorBurn => 10,
            BlendMode::Difference => 11,
            BlendMode::Exclusion => 12,
            BlendMode::Subtract => 13,
        }
    }
}

/// Reference blend of one colour channel (foreground `s`, backdrop `b`, both
/// in [0,1]).  Mirrors the GLSL `blend_channel` exactly — this is the
/// executable spec the shader is verified against; production renders run the
/// GLSL, so outside tests this function is intentionally unused.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn blend_channel(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Add => (b + s).min(1.0),
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        BlendMode::Overlay => hard_light(s, b), // overlay(b,s) = hard_light(s,b)
        BlendMode::SoftLight => {
            // W3C soft-light.
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::ColorDodge => {
            if b <= 0.0 { 0.0 }
            else if s >= 1.0 { 1.0 }
            else { (b / (1.0 - s)).min(1.0) }
        }
        BlendMode::ColorBurn => {
            if b >= 1.0 { 1.0 }
            else if s <= 0.0 { 0.0 }
            else { 1.0 - ((1.0 - b) / s).min(1.0) }
        }
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        BlendMode::Subtract => (b - s).max(0.0),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        // screen(b, 2s − 1)
        let s2 = 2.0 * s - 1.0;
        b + s2 - b * s2
    }
}

/// Full pixel composite, reference implementation of the GLSL `main`:
/// blend the channels, then alpha-composite (source-over) with the source
/// alpha scaled by the layer opacity.
///
/// `backdrop`/`source` are straight (non-premultiplied) RGBA.
#[cfg(test)]
pub(crate) fn composite_pixel(
    mode: BlendMode,
    backdrop: [f32; 4],
    source: [f32; 4],
    opacity: f32,
) -> [f32; 4] {
    let sa = (source[3] * opacity).clamp(0.0, 1.0);
    let ba = backdrop[3];
    let mut out = [0.0f32; 4];
    for c in 0..3 {
        // W3C: result = (1−ba)·s + ba·B(b,s), then source-over with sa.
        let blended = (1.0 - ba) * source[c] + ba * blend_channel(mode, backdrop[c], source[c]);
        out[c] = sa * blended + (1.0 - sa) * backdrop[c] * ba;
        // Un-premultiply against the output alpha below.
    }
    out[3] = sa + ba * (1.0 - sa);
    if out[3] > 0.0 {
        for c in 0..3 {
            out[c] /= out[3];
        }
    }
    out
}

/// GLSL function bodies shared by the composite shader (`#version 150 core`).
///
/// `blend_channel(mode, b, s)` must stay formula-identical to
/// [`blend_channel`] above — the Rust version is the executable spec.
pub(crate) const GLSL_BLEND_FN: &str = r#"
float hard_light(float b, float s) {
    if (s <= 0.5) return b * 2.0 * s;
    float s2 = 2.0 * s - 1.0;
    return b + s2 - b * s2;
}
float blend_channel(int mode, float b, float s) {
    if (mode == 0) return s;
    if (mode == 1) return min(b + s, 1.0);
    if (mode == 2) return b * s;
    if (mode == 3) return b + s - b * s;
    if (mode == 4) return hard_light(s, b);
    if (mode == 5) {
        if (s <= 0.5) return b - (1.0 - 2.0 * s) * b * (1.0 - b);
        float d = (b <= 0.25) ? ((16.0 * b - 12.0) * b + 4.0) * b : sqrt(b);
        return b + (2.0 * s - 1.0) * (d - b);
    }
    if (mode == 6) return hard_light(b, s);
    if (mode == 7) return min(b, s);
    if (mode == 8) return max(b, s);
    if (mode == 9) {
        if (b <= 0.0) return 0.0;
        if (s >= 1.0) return 1.0;
        return min(b / (1.0 - s), 1.0);
    }
    if (mode == 10) {
        if (b >= 1.0) return 1.0;
        if (s <= 0.0) return 0.0;
        return 1.0 - min((1.0 - b) / s, 1.0);
    }
    if (mode == 11) return abs(b - s);
    if (mode == 12) return b + s - 2.0 * b * s;
    return max(b - s, 0.0);
}
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn serde_snake_case_roundtrip() {
        for mode in BlendMode::ALL {
            let json = serde_json::to_value(mode).unwrap();
            let back: BlendMode = serde_json::from_value(json).unwrap();
            assert_eq!(mode, back);
        }
        assert_eq!(serde_json::to_value(BlendMode::ColorDodge).unwrap(), "color_dodge");
        assert_eq!(BlendMode::default(), BlendMode::Normal);
    }

    #[test]
    fn shader_ids_are_unique_and_dense() {
        let mut ids: Vec<i32> = BlendMode::ALL.iter().map(|m| m.shader_id()).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..14).collect::<Vec<_>>());
    }

    #[test]
    fn normal_returns_source() {
        assert!(close(blend_channel(BlendMode::Normal, 0.3, 0.8), 0.8));
    }

    #[test]
    fn multiply_and_screen_are_duals() {
        let (b, s) = (0.4, 0.7);
        assert!(close(blend_channel(BlendMode::Multiply, b, s), 0.28));
        assert!(close(blend_channel(BlendMode::Screen, b, s), 0.82));
        // screen(b,s) = 1 − (1−b)(1−s)
        assert!(close(
            blend_channel(BlendMode::Screen, b, s),
            1.0 - (1.0 - b) * (1.0 - s),
        ));
    }

    #[test]
    fn overlay_is_hard_light_swapped() {
        for &(b, s) in &[(0.2, 0.7), (0.8, 0.3), (0.5, 0.5)] {
            assert!(close(
                blend_channel(BlendMode::Overlay, b, s),
                blend_channel(BlendMode::HardLight, s, b),
            ));
        }
    }

    #[test]
    fn darken_lighten_difference() {
        assert!(close(blend_channel(BlendMode::Darken, 0.4, 0.7), 0.4));
        assert!(close(blend_channel(BlendMode::Lighten, 0.4, 0.7), 0.7));
        assert!(close(blend_channel(BlendMode::Difference, 0.4, 0.7), 0.3));
        assert!(close(blend_channel(BlendMode::Subtract, 0.4, 0.7), 0.0));
        assert!(close(blend_channel(BlendMode::Subtract, 0.7, 0.4), 0.3));
    }

    #[test]
    fn dodge_burn_edge_cases_never_divide_by_zero() {
        assert!(close(blend_channel(BlendMode::ColorDodge, 0.5, 1.0), 1.0));
        assert!(close(blend_channel(BlendMode::ColorDodge, 0.0, 0.5), 0.0));
        assert!(close(blend_channel(BlendMode::ColorBurn, 0.5, 0.0), 0.0));
        assert!(close(blend_channel(BlendMode::ColorBurn, 1.0, 0.5), 1.0));
        for mode in BlendMode::ALL {
            for &(b, s) in &[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)] {
                let v = blend_channel(mode, b, s);
                assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{mode:?}({b},{s}) = {v}");
            }
        }
    }

    #[test]
    fn soft_light_matches_w3c_reference_points() {
        // s = 0.5 leaves the backdrop unchanged.
        for &b in &[0.0, 0.25, 0.5, 0.9] {
            assert!(close(blend_channel(BlendMode::SoftLight, b, 0.5), b));
        }
    }

    #[test]
    fn composite_full_opacity_normal_replaces_backdrop() {
        let out = composite_pixel(BlendMode::Normal, [0.2, 0.2, 0.2, 1.0], [0.8, 0.4, 0.1, 1.0], 1.0);
        assert!(close(out[0], 0.8) && close(out[1], 0.4) && close(out[2], 0.1) && close(out[3], 1.0));
    }

    #[test]
    fn composite_zero_opacity_keeps_backdrop() {
        let backdrop = [0.2, 0.5, 0.7, 1.0];
        let out = composite_pixel(BlendMode::Multiply, backdrop, [0.9, 0.9, 0.9, 1.0], 0.0);
        for c in 0..4 {
            assert!(close(out[c], backdrop[c]));
        }
    }

    #[test]
    fn composite_transparent_source_keeps_backdrop() {
        let backdrop = [0.3, 0.3, 0.3, 1.0];
        let out = composite_pixel(BlendMode::Add, backdrop, [1.0, 1.0, 1.0, 0.0], 1.0);
        for c in 0..4 {
            assert!(close(out[c], backdrop[c]));
        }
    }

    #[test]
    fn composite_over_transparent_backdrop_shows_source() {
        // First layer over an empty stage: blend must not darken it.
        let out = composite_pixel(BlendMode::Multiply, [0.0, 0.0, 0.0, 0.0], [0.6, 0.7, 0.8, 1.0], 1.0);
        assert!(close(out[0], 0.6) && close(out[1], 0.7) && close(out[2], 0.8) && close(out[3], 1.0));
    }

    #[test]
    fn glsl_source_contains_every_mode_id() {
        for mode in BlendMode::ALL {
            if mode != BlendMode::Subtract {
                assert!(
                    GLSL_BLEND_FN.contains(&format!("mode == {}", mode.shader_id())),
                    "GLSL missing mode {mode:?}"
                );
            }
        }
    }
}
