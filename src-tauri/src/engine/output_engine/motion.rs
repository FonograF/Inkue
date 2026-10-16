//! Layer motion: moving a visual layer while it plays (geometry and path fades).
//!
//! A [`MotionProgram`] is handed to a slot once, when a fade starts; the
//! render thread then evaluates it **every frame** ([`MotionRun::sample`]), so
//! a moving picture is as smooth as the output's refresh rate — the Fade
//! Cue's own ~30 fps tick would judder.  The result, a [`LayerMotion`], is a
//! transform of the whole layer in output space, applied by the composite
//! shader through [`sample_matrix`].  That is QLab's model: translation moves
//! the layer, letterbox bars included, on top of the cue's own geometry.
//!
//! Pure math, no GL and no locks: everything here is unit-tested on its own.
//! The QLab behaviour it reproduces was measured frame by frame from screen
//! recordings of QLab 5 (build 5603) moving a still image: constant speed by
//! path length (a long run then a short hop kept one speed); smoothing that
//! fits Catmull-Rom to ~13 px (straight segments: 60 px); merge joining the
//! path at the merge fraction of both time and length; each loop pass
//! restarting from the layer's starting position.
//!
//! ## Units
//!
//! | Field | Unit |
//! |---|---|
//! | `tx`, `ty` | fraction of the output's width / height; centre origin, **Y up** (QLab's convention) |
//! | `rotation_deg` | degrees, clockwise (like [`VideoGeometry::rotation`](super::VideoGeometry)) |
//! | `scale` | linear factor, 1.0 = unchanged |
//!
//! Paths are measured in output **pixels** (x scaled by the aspect ratio), so
//! "constant speed" and the spline's shape match what the audience sees, and
//! what QLab does with stage pixels.

use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::engine::ring_command::FadeCurve;

/// Samples taken along each span of a smoothed path.  The distance table
/// interpolates linearly between them; 32 keeps the error well under a pixel
/// for any path that fits on a stage.
const SPLINE_SAMPLES_PER_SPAN: usize = 32;

/// Two path points closer than this (in metric units) are one point: a zero
/// span would break the centripetal parameterisation.
const POINT_EPSILON: f64 = 1e-9;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A layer's runtime offset from its cue geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerMotion {
    /// Horizontal offset, fraction of the output width (+ = right).
    pub tx: f64,
    /// Vertical offset, fraction of the output height (+ = up).
    pub ty: f64,
    /// Clockwise rotation about the output centre, in degrees.
    pub rotation_deg: f64,
    /// Scale about the output centre (1.0 = unchanged).
    pub scale: f64,
}

impl LayerMotion {
    /// No offset: the layer exactly as its cue geometry draws it.
    pub const IDENTITY: Self = Self { tx: 0.0, ty: 0.0, rotation_deg: 0.0, scale: 1.0 };

    /// `true` when this motion leaves the layer untouched.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    fn get(&self, param: MotionParam) -> f64 {
        match param {
            MotionParam::TranslationX => self.tx,
            MotionParam::TranslationY => self.ty,
        }
    }

    fn set(&mut self, param: MotionParam, value: f64) {
        match param {
            MotionParam::TranslationX => self.tx = value,
            MotionParam::TranslationY => self.ty = value,
        }
    }
}

impl Default for LayerMotion {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A layer property one axis of a path drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionParam {
    /// [`LayerMotion::tx`].
    TranslationX,
    /// [`LayerMotion::ty`].
    TranslationY,
}

impl MotionParam {
    /// How many output pixels one unit of this parameter spans, per pixel of
    /// output height — what turns a path into the metric space it is
    /// measured in.
    fn metric_scale(self, aspect: f64) -> f64 {
        match self {
            MotionParam::TranslationX => aspect,
            MotionParam::TranslationY => 1.0,
        }
    }
}

/// Curve mode: fade some properties from where they are to a target.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveMotion {
    /// Target horizontal offset; `None` = not faded.
    pub tx: Option<f64>,
    /// Target vertical offset; `None` = not faded.
    pub ty: Option<f64>,
    /// Target rotation in degrees; `None` = not faded.
    pub rotation_deg: Option<f64>,
    /// Target scale; `None` = not faded.
    pub scale: Option<f64>,
    /// Fade duration.
    pub duration: Duration,
    /// Envelope for a property whose value increases.
    pub rising: FadeCurve,
    /// Envelope for a property whose value decreases.
    pub falling: FadeCurve,
}

