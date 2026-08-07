//! Fade curve shapes — the authoring model behind QLab's Curve tab.
//!
//! A fade needs **two** shapes, not one: an envelope that sounds natural going
//! up is not the one that sounds natural coming down. QLab stores an `upShape`
//! and a `downShape` with a lock that mirrors them; [`FadeShapes`] is the same
//! idea. Inkue previously derived the falling curve by inverting the rising
//! one, which is exactly the locked case.
//!
//! Everything here is normalised **progress**: `sample(t)` answers "how far
//! from the start value towards the end value are we at time `t`", `0 → 1`,
//! whichever direction the underlying value is actually moving. That keeps one
//! meaning across audio gain, DMX levels and video opacity. (QLab writes its
//! `downShape` as a falling `v`; the importer flips it to progress.)
//!
//! The shapes are evaluated off the real-time thread and [`CurveShape::bake`]d
//! into an [`CurveTable`] for the audio callback.

use serde::{Deserialize, Serialize};

use crate::engine::ring_command::CurveTable;

/// One control point of a custom curve. Both axes are normalised `0..=1`:
/// `t` is elapsed fade time, `v` is progress towards the target.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub t: f64,
    pub v: f64,
}

impl CurvePoint {
    pub fn new(t: f64, v: f64) -> Self {
        Self { t: t.clamp(0.0, 1.0), v: v.clamp(0.0, 1.0) }
    }
}

/// The kinds of curve an operator can choose, matching QLab's Curve tab menu
/// (minus 2D Path, which QLab itself does not offer for audio).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    /// Straight line, and with control points straight *segments* through
    /// them. This is the hand-editable kind: a point sets where the curve
    /// goes, Alt-dragging a segment sets how it gets there.
    ///
    /// Accepts `"custom"` on the way in: an earlier build smoothed a spline
    /// through the points automatically, which Alt-bending replaced — explicit
    /// beats guessing, and the two modes were doing the same job.
    #[serde(alias = "custom")]
    Linear,
    /// Ease-in, ease-out. The default, and what most fades should use.
    #[default]
    SCurve,
    /// Inkue's original exponential shape, kept so existing shows load
    /// unchanged.
    Exponential,
    /// Mathematically shaped by [`CurveShape::intensity`]: 0 is linear,
    /// positive starts slow and accelerates, negative the reverse.
    Parametric,
}

impl CurveKind {
    /// Whether control points are meaningful for this kind.
    pub fn uses_points(self) -> bool {
        matches!(self, CurveKind::Linear)
    }
}

/// One fade envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveShape {
    pub kind: CurveKind,
    /// Shaping parameter for [`CurveKind::Parametric`], roughly −10..10.
    #[serde(default)]
    pub intensity: f64,
    /// Control points for the point-based kinds, ordered by `t`. The
    /// endpoints (0,0) and (1,1) are implicit and never stored, so a curve
    /// can never fail to start at the start or reach the target.
    #[serde(default)]
    pub points: Vec<CurvePoint>,
    /// Per-segment bow, `-1..=1`, one entry per gap between resolved points
    /// (so `points.len() + 1` of them). `0` leaves the segment as the kind
    /// draws it; positive bows it up, negative down.
    ///
    /// Kept beside the points rather than on them because a segment belongs to
    /// *two* points, and the outer segments touch the implicit endpoints —
    /// which have nowhere to store anything. Evaluation tolerates a list of
    /// the wrong length, so a hand-edited file cannot break a fade.
    #[serde(default)]
    pub bends: Vec<f64>,
}

impl Default for CurveShape {
    fn default() -> Self {
        Self { kind: CurveKind::SCurve, intensity: 0.0, points: Vec::new(), bends: Vec::new() }
    }
}

impl CurveShape {
    /// A shape with no control points, of the given kind.
    pub fn of_kind(kind: CurveKind) -> Self {
        Self { kind, ..Self::default() }
    }

    /// Progress towards the target at normalised time `t`.
    pub fn sample(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let value = match self.kind {
            CurveKind::Linear => self.through_points(t),
            CurveKind::SCurve => t * t * (3.0 - 2.0 * t),
            CurveKind::Exponential => {
                const K: f64 = 5.0;
                (K * t).exp_m1() / K.exp_m1()
            }
            CurveKind::Parametric => parametric(t, self.intensity),
        };
        value.clamp(0.0, 1.0)
    }

