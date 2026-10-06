//! Crossfade — one visual cue dissolving into another, with no mpv or GL.
//!
//! A true dissolve between the picture **with** the outgoing content (`X`) and
//! the picture **with** the incoming content (`Y`) is `X·(1−w) + Y·w`.  Fading
//! two layer opacities cannot produce that: the compositor stacks layers with
//! "over", so `A: 1→0, B: 0→1` gives `B·w + A·(1−w)²` — a dip to black halfway
//! — and animating only the upper layer is exact only where the lower one is
//! opaque: a letterboxed photo dissolving over a full-frame one leaves the old
//! picture standing in the bars, then cuts it at the end.
//!
//! So a dissolve is rendered the way it is defined.  The output composites its
//! whole layer stack once **without the incoming layer** and once **without
//! the outgoing ones**, and mixes the two pictures with the weight `w`
//! ([`plan_dissolves`]).  Blend modes, layers in between and transparent pixels
//! all come out right because nothing is approximated.  Several dissolves in
//! flight on one output expand into every combination (`2ⁿ` composites, each
//! weighted by the product of its dissolves' weights); a dissolve at rest
//! (`w` = 0 or 1) costs nothing extra.
//!
//! An outgoing layer on **another** output has no picture to share with the
//! incoming one: it simply fades out on its own window while the incoming one
//! fades in on its own ([`Departure::Faded`]).
//!
//! Every dissolve runs on a [`DissolveClock`] that starts on the incoming
//! content's first visible frame — a video takes a moment to decode, a camera
//! longer — and pauses with the Fade Cue that drives it.

use std::time::{Duration, Instant};

use super::types::{CrossfadePhase, VoiceId};
use crate::engine::ring_command::FadeCurve;

/// Dissolves rendered exactly at the same time on one output.  Each one in
/// flight doubles the composites of that output's frame, so a third
/// simultaneous dissolve (rare: three overlapping crossfades on one screen) is
/// approximated instead — see [`plan_dissolves`].
pub(super) const MAX_EXACT_DISSOLVES: usize = 2;

/// A weight this close to 0 or 1 is at rest: no extra composite for it.
const AT_REST: f32 = 1e-4;

// ---------------------------------------------------------------------------
// The link
// ---------------------------------------------------------------------------

/// How an outgoing layer leaves during a crossfade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Departure {
    /// Same output as the incoming content: dissolved by the compositor.
    Mixed,
    /// Another output: its own layer opacity fades from `from` to 0.
    Faded { from: f32 },
}

/// One outgoing layer of a crossfade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Outgoing {
    pub voice: VoiceId,
    pub departure: Departure,
}

/// The clock of one dissolve: starts on the incoming content's first visible
/// frame and pauses with the Fade Cue that drives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DissolveClock {
    revealed: bool,
    paused: bool,
    /// When the current running stretch began (`None` = not running).
    running_since: Option<Instant>,
    /// Time accumulated by the stretches before the current one.
    banked: Duration,
    /// When the dissolve first started — the instant the Fade Cue anchors its
    /// own clock on, so picture and sound move together.
    started_at: Option<Instant>,
}

impl DissolveClock {
    fn new() -> Self {
        Self { revealed: false, paused: false, running_since: None, banked: Duration::ZERO, started_at: None }
    }

    /// The incoming content shows its first frame.
    pub(super) fn reveal(&mut self, now: Instant) {
        if self.revealed {
            return;
        }
        self.revealed = true;
        if !self.paused {
            self.run(now);
        }
    }

    pub(super) fn pause(&mut self, now: Instant) {
        if self.paused {
            return;
        }
        self.paused = true;
        if let Some(since) = self.running_since.take() {
            self.banked += now.saturating_duration_since(since);
        }
    }

    pub(super) fn resume(&mut self, now: Instant) {
        if !self.paused {
            return;
        }
        self.paused = false;
        if self.revealed {
            self.run(now);
        }
    }

    fn run(&mut self, now: Instant) {
        self.running_since = Some(now);
        self.started_at.get_or_insert(now);
    }

    /// Time the dissolve has been running.
    pub(super) fn elapsed(&self, now: Instant) -> Duration {
        self.banked
            + self.running_since.map(|since| now.saturating_duration_since(since)).unwrap_or_default()
    }

    pub(super) fn is_running(&self) -> bool {
        self.running_since.is_some()
    }