/// Path mode: move two properties along a drawn 2D path at constant speed.
#[derive(Debug, Clone, PartialEq)]
pub struct PathMotion {
    /// Path points `[x, y]`, start first, in the units of `x_param` / `y_param`.
    pub points: Vec<[f64; 2]>,
    /// Catmull-Rom spline through the points (`false` = straight segments).
    pub smooth: bool,
    /// Fraction of the duration (0–1) spent joining the path from the
    /// layer's current value; the path before that point is skipped.
    pub merge: f64,
    /// Repeat until stopped; every pass restarts from the starting value.
    pub looping: bool,
    /// Time for one pass.
    pub duration: Duration,
    /// Property driven by the path's horizontal axis.
    pub x_param: MotionParam,
    /// Property driven by the path's vertical axis.
    pub y_param: MotionParam,
}

/// What a fade does to a layer.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionProgram {
    /// Fade to target values.  Boxed: two baked curve tables make it the
    /// large variant.
    Curve(Box<CurveMotion>),
    /// Follow a path.
    Path(PathMotion),
}

/// How a stopped motion leaves its layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionStop {
    /// Keep the layer where the motion has taken it.
    Hold,
    /// Return the layer to where it was before the motion started.
    Revert,
}

// ---------------------------------------------------------------------------
// MotionRun — one program running on one layer
// ---------------------------------------------------------------------------

/// A program in flight on a layer, with its own pausable clock.
pub(super) struct MotionRun {
    /// Who started it (the Fade Cue's id).  Only the owner may pause or stop
    /// it: a later fade on the same layer replaces the run, and the earlier
    /// fade stopping afterwards must not cancel the newer one.
    owner: Uuid,
    program: MotionProgram,
    /// The layer's offset when the program started (merge source, curve
    /// start, loop restart point, revert target).
    start: LayerMotion,
    started_at: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    /// The path in metric space, built for one aspect ratio.  Rebuilt if the
    /// output is resized mid-run.
    compiled: Option<(f64, CompiledPath)>,
}

impl MotionRun {
    /// Start `program` for `owner` from `start` at `now`.
    pub(super) fn new(owner: Uuid, program: MotionProgram, start: LayerMotion, now: Instant) -> Self {
        Self {
            owner,
            program,
            start,
            started_at: now,
            paused_at: None,
            paused_total: Duration::ZERO,
            compiled: None,
        }
    }

    /// `true` when `owner` started this run.
    pub(super) fn is_owned_by(&self, owner: Uuid) -> bool {
        self.owner == owner
    }

    /// The layer's offset before this run.
    pub(super) fn start(&self) -> LayerMotion {
        self.start
    }