    /// The full point list including the implicit endpoints, ordered and
    /// de-duplicated on `t` so interpolation never divides by zero.
    fn resolved_points(&self) -> Vec<CurvePoint> {
        let mut points = Vec::with_capacity(self.points.len() + 2);
        points.push(CurvePoint::new(0.0, 0.0));
        let mut interior: Vec<CurvePoint> = self
            .points
            .iter()
            .copied()
            .filter(|p| p.t > 0.0 && p.t < 1.0)
            .collect();
        interior.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        for point in interior {
            if point.t > points.last().map(|p: &CurvePoint| p.t).unwrap_or(0.0) {
                points.push(point);
            }
        }
        points.push(CurvePoint::new(1.0, 1.0));
        points
    }

    /// Straight segments through the control points, each optionally bowed.
    fn through_points(&self, t: f64) -> f64 {
        let points = self.resolved_points();
        let index = match points.iter().position(|p| p.t >= t) {
            Some(0) => return points[0].v,
            Some(i) => i,
            None => return points[points.len() - 1].v,
        };
        let (left, right) = (points[index - 1], points[index]);
        let span = right.t - left.t;
        if span <= 0.0 {
            return right.v;
        }
        // Bow the segment by warping its local parameter. Doing it here rather
        // than on the values keeps the segment inside its endpoints whatever
        // the bend, for straight and splined segments alike.
        let local = bow((t - left.t) / span, self.bend_for(index - 1));
        left.v + (right.v - left.v) * local
    }

    /// The bow of segment `index`, or none when the list is short.
    fn bend_for(&self, index: usize) -> f64 {
        self.bends.get(index).copied().unwrap_or(0.0).clamp(-1.0, 1.0)
    }

    /// Whether any segment is bowed away from what its kind would draw.
    pub fn is_bowed(&self) -> bool {
        self.bends.iter().any(|b| b.abs() > 1e-9)
    }

    /// Sample this shape into a table the audio callback can read.
    pub fn bake(&self) -> CurveTable {
        CurveTable::from_fn(|t| self.sample(t))
    }

    /// The engine curve for this shape: the analytic kinds stay exact, and
    /// only a point-based curve pays for a baked table.
    pub fn to_engine(&self) -> crate::engine::ring_command::FadeCurve {
        use crate::engine::ring_command::FadeCurve;
        match self.kind {
            // A straight line only stays exact while nothing bends it.
            CurveKind::Linear if self.points.is_empty() && !self.is_bowed() => FadeCurve::Linear,
            CurveKind::SCurve => FadeCurve::SCurve,
            CurveKind::Exponential => FadeCurve::Exponential,
            _ => FadeCurve::Table(self.bake()),
        }
    }
}

/// Warp a segment's local parameter to bow it. `bend` 0 is untouched;
/// positive lifts the segment above its chord, negative sags it below.
///
/// Reuses the parametric shaping, so a bowed segment is still monotone and
/// still lands exactly on both of its endpoints.
pub fn bow(local: f64, bend: f64) -> f64 {
    if bend.abs() < 1e-9 {
        return local;
    }
    parametric(local, -bend.clamp(-1.0, 1.0) * BOW_STRENGTH)
}

/// How hard a full-scale bend pushes. 4 reaches a pronounced bow while the
/// segment stays readable at the extremes.
const BOW_STRENGTH: f64 = 4.0;

/// The bend that makes a segment pass through `target` at `local`.
///
/// The inverse of [`bow`], so dragging with Alt puts the curve under the
/// cursor instead of nudging it by an arbitrary amount.
pub fn bend_through(local: f64, target: f64) -> f64 {
    const EPSILON: f64 = 1e-4;
    let u = local.clamp(EPSILON, 1.0 - EPSILON);
    let v = target.clamp(EPSILON, 1.0 - EPSILON);
    let k = if v > u {
        // Above the chord: 1 - (1-u)^(1-k)
        1.0 - (1.0 - v).ln() / (1.0 - u).ln()
    } else {
        // Below the chord: u^(1+k)
        v.ln() / u.ln() - 1.0
    };
    (-k / BOW_STRENGTH).clamp(-1.0, 1.0)
}