    /// The incoming content has shown its first frame (the dissolve may still
    /// be held by a pause).
    pub(super) fn is_revealed(&self) -> bool {
        self.revealed
    }
}

/// A crossfade, kept on the **incoming** content's slot.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CrossfadeLink {
    pub outgoing: Vec<Outgoing>,
    pub duration_ms: u32,
    /// Shape of the incoming picture's rise (the Fade's rising curve).
    pub up: FadeCurve,
    /// Shape of an outgoing picture's fall on another output.
    pub down: FadeCurve,
    pub clock: DissolveClock,
}

impl CrossfadeLink {
    pub(super) fn new(outgoing: Vec<Outgoing>, duration_ms: u32, up: FadeCurve, down: FadeCurve) -> Self {
        Self { outgoing, duration_ms, up, down, clock: DissolveClock::new() }
    }

    /// Linear progress, `0 → 1`.
    pub(super) fn progress(&self, now: Instant) -> f32 {
        if self.duration_ms == 0 {
            return if self.clock.revealed && self.clock.started_at.is_some() { 1.0 } else { 0.0 };
        }
        let elapsed = self.clock.elapsed(now).as_secs_f64() * 1000.0;
        (elapsed / self.duration_ms as f64).clamp(0.0, 1.0) as f32
    }

    /// How much of the picture belongs to the incoming content.
    pub(super) fn incoming_weight(&self, now: Instant) -> f32 {
        self.up.apply(self.progress(now) as f64) as f32
    }

    /// Opacity factor of an outgoing layer that fades out on another output.
    pub(super) fn outgoing_factor(&self, now: Instant) -> f32 {
        1.0 - self.down.apply(self.progress(now) as f64) as f32
    }

    pub(super) fn has_landed(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    /// `true` when at least one outgoing layer shares the incoming output.
    pub(super) fn dissolves_on_its_output(&self) -> bool {
        self.outgoing.iter().any(|o| o.departure == Departure::Mixed)
    }

    /// Still needs frames: started, unpaused, not landed.
    pub(super) fn is_animating(&self, now: Instant) -> bool {
        self.clock.is_running() && !self.has_landed(now)
    }

    pub(super) fn phase(&self) -> CrossfadePhase {
        match self.clock.started_at {
            Some(started_at) => CrossfadePhase::Started { started_at },
            None => CrossfadePhase::Waiting,
        }
    }
}

// ---------------------------------------------------------------------------
// Compositing plan
// ---------------------------------------------------------------------------

/// One dissolve to render on an output, in that output's slot indices.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MixLink {
    pub incoming: usize,
    pub incoming_key: u64,
    /// Outgoing layers on this output: `(slot index, layer key)`.
    pub outgoing: Vec<(usize, u64)>,
    /// Share of the picture that belongs to the incoming layer, `0..=1`.
    pub weight: f32,
}

/// One composite of the layer stack with some layers left out, and its share
/// of the final picture.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StackPass {
    /// Slot indices left out of this composite (sorted, unique).
    pub excluded: Vec<usize>,
    pub weight: f32,
}

/// How to render one output's dissolves this frame.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DissolvePlan {
    /// The composites whose weighted sum is the picture.  One pass with no
    /// exclusion when nothing dissolves.
    pub passes: Vec<StackPass>,
    /// Dissolves beyond [`MAX_EXACT_DISSOLVES`], approximated by animating the
    /// upper layers only: `(slot index, opacity factor)`.  Exact as long as
    /// the lower content is opaque.
    pub approximated: Vec<(usize, f32)>,
}

impl DissolvePlan {
    /// Nothing to mix: a single plain composite.
    pub(super) fn is_plain(&self) -> bool {
        self.approximated.is_empty() && self.passes.len() == 1 && self.passes[0].excluded.is_empty()
    }
}