    /// `true` while the clock is held.
    pub(super) fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Hold the clock.  No-op when already paused.
    pub(super) fn pause(&mut self, now: Instant) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    /// Release the clock.  No-op when running.
    pub(super) fn resume(&mut self, now: Instant) {
        if let Some(at) = self.paused_at.take() {
            self.paused_total += now.saturating_duration_since(at);
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        let until = self.paused_at.unwrap_or(now);
        until.saturating_duration_since(self.started_at).saturating_sub(self.paused_total)
    }

    /// The layer's offset at `now` on an output of `aspect` (width / height),
    /// and whether the program has finished (never, for a looping path).
    pub(super) fn sample(&mut self, now: Instant, aspect: f64) -> (LayerMotion, bool) {
        let t = self.elapsed(now).as_secs_f64();
        match &self.program {
            MotionProgram::Curve(curve) => sample_curve(curve, self.start, t),
            MotionProgram::Path(path) => {
                let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
                if self.compiled.as_ref().is_some_and(|(a, _)| (*a - aspect).abs() > 1e-9) {
                    self.compiled = None;
                }
                let (_, compiled) = self
                    .compiled
                    .get_or_insert_with(|| (aspect, CompiledPath::new(path, aspect)));
                sample_path(path, compiled, self.start, aspect, t)
            }
        }
    }
}

fn sample_curve(curve: &CurveMotion, start: LayerMotion, t: f64) -> (LayerMotion, bool) {
    let total = curve.duration.as_secs_f64();
    let p = if total <= 0.0 { 1.0 } else { (t / total).clamp(0.0, 1.0) };
    let lerp = |from: f64, to: Option<f64>| match to {
        None => from,
        Some(to) => {
            let envelope = if to >= from { curve.rising } else { curve.falling };
            from + (to - from) * envelope.apply(p)
        }
    };
    let motion = LayerMotion {
        tx: lerp(start.tx, curve.tx),
        ty: lerp(start.ty, curve.ty),
        rotation_deg: lerp(start.rotation_deg, curve.rotation_deg),
        scale: lerp(start.scale, curve.scale),
    };
    (motion, p >= 1.0)
}

fn sample_path(
    path: &PathMotion,
    compiled: &CompiledPath,
    start: LayerMotion,
    aspect: f64,
    t: f64,
) -> (LayerMotion, bool) {
    let total = path.duration.as_secs_f64();
    let (pass_t, finished) = if total <= 0.0 {
        (total, true)
    } else if path.looping {
        (t % total, false)
    } else {
        (t.min(total), t >= total)
    };
    let p = if total <= 0.0 { 1.0 } else { pass_t / total };
    let (sx, sy) = (path.x_param.metric_scale(aspect), path.y_param.metric_scale(aspect));
    let merge = path.merge.clamp(0.0, 1.0);

    let point = if merge > 0.0 && p < merge {
        // Join the path: Hermite from the layer's start (at rest) to the path
        // point at `merge`, arriving with the path's own velocity.
        let from = [start.get(path.x_param) * sx, start.get(path.y_param) * sy];
        let (to, tangent) = compiled.at_distance(merge * compiled.length);
        let arrive = [tangent[0] * compiled.length * merge, tangent[1] * compiled.length * merge];
        hermite(from, to, arrive, p / merge)
    } else {
        compiled.at_distance(p * compiled.length).0
    };

    let mut motion = start;
    motion.set(path.x_param, point[0] / sx);
    motion.set(path.y_param, point[1] / sy);
    (motion, finished)
}

/// Cubic Hermite from `p0` (zero velocity) to `p1` with end tangent `m1`.
fn hermite(p0: [f64; 2], p1: [f64; 2], m1: [f64; 2], u: f64) -> [f64; 2] {
    let u = u.clamp(0.0, 1.0);
    let (u2, u3) = (u * u, u * u * u);
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    [
        h00 * p0[0] + h01 * p1[0] + h11 * m1[0],
        h00 * p0[1] + h01 * p1[1] + h11 * m1[1],
    ]
}

// ---------------------------------------------------------------------------
// CompiledPath — a path as a distance table
// ---------------------------------------------------------------------------

/// A path flattened to a polyline in metric space, with the distance from
/// the start to every vertex: playback is a binary search, not a spline
/// evaluation.
#[derive(Debug, Clone)]
struct CompiledPath {
    vertices: Vec<[f64; 2]>,
    /// `distances[i]` = path length from the start to `vertices[i]`.
    distances: Vec<f64>,
    length: f64,
}

impl CompiledPath {
    fn new(path: &PathMotion, aspect: f64) -> Self {
        let (sx, sy) = (path.x_param.metric_scale(aspect), path.y_param.metric_scale(aspect));
        let mut points: Vec<[f64; 2]> = Vec::with_capacity(path.points.len());
        for p in &path.points {
            let metric = [p[0] * sx, p[1] * sy];
            if points.last().is_none_or(|last| distance(*last, metric) > POINT_EPSILON) {
                points.push(metric);
            }
        }
        let vertices = if path.smooth && points.len() > 2 { catmull_rom(&points) } else { points };
        Self::from_vertices(vertices)
    }

    fn from_vertices(vertices: Vec<[f64; 2]>) -> Self {
        let mut distances = Vec::with_capacity(vertices.len());
        let mut length = 0.0;
        for (i, v) in vertices.iter().enumerate() {
            if i > 0 {
                length += distance(vertices[i - 1], *v);
            }
            distances.push(length);
        }
        Self { vertices, distances, length }
    }

    /// The point at `d` along the path, and the unit tangent there.  An empty
    /// path sits at the origin; a single point or a zero-length path has no
    /// direction (tangent zero).
    fn at_distance(&self, d: f64) -> ([f64; 2], [f64; 2]) {
        match self.vertices.len() {
            0 => return ([0.0, 0.0], [0.0, 0.0]),
            1 => return (self.vertices[0], [0.0, 0.0]),
            _ => {}
        }
        if self.length <= 0.0 {
            return (self.vertices[0], [0.0, 0.0]);
        }
        let d = d.clamp(0.0, self.length);
        // First vertex at or past `d`; the span ending there contains it.
        let end = self.distances.partition_point(|&x| x < d).clamp(1, self.vertices.len() - 1);
        let (a, b) = (self.vertices[end - 1], self.vertices[end]);
        let span = self.distances[end] - self.distances[end - 1];
        let u = if span > 0.0 { (d - self.distances[end - 1]) / span } else { 0.0 };
        let point = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
        let tangent = if span > 0.0 { [(b[0] - a[0]) / span, (b[1] - a[1]) / span] } else { [0.0, 0.0] };
        (point, tangent)
    }
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Centripetal Catmull-Rom (α = 0.5) through `points`, flattened.  The end
/// points are reflected to give the first and last spans a tangent.
///
/// QLab's smoothing matched Catmull-Rom to within the measurement error, and
/// the variants were indistinguishable; centripetal is the one that never
/// forms cusps or self-loops on unevenly spaced points.
fn catmull_rom(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = points.len();
    let reflect = |a: [f64; 2], b: [f64; 2]| [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]];
    let mut controls = Vec::with_capacity(n + 2);
    controls.push(reflect(points[0], points[1]));
    controls.extend_from_slice(points);
    controls.push(reflect(points[n - 1], points[n - 2]));