/// A power curve about the midpoint. `intensity` 0 is linear; positive eases
/// in (slow start), negative eases out.
fn parametric(t: f64, intensity: f64) -> f64 {
    let k = intensity.clamp(-10.0, 10.0);
    if k.abs() < 1e-9 {
        return t;
    }
    if k > 0.0 {
        t.powf(1.0 + k)
    } else {
        1.0 - (1.0 - t).powf(1.0 - k)
    }
}

// ---------------------------------------------------------------------------
// FadeShapes
// ---------------------------------------------------------------------------

/// The rising and falling envelopes of one fade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FadeShapes {
    /// Used when the value is increasing.
    pub up: CurveShape,
    /// Used when the value is decreasing. Ignored while [`Self::mirrored`].
    pub down: CurveShape,
    /// QLab's lock: one shape drives both directions.
    pub mirrored: bool,
}

impl Default for FadeShapes {
    fn default() -> Self {
        // Locked by default: that is the behaviour every existing show has,
        // since a single curve used to drive both directions.
        Self { up: CurveShape::default(), down: CurveShape::default(), mirrored: true }
    }
}

impl FadeShapes {
    /// Both directions on the given kind, locked together.
    pub fn of_kind(kind: CurveKind) -> Self {
        Self {
            up: CurveShape::of_kind(kind),
            down: CurveShape::of_kind(kind),
            mirrored: true,
        }
    }

    /// The shape that applies for the direction the value is travelling.
    pub fn for_direction(&self, rising: bool) -> &CurveShape {
        if rising || self.mirrored { &self.up } else { &self.down }
    }