/// Expand an output's dissolves into weighted composites.
///
/// A dissolve at rest is folded into every pass for free (`w = 0` leaves its
/// incoming layer out, `w = 1` its outgoing ones).  Dissolves in flight
/// branch: each composite takes one side of every dissolve and is weighted by
/// the product of those sides' weights, so the weights always sum to 1.
pub(super) fn plan_dissolves(links: &[MixLink]) -> DissolvePlan {
    let mut always_excluded: Vec<usize> = Vec::new();
    let mut in_flight: Vec<&MixLink> = Vec::new();
    let mut approximated: Vec<(usize, f32)> = Vec::new();

    for link in links {
        let w = link.weight.clamp(0.0, 1.0);
        if w <= AT_REST {
            always_excluded.push(link.incoming);
        } else if w >= 1.0 - AT_REST {
            always_excluded.extend(link.outgoing.iter().map(|&(slot, _)| slot));
        } else if in_flight.len() < MAX_EXACT_DISSOLVES {
            in_flight.push(link);
        } else {
            approximated.extend(approximate(link, w));
        }
    }

    let mut passes: Vec<StackPass> = Vec::with_capacity(1 << in_flight.len());
    for sides in 0..(1u32 << in_flight.len()) {
        let mut weight = 1.0_f32;
        let mut excluded = always_excluded.clone();
        for (bit, link) in in_flight.iter().enumerate() {
            let w = link.weight.clamp(0.0, 1.0);
            if sides & (1 << bit) != 0 {
                weight *= w;
                excluded.extend(link.outgoing.iter().map(|&(slot, _)| slot));
            } else {
                weight *= 1.0 - w;
                excluded.push(link.incoming);
            }
        }
        excluded.sort_unstable();
        excluded.dedup();
        passes.push(StackPass { excluded, weight });
    }

    DissolvePlan { passes, approximated }
}

/// "Upper layers only" for a dissolve the exact budget cannot afford: the
/// incoming layer fades in over the outgoing layers below it, the outgoing
/// layers above it fade out over it.
fn approximate(link: &MixLink, w: f32) -> Vec<(usize, f32)> {
    let mut factors = Vec::with_capacity(link.outgoing.len() + 1);
    let mut incoming_factor = 1.0;
    for &(slot, key) in &link.outgoing {
        if key > link.incoming_key {
            factors.push((slot, 1.0 - w));
        } else {
            incoming_factor = w;
        }
    }
    factors.push((link.incoming, incoming_factor));
    factors
}