    let mut out = Vec::with_capacity((n - 1) * SPLINE_SAMPLES_PER_SPAN + 1);
    for span in 0..n - 1 {
        let [p0, p1, p2, p3] = [controls[span], controls[span + 1], controls[span + 2], controls[span + 3]];
        let knot = |ti: f64, a: [f64; 2], b: [f64; 2]| ti + distance(a, b).max(POINT_EPSILON).sqrt();
        let t0 = 0.0;
        let t1 = knot(t0, p0, p1);
        let t2 = knot(t1, p1, p2);
        let t3 = knot(t2, p2, p3);
        for k in 0..SPLINE_SAMPLES_PER_SPAN {
            let t = t1 + (t2 - t1) * k as f64 / SPLINE_SAMPLES_PER_SPAN as f64;
            let lerp = |a: [f64; 2], b: [f64; 2], ta: f64, tb: f64| {
                let w = (t - ta) / (tb - ta);
                [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
            };
            let a1 = lerp(p0, p1, t0, t1);
            let a2 = lerp(p1, p2, t1, t2);
            let a3 = lerp(p2, p3, t2, t3);
            let b1 = lerp(a1, a2, t0, t2);
            let b2 = lerp(a2, a3, t1, t3);
            out.push(lerp(b1, b2, t1, t2));
        }
    }
    out.push(points[n - 1]);
    out
}

// ---------------------------------------------------------------------------
// Shader matrix
// ---------------------------------------------------------------------------

/// The composite shader's `u_layer_xform`: maps an output UV to the layer UV
/// to sample there (the **inverse** of the layer's motion), as a GL
/// column-major `mat3`.  UVs are `[0, 1]²` with Y up, like `v_uv`.
///
/// Rotation and scale are done in pixels, about the output centre, so a
/// 16:9 layer turns without shearing.
pub(super) fn sample_matrix(motion: &LayerMotion, width: u32, height: u32) -> [f32; 9] {
    let m = sample_matrix_rows(motion, width, height);
    // Row-major → column-major.
    [
        m[0][0] as f32, m[1][0] as f32, m[2][0] as f32,
        m[0][1] as f32, m[1][1] as f32, m[2][1] as f32,
        m[0][2] as f32, m[1][2] as f32, m[2][2] as f32,
    ]
}

/// [`sample_matrix`] in row-major `f64`, for tests and composition.
fn sample_matrix_rows(motion: &LayerMotion, width: u32, height: u32) -> [[f64; 3]; 3] {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = if motion.scale.abs() < 1e-6 { 1e-6 } else { motion.scale };
    let (sin, cos) = motion.rotation_deg.to_radians().sin_cos();
    // Output UV → centred pixels → remove translation → undo clockwise
    // rotation → undo scale → layer UV.
    let to_px = [[w, 0.0, -0.5 * w], [0.0, h, -0.5 * h], [0.0, 0.0, 1.0]];
    let untranslate = [[1.0, 0.0, -motion.tx * w], [0.0, 1.0, -motion.ty * h], [0.0, 0.0, 1.0]];
    // Clockwise by θ (Y up) is (x cos + y sin, −x sin + y cos); its inverse:
    let unrotate = [[cos / scale, -sin / scale, 0.0], [sin / scale, cos / scale, 0.0], [0.0, 0.0, 1.0]];
    let to_uv = [[1.0 / w, 0.0, 0.5], [0.0, 1.0 / h, 0.5], [0.0, 0.0, 1.0]];
    mul(to_uv, mul(unrotate, mul(untranslate, to_px)))
}

fn mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

/// The identity `u_layer_xform` (no motion), column-major.
pub(super) const IDENTITY_MATRIX: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ASPECT: f64 = 16.0 / 9.0;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    fn path(points: &[[f64; 2]]) -> PathMotion {
        PathMotion {
            points: points.to_vec(),
            smooth: false,
            merge: 0.0,
            looping: false,
            duration: secs(3.0),
            x_param: MotionParam::TranslationX,
            y_param: MotionParam::TranslationY,
        }
    }