    /// Progress at `t` for a fade travelling in the given direction.
    pub fn sample(&self, t: f64, rising: bool) -> f64 {
        self.for_direction(rising).sample(t)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Every curve must start at 0 and finish at 1, whatever its shape —
    /// otherwise a fade would not reach its target.
    #[test]
    fn every_kind_spans_the_full_range() {
        for kind in [
            CurveKind::Linear,
            CurveKind::SCurve,
            CurveKind::Exponential,
            CurveKind::Parametric,
        ] {
            let mut shape = CurveShape::of_kind(kind);
            shape.intensity = 3.0;
            shape.points = vec![CurvePoint::new(0.4, 0.8)];
            assert!(close(shape.sample(0.0), 0.0), "{kind:?} must start at 0");
            assert!(close(shape.sample(1.0), 1.0), "{kind:?} must end at 1");
        }
    }

    #[test]
    fn linear_is_the_identity() {
        let shape = CurveShape::of_kind(CurveKind::Linear);
        assert!(close(shape.sample(0.25), 0.25));
        assert!(close(shape.sample(0.5), 0.5));
    }

    #[test]
    fn s_curve_is_symmetric_about_the_midpoint() {
        let shape = CurveShape::of_kind(CurveKind::SCurve);
        assert!(close(shape.sample(0.5), 0.5));
        assert!(close(shape.sample(0.25) + shape.sample(0.75), 1.0));
    }

    #[test]
    fn parametric_intensity_leans_the_curve_each_way() {
        let mut shape = CurveShape::of_kind(CurveKind::Parametric);
        shape.intensity = 0.0;
        assert!(close(shape.sample(0.5), 0.5), "zero intensity is linear");

        shape.intensity = 3.0;
        assert!(shape.sample(0.5) < 0.5, "positive starts slow");
        shape.intensity = -3.0;
        assert!(shape.sample(0.5) > 0.5, "negative starts fast");
    }

    #[test]
    fn a_control_point_is_passed_through_exactly() {
        // The whole promise of the editor: the curve goes where you put it.
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.3, 0.9)];
        assert!(close(shape.sample(0.3), 0.9));
    }

    #[test]
    fn linear_points_make_straight_segments() {
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.5, 0.25)];
        // Halfway into the first segment: half of 0.25.
        assert!(close(shape.sample(0.25), 0.125));
        // Halfway into the second: 0.25 + half of the remaining 0.75.
        assert!(close(shape.sample(0.75), 0.625));
    }

    #[test]
    fn points_are_ordered_and_endpoints_cannot_be_removed() {
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        // Deliberately out of order, plus points on the endpoints that must be
        // ignored rather than duplicating them.
        shape.points = vec![
            CurvePoint::new(0.75, 0.9),
            CurvePoint::new(0.0, 0.5),
            CurvePoint::new(0.25, 0.1),
            CurvePoint::new(1.0, 0.5),
        ];
        let resolved = shape.resolved_points();
        assert_eq!(resolved.first().map(|p| (p.t, p.v)), Some((0.0, 0.0)));
        assert_eq!(resolved.last().map(|p| (p.t, p.v)), Some((1.0, 1.0)));
        let times: Vec<f64> = resolved.iter().map(|p| p.t).collect();
        assert_eq!(times, vec![0.0, 0.25, 0.75, 1.0]);
    }

    #[test]
    fn a_point_out_of_range_is_clamped_not_trusted() {
        let point = CurvePoint::new(-3.0, 42.0);
        assert_eq!((point.t, point.v), (0.0, 1.0));
    }

    // -- Direction ---------------------------------------------------------

    #[test]
    fn a_locked_pair_uses_one_shape_both_ways() {
        let shapes = FadeShapes::of_kind(CurveKind::Exponential);
        assert!(shapes.mirrored);
        assert!(close(shapes.sample(0.3, true), shapes.sample(0.3, false)));
    }

    #[test]
    fn unlocking_lets_the_two_directions_differ() {
        // The point of the feature: an envelope that sounds right going up is
        // not the one that sounds right coming down.
        let mut shapes = FadeShapes::of_kind(CurveKind::Linear);
        shapes.mirrored = false;
        shapes.down = CurveShape::of_kind(CurveKind::SCurve);
        assert!(close(shapes.sample(0.25, true), 0.25), "rising stays linear");
        assert!(!close(shapes.sample(0.25, false), 0.25), "falling follows its own shape");
        assert!(close(shapes.sample(0.25, false), CurveShape::of_kind(CurveKind::SCurve).sample(0.25)));
    }

    #[test]
    fn the_default_is_a_locked_s_curve_so_existing_shows_behave_identically() {
        let shapes = FadeShapes::default();
        assert_eq!(shapes.up.kind, CurveKind::SCurve);
        assert!(shapes.mirrored);
    }

    // -- Baking ------------------------------------------------------------

    #[test]
    fn a_baked_table_tracks_the_shape_it_came_from() {
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.3, 0.7)];
        shape.bends = vec![0.4, -0.3];
        let table = shape.bake();
        for step in 0..=100 {
            let t = step as f64 / 100.0;
            let error = (table.eval(t) - shape.sample(t)).abs();
            assert!(error < 0.01, "t={t}: table {} vs shape {}", table.eval(t), shape.sample(t));
        }
    }

    #[test]
    fn baking_keeps_the_endpoints_exact() {
        let table = CurveShape::of_kind(CurveKind::SCurve).bake();
        assert!(close(table.eval(0.0), 0.0));
        assert!(close(table.eval(1.0), 1.0));
    }

    #[test]
    fn analytic_shapes_reach_the_engine_without_a_table() {
        use crate::engine::ring_command::FadeCurve;
        // Exact evaluation for the common cases; only a drawn curve pays for
        // sampling.
        assert!(matches!(CurveShape::of_kind(CurveKind::SCurve).to_engine(), FadeCurve::SCurve));
        assert!(matches!(CurveShape::of_kind(CurveKind::Linear).to_engine(), FadeCurve::Linear));
        assert!(matches!(
            CurveShape::of_kind(CurveKind::Exponential).to_engine(),
            FadeCurve::Exponential
        ));

        let mut shaped = CurveShape::of_kind(CurveKind::Linear);
        shaped.points = vec![CurvePoint::new(0.5, 0.2)];
        assert!(matches!(shaped.to_engine(), FadeCurve::Table(_)));
    }

    #[test]
    fn serde_roundtrip_keeps_the_points() {
        let mut shapes = FadeShapes::of_kind(CurveKind::Linear);
        shapes.mirrored = false;
        shapes.up.points = vec![CurvePoint::new(0.25, 0.6)];
        shapes.down.intensity = 2.5;

        let json = serde_json::to_string(&shapes).unwrap();
        assert!(json.contains("linear"), "snake_case on the wire: {json}");
        let back: FadeShapes = serde_json::from_str(&json).unwrap();
        assert_eq!(back, shapes);
    }

    #[test]
    fn a_shape_deserialises_from_just_a_kind() {
        // Old shows carry only the curve name; points and intensity default.
        let shape: CurveShape = serde_json::from_str(r#"{"kind":"exponential"}"#).unwrap();
        assert_eq!(shape.kind, CurveKind::Exponential);
        assert!(shape.points.is_empty());
    }

    // -- Bowed segments (Alt-drag) ----------------------------------------

    #[test]
    fn a_bow_lifts_the_segment_without_moving_its_ends() {
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.bends = vec![0.5];
        assert!(close(shape.sample(0.0), 0.0), "start is pinned");
        assert!(close(shape.sample(1.0), 1.0), "end is pinned");
        assert!(shape.sample(0.5) > 0.5, "the middle rides above the chord");
    }

    #[test]
    fn a_negative_bow_sags_the_segment() {
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.bends = vec![-0.5];
        assert!(shape.sample(0.5) < 0.5);
    }

    #[test]
    fn a_bowed_segment_still_lands_on_both_control_points() {
        // The bow warps time inside the segment, so it can never drag the
        // curve off a point the operator placed.
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.4, 0.7)];
        shape.bends = vec![0.8, -0.8];
        assert!(close(shape.sample(0.4), 0.7));
        assert!(close(shape.sample(0.0), 0.0));
        assert!(close(shape.sample(1.0), 1.0));
    }

    #[test]
    fn a_bowed_segment_never_leaves_its_endpoints_range() {
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.5, 0.5)];
        for bend in [-1.0, -0.6, 0.6, 1.0] {
            shape.bends = vec![bend, bend];
            for step in 0..=200 {
                let t = step as f64 / 200.0;
                let v = shape.sample(t);
                assert!((0.0..=1.0).contains(&v), "bend {bend} at t={t} gave {v}");
            }
        }
    }

    #[test]
    fn dragging_puts_the_curve_under_the_cursor() {
        // What Alt-drag promises: let go and the curve passes through where
        // you left it.
        for (local, target) in [(0.5, 0.8), (0.25, 0.1), (0.7, 0.9), (0.5, 0.2)] {
            let bend = bend_through(local, target);
            let mut shape = CurveShape::of_kind(CurveKind::Linear);
            shape.bends = vec![bend];
            let got = shape.sample(local);
            assert!((got - target).abs() < 0.02, "wanted {target} at {local}, got {got}");
        }
    }

    #[test]
    fn an_unbowed_drag_is_the_identity() {
        assert!(close(bend_through(0.5, 0.5), 0.0));
        assert!(close(bow(0.37, 0.0), 0.37));
    }

    #[test]
    fn a_bowed_straight_line_no_longer_reaches_the_engine_as_linear() {
        use crate::engine::ring_command::FadeCurve;
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        assert!(matches!(shape.to_engine(), FadeCurve::Linear));
        shape.bends = vec![0.5];
        assert!(matches!(shape.to_engine(), FadeCurve::Table(_)), "a bow must be baked");
    }

    #[test]
    fn a_short_or_absent_bend_list_is_harmless() {
        // Nothing stops a hand-edited file carrying the wrong count.
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.5, 0.5)];
        shape.bends = vec![];
        assert!(close(shape.sample(0.5), 0.5));
        shape.bends = vec![0.0, 0.0, 0.0, 0.0, 0.0];
        assert!(close(shape.sample(0.5), 0.5));
    }

    #[test]
    fn a_shape_saved_as_custom_loads_as_the_editable_linear_kind() {
        // The smoothing mode was merged into linear-plus-Alt-bend; a file
        // written before that must still open, on the kind that replaced it.
        let shape: CurveShape =
            serde_json::from_str(r#"{"kind":"custom","points":[{"t":0.5,"v":0.2}]}"#).unwrap();
        assert_eq!(shape.kind, CurveKind::Linear);
        assert_eq!(shape.points.len(), 1);
    }

    #[test]
    fn a_new_point_gives_straight_segments_not_a_curve() {
        // Adding a point must not invent a shape: the operator asks for a
        // corner, and reaches for Alt when they want a bow.
        let mut shape = CurveShape::of_kind(CurveKind::Linear);
        shape.points = vec![CurvePoint::new(0.5, 0.25)];
        assert!(close(shape.sample(0.25), 0.125), "first segment is straight");
        assert!(close(shape.sample(0.75), 0.625), "second segment is straight");
    }
}
