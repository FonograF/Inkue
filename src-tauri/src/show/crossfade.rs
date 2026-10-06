//! What a Fade Cue drives — and the crossfade it may dissolve its targets into.
//!
//! The transport calls [`on_fade_go`] right after it GOes a Fade Cue; the event
//! loop calls [`advance_pending`] every tick to finish the crossfades that had
//! to wait — for the Fade's own pre-wait, or for the incoming cue to show a
//! picture (its pre-wait, a camera that is still opening).
//!
//! A **plain fade** gets its targets' voices: audio voices with their current
//! gain and pan, visual layers with their current opacity.
//!
//! A **crossfade** (`crossfade_into`) starts the incoming visual cue and, once
//! it has a picture, hands the output engine the targets' layers to dissolve
//! away (`engine/output_engine/crossfade.rs`).  The incoming sound starts silent
//! and rises with the dissolve; the outgoing targets' sound falls to silence;
//! the Fade stops them when it lands.  Targets with no picture (an audio cue)
//! are faded the plain way, on the same clock.  When there is nothing to
//! dissolve into — the cue is missing, not visual, already on screen, or fails
//! to start — the Fade falls back to a plain fade of all its targets.

use crate::cue::context::CueContext;
use crate::cue::types::{CrossfadeLinked, CueId, FadeAction, FadeCrossfade};
use crate::engine::output_engine::CrossfadeRequest;
use crate::engine::ring_command::VoiceId;

use super::cue_list::CueList;

/// Set up what the Fade Cue `fade_id` drives, right after its GO.  Returns the
/// cues it started (a crossfade's incoming cue), for the caller to report as
/// triggered.
pub fn on_fade_go(cue_list: &mut CueList, ctx: &CueContext, fade_id: CueId) -> Vec<CueId> {
    let Some(spec) = cue_list.get(&fade_id).and_then(|c| c.fade_specification()) else {
        return Vec::new();
    };
    match spec.crossfade_into.filter(|&into| into != fade_id) {
        None => {
            set_plain_fade(cue_list, ctx, fade_id, &spec, &[]);
            Vec::new()
        }
        Some(incoming_cue) => {
            if let Some(fade) = cue_list.get_mut(&fade_id) {
                fade.set_crossfade(FadeCrossfade::Pending { incoming_cue, incoming_started: false });
            }
            advance(cue_list, ctx, fade_id)
        }
    }
}

/// Push every running Fade Cue whose crossfade is still being set up one step
/// further.  Returns the cues started on the way.
pub fn advance_pending(cue_list: &mut CueList, ctx: &CueContext) -> Vec<CueId> {
    let pending: Vec<CueId> = cue_list
        .cues
        .iter()
        .filter(|c| c.is_running() && c.pending_crossfade().is_some())
        .map(|c| c.id())
        .collect();
    pending.into_iter().flat_map(|fade_id| advance(cue_list, ctx, fade_id)).collect()
}

/// One step of setting up a crossfade: start the incoming cue, wait for its
/// picture, link the dissolve — or give up and fade the plain way.
fn advance(cue_list: &mut CueList, ctx: &CueContext, fade_id: CueId) -> Vec<CueId> {
    let Some(fade) = cue_list.get(&fade_id) else { return Vec::new() };
    // The Fade's own pre-wait comes first.
    if !fade.is_action_started() {
        return Vec::new();
    }
    let (Some((incoming_cue, incoming_started)), Some(spec)) =
        (fade.pending_crossfade(), fade.fade_specification())
    else {
        return Vec::new();
    };

    let mut started = Vec::new();
    match incoming_state(cue_list, incoming_cue) {
        Incoming::Unusable => return give_up(cue_list, ctx, fade_id, &spec),
        // Already on screen before this Fade: there is no entrance to dissolve.
        Incoming::Running if !incoming_started => return give_up(cue_list, ctx, fade_id, &spec),
        Incoming::Running => {}
        Incoming::Idle if incoming_started => {
            // It stopped (or failed) while this Fade was waiting for it.
            return give_up(cue_list, ctx, fade_id, &spec);
        }
        Incoming::Idle => {
            let Some(incoming) = cue_list.get_mut_recursive(&incoming_cue) else {
                return give_up(cue_list, ctx, fade_id, &spec);
            };
            let launched = incoming.go(ctx).is_ok() && incoming.is_running();
            if !launched {
                log::warn!("[crossfade] cue {incoming_cue} did not start — plain fade instead");
                return give_up(cue_list, ctx, fade_id, &spec);
            }
            started.push(incoming_cue);
            if let Some(fade) = cue_list.get_mut(&fade_id) {
                fade.set_crossfade(FadeCrossfade::Pending { incoming_cue, incoming_started: true });
            }
        }
    }

    // No picture yet (its pre-wait): try again on the next tick.
    let Some(incoming_voice) = cue_list.get_recursive(&incoming_cue).and_then(|c| c.playing_voice_id()) else {
        return started;
    };
    if !link(cue_list, ctx, fade_id, &spec, incoming_cue, incoming_voice) {
        give_up(cue_list, ctx, fade_id, &spec);
    }
    started
}

enum Incoming {
    /// Missing, or not a picture to dissolve into.
    Unusable,
    Idle,
    Running,
}

fn incoming_state(cue_list: &CueList, incoming_cue: CueId) -> Incoming {
    match cue_list.get_recursive(&incoming_cue) {
        Some(cue) if !cue.is_visual() => Incoming::Unusable,
        Some(cue) if cue.is_running() || cue.is_paused() => Incoming::Running,
        Some(_) => Incoming::Idle,
        None => Incoming::Unusable,
    }
}