    /// Sample `program` at `t` seconds after its start.
    fn at(program: &MotionProgram, start: LayerMotion, t: f64) -> (LayerMotion, bool) {
        let t0 = Instant::now();
        let mut run = MotionRun::new(Uuid::nil(), program.clone(), start, t0);
        run.sample(t0 + secs(t), ASPECT)
    }

    fn apply(m: [[f64; 3]; 3], uv: [f64; 2]) -> [f64; 2] {
        [
            m[0][0] * uv[0] + m[0][1] * uv[1] + m[0][2],
            m[1][0] * uv[0] + m[1][1] * uv[1] + m[1][2],
        ]
    }

    // ── Path: constant speed ────────────────────────────────────────────────

    #[test]
    fn path_moves_at_constant_speed_across_uneven_segments() {
        // QLab measurement: a long run then a short hop, no slow-down on the hop.
        // Horizontal only, so metric distance = Δx · aspect for both spans.
        let program = MotionProgram::Path(path(&[[-0.4, 0.0], [0.3, 0.0], [0.4, 0.0]]));
        let start = LayerMotion::IDENTITY;
        // Total 0.8 of width over 3 s → 0.8/3 per second, on both spans.
        for t in [0.5, 1.5, 2.5, 2.9] {
            let (m, _) = at(&program, start, t);
            assert!(close(m.tx, -0.4 + 0.8 * t / 3.0, 1e-9), "t={t}: {}", m.tx);
        }
    }

    #[test]
    fn path_speed_is_measured_in_pixels_not_fractions() {
        // Right 0.1 of width, then up 0.1 of height. On 16:9 the first span is
        // 1.78× longer on screen, so it must take 1.78× the time.
        let program = MotionProgram::Path(path(&[[0.0, 0.0], [0.1, 0.0], [0.1, 0.1]]));
        let first = 0.1 * ASPECT;
        let corner_t = 3.0 * first / (first + 0.1);
        let (m, _) = at(&program, LayerMotion::IDENTITY, corner_t);
        assert!(close(m.tx, 0.1, 1e-9) && close(m.ty, 0.0, 1e-9), "{m:?}");
    }

    #[test]
    fn path_ends_on_its_last_point_and_finishes() {
        let program = MotionProgram::Path(path(&[[0.0, 0.0], [0.2, 0.1]]));
        let (m, done) = at(&program, LayerMotion::IDENTITY, 3.0);
        assert!(done);
        assert!(close(m.tx, 0.2, 1e-12) && close(m.ty, 0.1, 1e-12));
        let (_, done) = at(&program, LayerMotion::IDENTITY, 2.99);
        assert!(!done);
    }

    #[test]
    fn path_leaves_unmapped_properties_alone() {
        let start = LayerMotion { tx: 0.0, ty: 0.0, rotation_deg: 30.0, scale: 2.0 };
        let (m, _) = at(&MotionProgram::Path(path(&[[0.0, 0.0], [0.2, 0.0]])), start, 1.0);
        assert_eq!((m.rotation_deg, m.scale), (30.0, 2.0));
    }

    #[test]
    fn path_axes_follow_the_parameter_map() {
        let mut p = path(&[[0.0, 0.0], [0.0, 0.3]]);
        p.x_param = MotionParam::TranslationY;
        p.y_param = MotionParam::TranslationX;
        let (m, _) = at(&MotionProgram::Path(p), LayerMotion::IDENTITY, 3.0);
        assert!(close(m.tx, 0.3, 1e-12) && close(m.ty, 0.0, 1e-12), "{m:?}");
    }

    // ── Path: smoothing ─────────────────────────────────────────────────────

    #[test]
    fn smooth_path_passes_through_every_point() {
        let points = [[-0.4, -0.3], [0.35, -0.3], [-0.4, 0.3], [0.4, 0.35]];
        let mut p = path(&points);
        p.smooth = true;
        let compiled = CompiledPath::new(&p, ASPECT);
        for pt in points {
            let metric = [pt[0] * ASPECT, pt[1]];
            let nearest = compiled
                .vertices
                .iter()
                .map(|v| distance(*v, metric))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest < 1e-9, "{pt:?} missed by {nearest}");
        }
    }