/// Running weighted average of composites: the factor that folds a new
/// composite of weight `weight` into an accumulation that already holds
/// `accumulated` (`mix(accumulation, composite, factor)`).
pub(super) fn fold_factor(accumulated: f32, weight: f32) -> f32 {
    let total = accumulated + weight;
    if total <= 0.0 { 0.0 } else { weight / total }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::output_engine::blend::{composite_pixel, BlendMode};

    fn link(incoming: usize, outgoing: &[usize], weight: f32) -> MixLink {
        MixLink {
            incoming,
            incoming_key: 100 + incoming as u64,
            outgoing: outgoing.iter().map(|&s| (s, 100 + s as u64)).collect(),
            weight,
        }
    }

    fn total_weight(plan: &DissolvePlan) -> f32 {
        plan.passes.iter().map(|p| p.weight).sum()
    }

    // ── Clock ────────────────────────────────────────────────────────────────

    #[test]
    fn the_clock_waits_for_the_first_frame() {
        let t0 = Instant::now();
        let link = CrossfadeLink::new(Vec::new(), 1000, FadeCurve::Linear, FadeCurve::Linear);
        assert_eq!(link.progress(t0 + Duration::from_secs(5)), 0.0);
        assert_eq!(link.phase(), CrossfadePhase::Waiting);
    }

    #[test]
    fn the_clock_runs_from_the_reveal() {
        let t0 = Instant::now();
        let mut link = CrossfadeLink::new(Vec::new(), 1000, FadeCurve::Linear, FadeCurve::Linear);
        link.clock.reveal(t0);
        assert!((link.progress(t0 + Duration::from_millis(250)) - 0.25).abs() < 1e-3);
        assert_eq!(link.phase(), CrossfadePhase::Started { started_at: t0 });
        assert!(link.has_landed(t0 + Duration::from_millis(1000)));
    }

    #[test]
    fn a_paused_dissolve_holds_its_progress() {
        let t0 = Instant::now();
        let mut link = CrossfadeLink::new(Vec::new(), 1000, FadeCurve::Linear, FadeCurve::Linear);
        link.clock.reveal(t0);
        link.clock.pause(t0 + Duration::from_millis(400));
        let held = link.progress(t0 + Duration::from_secs(10));
        assert!((held - 0.4).abs() < 1e-3, "got {held}");
        assert!(!link.is_animating(t0 + Duration::from_secs(10)));
        link.clock.resume(t0 + Duration::from_secs(10));
        let after = link.progress(t0 + Duration::from_millis(10_100));
        assert!((after - 0.5).abs() < 1e-3, "resumes where it stopped, got {after}");
    }

    #[test]
    fn a_reveal_during_a_pause_starts_on_resume() {
        let t0 = Instant::now();
        let mut link = CrossfadeLink::new(Vec::new(), 1000, FadeCurve::Linear, FadeCurve::Linear);
        link.clock.pause(t0);
        link.clock.reveal(t0 + Duration::from_millis(100));
        assert_eq!(link.phase(), CrossfadePhase::Waiting, "paused: the dissolve has not started");
        let resume = t0 + Duration::from_millis(900);
        link.clock.resume(resume);
        assert_eq!(link.phase(), CrossfadePhase::Started { started_at: resume });
        assert_eq!(link.progress(resume), 0.0);
    }

    #[test]
    fn a_zero_length_dissolve_lands_on_its_first_frame() {
        let t0 = Instant::now();
        let mut link = CrossfadeLink::new(Vec::new(), 0, FadeCurve::Linear, FadeCurve::Linear);
        assert_eq!(link.incoming_weight(t0), 0.0);
        link.clock.reveal(t0);
        assert_eq!(link.incoming_weight(t0), 1.0);
    }

    #[test]
    fn the_curves_shape_the_picture() {
        let t0 = Instant::now();
        let mut link = CrossfadeLink::new(Vec::new(), 1000, FadeCurve::SCurve, FadeCurve::Exponential);
        link.clock.reveal(t0);
        let quarter = t0 + Duration::from_millis(250);
        let s = FadeCurve::SCurve.apply(0.25) as f32;
        assert!((link.incoming_weight(quarter) - s).abs() < 1e-4, "the rising curve drives the incoming picture");
        let e = FadeCurve::Exponential.apply(0.25) as f32;
        assert!((link.outgoing_factor(quarter) - (1.0 - e)).abs() < 1e-4, "the falling curve drives a faded one");
    }

    #[test]
    fn only_an_outgoing_layer_on_the_same_output_is_mixed() {
        let voice = uuid::Uuid::new_v4();
        let elsewhere = CrossfadeLink::new(
            vec![Outgoing { voice, departure: Departure::Faded { from: 1.0 } }],
            500,
            FadeCurve::Linear,
            FadeCurve::Linear,
        );
        assert!(!elsewhere.dissolves_on_its_output());
        let here = CrossfadeLink::new(
            vec![Outgoing { voice, departure: Departure::Mixed }],
            500,
            FadeCurve::Linear,
            FadeCurve::Linear,
        );
        assert!(here.dissolves_on_its_output());
    }

    // ── Plan ─────────────────────────────────────────────────────────────────

    #[test]
    fn nothing_to_dissolve_is_one_plain_composite() {
        let plan = plan_dissolves(&[]);
        assert!(plan.is_plain());
        assert_eq!(plan.passes, vec![StackPass { excluded: vec![], weight: 1.0 }]);
    }

    #[test]
    fn a_dissolve_in_flight_mixes_the_two_pictures() {
        let plan = plan_dissolves(&[link(1, &[0], 0.25)]);
        assert_eq!(
            plan.passes,
            vec![
                StackPass { excluded: vec![1], weight: 0.75 },
                StackPass { excluded: vec![0], weight: 0.25 },
            ],
        );
    }

    #[test]
    fn a_dissolve_at_rest_costs_no_extra_composite() {
        let before = plan_dissolves(&[link(1, &[0], 0.0)]);
        assert_eq!(before.passes, vec![StackPass { excluded: vec![1], weight: 1.0 }]);
        let landed = plan_dissolves(&[link(1, &[0, 2], 1.0)]);
        assert_eq!(landed.passes, vec![StackPass { excluded: vec![0, 2], weight: 1.0 }]);
    }

    #[test]
    fn two_dissolves_in_flight_branch_into_four_weighted_composites() {
        let plan = plan_dissolves(&[link(1, &[0], 0.5), link(3, &[2], 0.25)]);
        assert_eq!(plan.passes.len(), 4);
        assert!((total_weight(&plan) - 1.0).abs() < 1e-6);
        let both_in = plan.passes.iter().find(|p| p.excluded == vec![0, 2]).unwrap();
        assert!((both_in.weight - 0.125).abs() < 1e-6);
    }

    #[test]
    fn chained_dissolves_keep_their_layers_apart() {
        // A→B still dissolving while B→C starts: B is incoming of one and
        // outgoing of the other.
        let plan = plan_dissolves(&[link(1, &[0], 0.5), link(2, &[1], 0.5)]);
        let excluded: Vec<Vec<usize>> = plan.passes.iter().map(|p| p.excluded.clone()).collect();
        assert!(excluded.contains(&vec![1, 2]), "A alone");
        assert!(excluded.contains(&vec![0, 2]), "B alone");
        assert!(excluded.contains(&vec![1]), "C over A");
        assert!(excluded.contains(&vec![0, 1]), "C alone");
    }

    #[test]
    fn a_third_dissolve_in_flight_is_approximated() {
        let plan = plan_dissolves(&[link(1, &[0], 0.5), link(3, &[2], 0.5), link(5, &[4], 0.3)]);
        assert_eq!(plan.passes.len(), 4, "the exact budget is two dissolves");
        assert_eq!(plan.approximated, vec![(5, 0.3)], "the incoming layer fades in over the lower outgoing one");
        assert!((total_weight(&plan) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_approximated_dissolve_fades_out_an_outgoing_layer_above() {
        let mut above = link(1, &[0], 0.3);
        above.outgoing = vec![(0, 999)];
        assert_eq!(approximate(&above, 0.3), vec![(0, 0.7), (1, 1.0)]);
    }

    #[test]
    fn folding_composites_is_a_weighted_average() {
        // Three composites of value 0, 1, 1 weighted 0.5, 0.25, 0.25 → 0.5.
        let mut value = 0.0_f32;
        let mut accumulated = 0.5_f32;
        for (composite, weight) in [(1.0_f32, 0.25_f32), (1.0, 0.25)] {
            let f = fold_factor(accumulated, weight);
            value = value + (composite - value) * f;
            accumulated += weight;
        }
        assert!((value - 0.5).abs() < 1e-6);
    }

    // ── The picture, on the CPU ──────────────────────────────────────────────

    /// Composite a stack of straight-alpha pixels over the opaque black stage,
    /// leaving out `excluded`.
    fn stack(layers: &[[f32; 4]], excluded: &[usize]) -> [f32; 4] {
        let mut out = [0.0, 0.0, 0.0, 1.0];
        for (index, layer) in layers.iter().enumerate() {
            if !excluded.contains(&index) {
                out = composite_pixel(BlendMode::Normal, out, *layer, 1.0);
            }
        }
        out
    }

    fn render(layers: &[[f32; 4]], plan: &DissolvePlan) -> f32 {
        plan.passes.iter().map(|p| stack(layers, &p.excluded)[0] * p.weight).sum()
    }

    #[test]
    fn a_letterboxed_picture_dissolves_in_its_bars_too() {
        // In the bars of a letterboxed incoming picture (transparent pixels),
        // the outgoing picture must fade to black with the dissolve — not
        // stand at full until the end and then cut.
        let outgoing = [1.0, 1.0, 1.0, 1.0];
        let incoming_bar = [0.0, 0.0, 0.0, 0.0];
        let plan = plan_dissolves(&[link(1, &[0], 0.25)]);
        let pixel = render(&[outgoing, incoming_bar], &plan);
        assert!((pixel - 0.75).abs() < 1e-5, "true dissolve: 75 % of the old picture, got {pixel}");
    }

    #[test]
    fn two_opaque_pictures_never_dip_to_black() {
        let outgoing = [0.8, 0.8, 0.8, 1.0];
        let incoming = [0.8, 0.8, 0.8, 1.0];
        for step in 0..=10 {
            let plan = plan_dissolves(&[link(1, &[0], step as f32 / 10.0)]);
            let pixel = render(&[outgoing, incoming], &plan);
            assert!((pixel - 0.8).abs() < 1e-5, "step {step}: {pixel}");
        }
    }

    #[test]
    fn a_layer_on_top_of_the_dissolve_is_untouched() {
        // A logo above both pictures stays exactly where it was.
        let logo = [0.2, 0.2, 0.2, 1.0];
        let plan = plan_dissolves(&[link(1, &[0], 0.5)]);
        let pixel = render(&[[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0], logo], &plan);
        assert!((pixel - 0.2).abs() < 1e-5);
    }
}