/// Hand the dissolve to the output engine and the rest of the targets to the
/// Fade.  `false` when the engine has no picture for the incoming voice.
fn link(
    cue_list: &mut CueList,
    ctx: &CueContext,
    fade_id: CueId,
    spec: &FadeAction,
    incoming_cue: CueId,
    incoming_voice: VoiceId,
) -> bool {
    let mut dissolved: Vec<CueId> = Vec::new();
    let mut outgoing_voices: Vec<VoiceId> = Vec::new();
    let mut outgoing_audio: Vec<(VoiceId, f32)> = Vec::new();
    for &target_id in spec.target_cue_ids.iter().filter(|&&t| t != incoming_cue) {
        let Some(target) = cue_list.get_recursive(&target_id) else { continue };
        let pictures = target.visual_voice_ids();
        if pictures.is_empty() {
            continue;
        }
        dissolved.push(target_id);
        for voice in audio_voices_of(target, ctx, &pictures) {
            outgoing_audio.push((voice, ctx.audio_engine.get_voice_gain(voice)));
        }
        outgoing_voices.extend(pictures);
    }

    let request = CrossfadeRequest {
        incoming: incoming_voice,
        outgoing: outgoing_voices,
        duration_ms: u32::try_from(spec.duration_ms).unwrap_or(u32::MAX),
        up: spec.up_curve,
        down: spec.down_curve,
    };
    if !ctx.output_engine.link_crossfade(&request) {
        return false;
    }

    let incoming_audio = silence_incoming_sound(ctx, incoming_voice);
    let mut not_plain = dissolved.clone();
    not_plain.push(incoming_cue);
    set_plain_fade(cue_list, ctx, fade_id, spec, &not_plain);
    if let Some(fade) = cue_list.get_mut(&fade_id) {
        fade.set_crossfade(FadeCrossfade::Linked(CrossfadeLinked {
            incoming_cue,
            incoming_voice,
            incoming_audio,
            outgoing_audio,
            targets: dissolved,
        }));
    }
    true
}

/// Nothing to dissolve into: every target is faded the plain way, from now —
/// except the cue the crossfade would have brought in, never faded by it.
fn give_up(cue_list: &mut CueList, ctx: &CueContext, fade_id: CueId, spec: &FadeAction) -> Vec<CueId> {
    let incoming: Vec<CueId> = spec.crossfade_into.into_iter().collect();
    set_plain_fade(cue_list, ctx, fade_id, spec, &incoming);
    if let Some(fade) = cue_list.get_mut(&fade_id) {
        fade.set_crossfade(FadeCrossfade::None);
    }
    Vec::new()
}

/// Hand the Fade the voices of its targets (all but `skip`): audio voices with
/// their current gain and pan, visual layers with their current opacity.
fn set_plain_fade(cue_list: &mut CueList, ctx: &CueContext, fade_id: CueId, spec: &FadeAction, skip: &[CueId]) {
    let mut audio: Vec<(VoiceId, f32, f32)> = Vec::new();
    let mut visual: Vec<(VoiceId, f32)> = Vec::new();
    for target_id in spec.target_cue_ids.iter().filter(|t| !skip.contains(t)) {
        // Recursive lookup so a Fade can target a cue nested in a group.
        let Some(target) = cue_list.get_recursive(target_id) else { continue };
        let pictures = target.visual_voice_ids();
        for voice in audio_voices_of(target, ctx, &pictures) {
            let gain = ctx.audio_engine.get_voice_gain(voice);
            let pan = ctx.audio_engine.get_voice_pan(voice);
            audio.push((voice, gain, pan));
        }
        for voice in pictures {
            visual.push((voice, ctx.output_engine.get_voice_opacity(voice)));
        }
    }

    // Brightness % → layer opacity; without an explicit visual target the
    // audio gain doubles as the opacity target (legacy semantics).
    let visual_target_opacity = spec
        .target_visual_alpha
        .map(|alpha| 1.0 - alpha as f32 / 255.0)
        .unwrap_or_else(|| spec.target_gain_linear.clamp(0.0, 1.0));

    if let Some(fade) = cue_list.get_mut(&fade_id) {
        fade.set_fade_voices(audio, visual, visual_target_opacity);
    }
}

/// The audio voices a target owns: its own (recursively, for a group) and the
/// sound track of each of its videos.  `pictures` are its visual voices, which
/// `all_voice_ids` also reports but which carry no sound themselves.
fn audio_voices_of(target: &dyn crate::cue::traits::Cue, ctx: &CueContext, pictures: &[VoiceId]) -> Vec<VoiceId> {
    let mut voices: Vec<VoiceId> =
        target.all_voice_ids().into_iter().filter(|v| !pictures.contains(v)).collect();
    voices.extend(pictures.iter().filter_map(|&picture| ctx.output_engine.video_audio_voice(picture)));
    voices
}

/// Mute the incoming video's sound track and return it with the level the
/// fade must bring it back to.  Images and cameras have none.
fn silence_incoming_sound(ctx: &CueContext, incoming_voice: VoiceId) -> Vec<(VoiceId, f32)> {
    let Some(audio) = ctx.output_engine.video_audio_voice(incoming_voice) else {
        return Vec::new();
    };
    let level = ctx.audio_engine.get_voice_gain(audio);
    let _ = ctx.audio_engine.set_voice_gain(audio, 0.0);
    vec![(audio, level)]
}