    #[test]
    fn smooth_path_bends_between_the_points() {
        // A right angle. Catmull-Rom interpolates, so the corner point is still
        // on the curve; what smoothing changes is the spans either side: they
        // bow out instead of running straight along y = 0 and x = 0.
        let mut straight = path(&[[-0.2, 0.0], [0.0, 0.0], [0.0, 0.2]]);
        straight.x_param = MotionParam::TranslationY; // square metric: both axes ×1
        let mut smooth = straight.clone();
        smooth.smooth = true;
        let s = CompiledPath::new(&straight, ASPECT);
        let c = CompiledPath::new(&smooth, ASPECT);
        let (on_line, _) = s.at_distance(s.length / 4.0);
        assert!(on_line[1].abs() < 1e-12, "straight span stays on y = 0");
        // Mid first span (by vertex count: half of the first span's samples).
        let bowed = c.vertices[SPLINE_SAMPLES_PER_SPAN / 2];
        assert!(bowed[1].abs() > 0.005, "smooth span bows off y = 0: {bowed:?}");
        // No kink at the corner: across the same short stretch the straight
        // path turns the full 90° at once, the smooth one well under half that.
        let turn = |p: &CompiledPath| {
            let (_, before) = p.at_distance(p.length / 2.0 - 0.01);
            let (_, after) = p.at_distance(p.length / 2.0 + 0.01);
            (before[0] * after[0] + before[1] * after[1]).clamp(-1.0, 1.0).acos().to_degrees()
        };
        assert!((turn(&s) - 90.0).abs() < 1e-6, "straight: {}°", turn(&s));
        assert!(turn(&c) < 45.0, "smooth: {}°", turn(&c));
    }

    #[test]
    fn two_point_smooth_path_is_the_straight_line() {
        let mut p = path(&[[0.0, 0.0], [0.3, 0.0]]);
        p.smooth = true;
        let (m, _) = at(&MotionProgram::Path(p), LayerMotion::IDENTITY, 1.5);
        assert!(close(m.tx, 0.15, 1e-9) && close(m.ty, 0.0, 1e-9));
    }

    /// The path editor (`src/lib/path.ts`) redraws this flattening in the UI;
    /// `src/lib/__tests__/path.test.ts` asserts these exact numbers, so a
    /// change on either side fails one of the two.
    #[test]
    fn shared_path_vector() {
        let mut p = path(&[[-0.4, -0.3], [0.35, -0.3], [-0.4, 0.3], [0.4, 0.35]]);
        p.smooth = true;
        let compiled = CompiledPath::new(&p, ASPECT);
        assert_eq!(compiled.vertices.len(), 97);
        let expected = [
            (0, [-0.4, -0.3]),
            (16, [0.06452616993973369, -0.33498517066005307]),
            (48, [-0.02504220802072924, -0.003799651069123458]),
            (80, [-0.09559465867115714, 0.35864230399818736]),
            (96, [0.4, 0.35]),
        ];
        for (index, [x, y]) in expected {
            let v = compiled.vertices[index];
            let (vx, vy) = (v[0] / ASPECT, v[1]);
            assert!(close(vx, x, 1e-12) && close(vy, y, 1e-12), "vertex {index}: ({vx}, {vy})");
        }
    }

    #[test]
    fn repeated_points_do_not_break_the_spline() {
        let mut p = path(&[[0.0, 0.0], [0.1, 0.0], [0.1, 0.0], [0.2, 0.1]]);
        p.smooth = true;
        let compiled = CompiledPath::new(&p, ASPECT);
        assert!(compiled.length.is_finite() && compiled.length > 0.0);
        assert!(compiled.vertices.iter().all(|v| v[0].is_finite() && v[1].is_finite()));
    }

    // ── Path: merge ─────────────────────────────────────────────────────────

    #[test]
    fn merge_starts_at_the_layer_and_joins_at_the_merge_point() {
        let mut p = path(&[[-0.4, 0.1], [0.4, 0.1]]);
        p.merge = 0.5;
        let program = MotionProgram::Path(p);
        let start = LayerMotion::IDENTITY;
        let (m, _) = at(&program, start, 0.0);
        assert!(close(m.tx, 0.0, 1e-12) && close(m.ty, 0.0, 1e-12), "starts where the layer is");
        // At 50 % of the time it is on the path at 50 % of its length.
        let (m, _) = at(&program, start, 1.5);
        assert!(close(m.tx, 0.0, 1e-9) && close(m.ty, 0.1, 1e-9), "{m:?}");
        // After that it follows the path.
        let (m, _) = at(&program, start, 2.25);
        assert!(close(m.tx, 0.2, 1e-9) && close(m.ty, 0.1, 1e-9), "{m:?}");
    }

    #[test]
    fn merge_skips_the_start_of_the_path() {
        // QLab: with merge 50 % the target never goes near the path's start.
        let mut p = path(&[[-0.4, 0.0], [0.4, 0.0]]);
        p.merge = 0.5;
        let program = MotionProgram::Path(p);
        let min_x = (0..=150)
            .map(|i| at(&program, LayerMotion::IDENTITY, i as f64 * 0.01).0.tx)
            .fold(f64::INFINITY, f64::min);
        assert!(min_x > -0.2, "came within {min_x} of the start");
    }

    #[test]
    fn merge_leaves_the_layer_at_rest_and_arrives_moving() {
        let mut p = path(&[[-0.4, 0.2], [0.4, 0.2]]);
        p.merge = 0.5;
        let program = MotionProgram::Path(p);
        let start = LayerMotion { tx: -0.1, ty: -0.2, ..LayerMotion::IDENTITY };
        let dt = 1e-4;
        let speed = |t: f64| {
            let (a, _) = at(&program, start, t);
            let (b, _) = at(&program, start, t + dt);
            ((b.tx - a.tx) * ASPECT).hypot(b.ty - a.ty) / dt
        };
        assert!(speed(0.0) < 0.01, "starts at rest");
        let path_speed = 0.8 * ASPECT / 3.0;
        assert!(close(speed(1.5 - 2.0 * dt), path_speed, 0.01), "arrives at path speed");
    }

    #[test]
    fn zero_merge_starts_on_the_path() {
        let program = MotionProgram::Path(path(&[[-0.4, 0.0], [0.4, 0.0]]));
        let (m, _) = at(&program, LayerMotion::IDENTITY, 0.0);
        assert!(close(m.tx, -0.4, 1e-12));
    }

    // ── Path: loop ──────────────────────────────────────────────────────────

    #[test]
    fn looping_path_restarts_from_the_layer_each_pass() {
        let mut p = path(&[[-0.4, 0.0], [0.4, 0.0]]);
        p.merge = 0.5;
        p.looping = true;
        let program = MotionProgram::Path(p);
        let start = LayerMotion { tx: 0.05, ..LayerMotion::IDENTITY };
        for pass in 0..3 {
            let t = pass as f64 * 3.0;
            let (m, done) = at(&program, start, t);
            assert!(!done, "a looping path never finishes");
            assert!(close(m.tx, 0.05, 1e-9), "pass {pass} starts at the layer: {}", m.tx);
            let (m, _) = at(&program, start, t + 2.25);
            assert!(close(m.tx, 0.2, 1e-9), "pass {pass} follows the path");
        }
    }

    // ── Clock ───────────────────────────────────────────────────────────────

    #[test]
    fn pause_holds_the_position() {
        let t0 = Instant::now();
        let program = MotionProgram::Path(path(&[[0.0, 0.0], [0.3, 0.0]]));
        let mut run = MotionRun::new(Uuid::nil(), program, LayerMotion::IDENTITY, t0);
        run.pause(t0 + secs(1.0));
        let held = run.sample(t0 + secs(2.5), ASPECT).0.tx;
        assert!(close(held, 0.1, 1e-9), "held at 1 s: {held}");
        run.resume(t0 + secs(4.0));
        // 3 s paused: wall 5 s = 2 s of motion.
        let moved = run.sample(t0 + secs(5.0), ASPECT).0.tx;
        assert!(close(moved, 0.2, 1e-9), "{moved}");
        assert!(!run.is_paused());
    }

    #[test]
    fn resized_output_rebuilds_the_path() {
        // Same fractions, different aspect: the corner time moves with it.
        let program = MotionProgram::Path(path(&[[0.0, 0.0], [0.1, 0.0], [0.1, 0.1]]));
        let t0 = Instant::now();
        let mut run = MotionRun::new(Uuid::nil(), program, LayerMotion::IDENTITY, t0);
        let _ = run.sample(t0, ASPECT);
        let (m, _) = run.sample(t0 + secs(1.5), 1.0); // square: corner at half time
        assert!(close(m.tx, 0.1, 1e-9) && close(m.ty, 0.0, 1e-9), "{m:?}");
    }

    // ── Curve ───────────────────────────────────────────────────────────────

    #[test]
    fn curve_fades_only_the_chosen_properties() {
        let curve = CurveMotion {
            tx: Some(0.5),
            ty: None,
            rotation_deg: Some(90.0),
            scale: None,
            duration: secs(2.0),
            rising: FadeCurve::Linear,
            falling: FadeCurve::Linear,
        };
        let start = LayerMotion { tx: 0.1, ty: 0.2, rotation_deg: 0.0, scale: 1.5 };
        let (m, done) = at(&MotionProgram::Curve(Box::new(curve.clone())), start, 1.0);
        assert!(!done);
        assert!(close(m.tx, 0.3, 1e-12) && close(m.rotation_deg, 45.0, 1e-9));
        assert_eq!((m.ty, m.scale), (0.2, 1.5));
        let (m, done) = at(&MotionProgram::Curve(Box::new(curve)), start, 2.0);
        assert!(done && close(m.tx, 0.5, 1e-12) && close(m.rotation_deg, 90.0, 1e-12));
    }

    #[test]
    fn curve_uses_the_falling_envelope_for_a_falling_value() {
        let curve = CurveMotion {
            tx: Some(0.0),
            ty: Some(1.0),
            rotation_deg: None,
            scale: None,
            duration: secs(1.0),
            rising: FadeCurve::Linear,
            falling: FadeCurve::SCurve,
        };
        let start = LayerMotion { tx: 1.0, ..LayerMotion::IDENTITY };
        let (m, _) = at(&MotionProgram::Curve(Box::new(curve)), start, 0.25);
        // tx falls on the S-curve (0.15625 of the way), ty rises linearly.
        assert!(close(m.tx, 1.0 - 0.15625, 1e-9), "{}", m.tx);
        assert!(close(m.ty, 0.25, 1e-9), "{}", m.ty);
    }

    // ── Shader matrix ───────────────────────────────────────────────────────

    #[test]
    fn identity_motion_samples_in_place() {
        let m = sample_matrix_rows(&LayerMotion::IDENTITY, 1920, 1080);
        for uv in [[0.0, 0.0], [0.3, 0.8], [1.0, 1.0]] {
            let s = apply(m, uv);
            assert!(close(s[0], uv[0], 1e-12) && close(s[1], uv[1], 1e-12));
        }
        let gl = sample_matrix(&LayerMotion::IDENTITY, 1920, 1080);
        assert!(gl.iter().zip(IDENTITY_MATRIX).all(|(a, b)| (a - b).abs() < 1e-6), "{gl:?}");
    }

    #[test]
    fn translation_moves_the_layer_right_and_up() {
        let motion = LayerMotion { tx: 0.25, ty: 0.1, ..LayerMotion::IDENTITY };
        let m = sample_matrix_rows(&motion, 1920, 1080);
        // The layer's centre is now drawn at (0.75, 0.6).
        let s = apply(m, [0.75, 0.6]);
        assert!(close(s[0], 0.5, 1e-12) && close(s[1], 0.5, 1e-12), "{s:?}");
    }

    #[test]
    fn rotation_is_clockwise_and_keeps_the_aspect_ratio() {
        let (w, h) = (1920u32, 1080u32);
        let motion = LayerMotion { rotation_deg: 90.0, ..LayerMotion::IDENTITY };
        let m = sample_matrix_rows(&motion, w, h);
        // Clockwise 90°: the layer pixel 100 px above centre lands 100 px right.
        let out = [0.5 + 100.0 / f64::from(w), 0.5];
        let s = apply(m, out);
        assert!(close(s[0], 0.5, 1e-12), "{s:?}");
        assert!(close(s[1], 0.5 + 100.0 / f64::from(h), 1e-12), "{s:?}");
    }

    #[test]
    fn scale_grows_about_the_centre() {
        let motion = LayerMotion { scale: 2.0, ..LayerMotion::IDENTITY };
        let m = sample_matrix_rows(&motion, 1920, 1080);
        let s = apply(m, [0.7, 0.3]);
        assert!(close(s[0], 0.6, 1e-12) && close(s[1], 0.4, 1e-12), "{s:?}");
    }

    #[test]
    fn gl_matrix_is_column_major() {
        let motion = LayerMotion { tx: 0.25, ..LayerMotion::IDENTITY };
        let gl = sample_matrix(&motion, 1000, 1000);
        // Translation sits in the third column: elements 6 and 7.
        assert!(close(f64::from(gl[6]), -0.25, 1e-6) && close(f64::from(gl[7]), 0.0, 1e-6), "{gl:?}");
    }

    #[test]
    fn zero_scale_does_not_produce_nan() {
        let motion = LayerMotion { scale: 0.0, ..LayerMotion::IDENTITY };
        assert!(sample_matrix(&motion, 1920, 1080).iter().all(|v| v.is_finite()));
    }
}
