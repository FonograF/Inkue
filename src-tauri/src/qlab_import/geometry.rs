//! QLab Fade cue geometry (the Geometry tab, Curve and Path modes) → the
//! Inkue Fade's `geometry`, `fade_opacity` and `target_brightness_pct`.
//!
//! Every key read here was decoded from real QLab 5 workspaces (build 5603),
//! one setting changed per file, and the result checked on screen in Inkue.
//! Unknown so far: how QLab stores a rotation fade, the codes for scale and
//! X/Y rotation as path axes, and what a `pathWidth` other than the stage
//! size does.  The rules that matter:
//!
//! - `fadeType` decides the mode (`1` Curve, `2` Path) and must be read
//!   first: QLab keeps the Curve-mode values in a Path fade, and Path mode
//!   ignores them (and every `do*` checkbox).
//! - QLab geometry is in **stage pixels, +Y up**; Inkue's is fractions of the
//!   output, +Y up — so values are divided by the target's stage size.
//! - Path points are relative to `pathOriginPosition`: QLab never rewrites
//!   them when the origin changes, it reads the same numbers from a new corner.
//!
//! Anything Inkue cannot reproduce yet (rotation, non-uniform scale, a path
//! driving rotation or scale) is left out and named in the cue's notes —
//! never silently dropped.

use serde_json::{json, Value};

use super::outputs::VideoOutputs;

/// Stage size when the workspace declares none (QLab 4 surfaces).
const FALLBACK_STAGE: (f64, f64) = (1920.0, 1080.0);

/// QLab `fadeType` of a Path-mode fade.
const FADE_TYPE_PATH: i64 = 2;

/// What a QLab Fade does to its pictures, in Inkue terms.
#[derive(Debug, Default, PartialEq)]
pub struct FadeGeometry {
    /// The Inkue `geometry` value, or `None` when nothing converts.
    pub geometry: Option<Value>,
    /// `fade_opacity`, when the QLab file says (`None` = keep Inkue's default).
    pub fade_opacity: Option<bool>,
    /// `target_brightness_pct` for an opacity fade.
    pub brightness_pct: Option<f64>,
    /// Notes about what could not be converted.
    pub notes: Vec<String>,
}

/// Map a QLab `FadeCue`'s geometry.
pub fn map(cue: &Value, outputs: &VideoOutputs) -> FadeGeometry {
    // The target (resolved by the archive) names the stage its picture is on.
    let target = cue.get("cueTarget").unwrap_or(&Value::Null);
    let stage = outputs.stage_size_of(target);
    let fade_type = cue
        .get("fadeType")
        .or_else(|| cue.pointer("/fade/fadeType"))
        .and_then(Value::as_i64);
    if fade_type == Some(FADE_TYPE_PATH) {
        path(cue, stage)
    } else {
        curve(cue, stage.unwrap_or(FALLBACK_STAGE))
    }
}

fn flag(cue: &Value, key: &str) -> bool {
    cue.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number(cue: &Value, key: &str) -> Option<f64> {
    cue.get(key).and_then(Value::as_f64)
}

// ---------------------------------------------------------------------------
// Curve mode
// ---------------------------------------------------------------------------

fn curve(cue: &Value, (width, height): (f64, f64)) -> FadeGeometry {
    let mut out = FadeGeometry::default();

    let translation = flag(cue, "doTranslation").then(|| {
        let x = number(cue, "translationX").unwrap_or(0.0);
        let y = number(cue, "translationY").unwrap_or(0.0);
        [x / width, y / height]
    });

    let scale = if flag(cue, "doScale") {
        let x = number(cue, "scaleX").unwrap_or(1.0);
        let y = number(cue, "scaleY").unwrap_or(x);
        if (x - y).abs() < 1e-9 {
            Some(x)
        } else {
            out.notes.push(format!(
                "[QLab scale fade to X {x} × Y {y} not converted — Inkue scales both axes together]"
            ));
            None
        }
    } else {
        None
    };

    if flag(cue, "doRotation") {
        out.notes.push("[QLab rotation fade not converted — rebuild it in the Geometry section]".into());
    }

    // QLab 4 files may not carry the Opacity box at all: keep the default then.
    if cue.get("doOpacity").is_some() {
        let fades = flag(cue, "doOpacity");
        out.fade_opacity = Some(fades);
        if fades {
            out.brightness_pct = Some(number(cue, "opacity").unwrap_or(1.0).clamp(0.0, 1.0) * 100.0);
        }
    }

    if translation.is_some() || scale.is_some() {
        out.geometry = Some(json!({
            "mode": "curve",
            "translation": translation,
            "scale": scale,
            "rotation_deg": Value::Null,
        }));
    }
    out
}

// ---------------------------------------------------------------------------
// Path mode
// ---------------------------------------------------------------------------

/// A QLab path parameter code → Inkue's, with the stage dimension that turns
/// its pixels into a fraction.
fn path_param(code: &str, (width, height): (f64, f64)) -> Option<(&'static str, f64)> {
    match code {
        "translationX" => Some(("translation_x", width)),
        "translationY" => Some(("translation_y", height)),
        _ => None,
    }
}

/// `"{412.09, -143.63}"` (an archived NSPoint) → `[x, y]`.
fn parse_point(text: &str) -> Option<[f64; 2]> {
    let inner = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (x, y) = inner.split_once(',')?;
    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
}

/// Where `pathOriginPosition`'s (0, 0) sits, from the canvas centre, in
/// canvas pixels (+Y up).  Center (1) and Bottom Right (5) are confirmed;
/// 2–4 follow the menu order (Top Left, Top Right, Bottom Left).
fn origin_offset(code: i64, (width, height): (f64, f64)) -> Option<[f64; 2]> {
    let (half_w, half_h) = (width / 2.0, height / 2.0);
    match code {
        1 => Some([0.0, 0.0]),
        2 => Some([-half_w, half_h]),
        3 => Some([half_w, half_h]),
        4 => Some([-half_w, -half_h]),
        5 => Some([half_w, -half_h]),
        _ => None,
    }
}

/// `pathLoop` is written as a bool or as an integer, depending on how the
/// box was last touched.
fn loops(cue: &Value) -> bool {
    match cue.get("pathLoop") {
        Some(Value::Bool(b)) => *b,
        Some(v) => v.as_i64().is_some_and(|n| n != 0),
        None => false,
    }
}

fn path(cue: &Value, stage: Option<(f64, f64)>) -> FadeGeometry {
    // Path mode moves geometry only: whatever the Curve-mode Opacity box was
    // left at does not apply.
    let mut out = FadeGeometry { fade_opacity: Some(false), ..FadeGeometry::default() };

    let canvas = number(cue, "pathWidth")
        .zip(number(cue, "pathHeight"))
        .filter(|&(w, h)| w > 0.0 && h > 0.0)
        .or(stage)
        .unwrap_or(FALLBACK_STAGE);
    let stage = stage.unwrap_or(canvas);

    // Which property each axis drives: `{ "translationX": 1, "translationY": 2 }`.
    let map = cue.pointer("/pathParameterMap/geometry").and_then(Value::as_object);
    let axis = |n: i64, default: &'static str| -> String {
        map.and_then(|m| m.iter().find(|(_, v)| v.as_i64() == Some(n)).map(|(k, _)| k.clone()))
            .unwrap_or_else(|| default.to_string())
    };
    let (x_code, y_code) = (axis(1, "translationX"), axis(2, "translationY"));
    let (Some((x_param, x_size)), Some((y_param, y_size))) =
        (path_param(&x_code, stage), path_param(&y_code, stage))
    else {
        out.notes.push(format!(
            "[QLab path fade not converted — it drives {x_code} and {y_code}; Inkue paths move position only]"
        ));
        return out;
    };

    let origin_code = number(cue, "pathOriginPosition").map_or(1, |n| n as i64);
    let origin = origin_offset(origin_code, canvas).unwrap_or_else(|| {
        out.notes.push(format!("[QLab path origin {origin_code} unknown — read as Center]"));
        [0.0, 0.0]
    });

    let raw: Vec<&Value> = cue
        .pointer("/fade/path/points")
        .and_then(Value::as_array)
        .map(|points| points.iter().collect())
        .unwrap_or_default();
    let points: Vec<[f64; 2]> = raw
        .iter()
        .filter_map(|p| p.as_str().and_then(parse_point))
        .map(|[x, y]| [(x + origin[0]) / x_size, (y + origin[1]) / y_size])
        .collect();
    if points.len() < raw.len() {
        out.notes.push(format!("[{} QLab path point(s) unreadable and left out]", raw.len() - points.len()));
    }
    if points.len() < 2 {
        out.notes.push("[QLab path fade has fewer than two points — not converted]".into());
        return out;
    }

    let merge = cue
        .pointer("/fade/path/easeInFraction")
        .and_then(Value::as_f64)
        .or_else(|| number(cue, "pathEaseInFraction"))
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    out.geometry = Some(json!({
        "mode": "path",
        "points": points,
        "smooth": cue.pointer("/fade/path/smoothPath").and_then(Value::as_bool).unwrap_or(false),
        "merge": merge,
        "loop_path": loops(cue),
        "x_param": x_param,
        "y_param": y_param,
    }));
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2880 × 1800 stage, like the test workspaces.
    fn outputs() -> VideoOutputs {
        VideoOutputs::from_settings(&json!({ "Video": { "stages": [{
            "__class__": "VideoStage", "name": "Stage 1", "width": 2880, "height": 1800,
            "uniqueID": "CDCD7077-5C6C-4DF4-B60E-4E21024CF57C",
            "viewport": { "uniqueID": "031791CF-6819-49E6-9C52-84164CF20194" },
        }] } }))
    }

    /// A QLab Path fade as decoded from `path.qlab5`, with leftover
    /// Curve-mode values that Path mode must ignore.
    fn path_fade() -> Value {
        json!({
            "__class__": "FadeCue",
            "fadeType": 2,
            "cueTarget": { "__class__": "VideoCue", "viewportID": "031791CF-6819-49E6-9C52-84164CF20194" },
            "doTranslation": true, "translationX": -717.7, "translationY": 395.9,
            "doOpacity": true, "opacity": 0.2,
            "pathWidth": 2880.0, "pathHeight": 1800.0, "pathOriginPosition": 1,
            "pathLoop": false, "pathEaseInFraction": 0.0,
            "pathParameterMap": { "geometry": { "translationX": 1, "translationY": 2 } },
            "fade": { "fadeType": 2, "path": {
                "__class__": "NetworkPath",
                "points": ["{1440, 0}", "{-720, -450}", "{0, 900}"],
                "smoothPath": false, "easeInFraction": 0.0,
            } },
        })
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ── Curve mode ──────────────────────────────────────────────────────────

    #[test]
    fn a_curve_translation_becomes_a_fraction_of_the_stage() {
        let cue = json!({
            "fadeType": 1, "doTranslation": true, "translationX": -720.0, "translationY": 450.0,
            "doOpacity": false,
        });
        let out = map(&cue, &outputs());
        let geometry = out.geometry.expect("a translation fade converts");
        assert_eq!(geometry["mode"], "curve");
        assert_eq!(geometry["translation"], json!([-0.25, 0.25]), "stage pixels, +Y up, kept as +Y up");
        assert_eq!((geometry["scale"].clone(), geometry["rotation_deg"].clone()), (Value::Null, Value::Null));
        assert_eq!(out.fade_opacity, Some(false), "the Opacity box was off");
    }

    #[test]
    fn unchecked_boxes_convert_nothing_even_with_values_set() {
        let cue = json!({ "fadeType": 1, "doTranslation": false, "translationX": 500.0, "doScale": false, "scaleX": 2.0 });
        assert_eq!(map(&cue, &outputs()).geometry, None);
    }

    #[test]
    fn a_uniform_scale_converts_and_a_stretch_is_named() {
        let uniform = json!({ "fadeType": 1, "doScale": true, "scaleX": 1.5, "scaleY": 1.5 });
        let out = map(&uniform, &outputs());
        assert_eq!(out.geometry.unwrap()["scale"], 1.5);
        assert!(out.notes.is_empty());

        let stretch = json!({ "fadeType": 1, "doScale": true, "scaleX": 2.0, "scaleY": 0.5 });
        let out = map(&stretch, &outputs());
        assert_eq!(out.geometry, None);
        assert!(out.notes[0].contains("scales both axes together"), "{:?}", out.notes);
    }

    #[test]
    fn a_rotation_fade_is_named_not_guessed() {
        let cue = json!({ "fadeType": 1, "doRotation": true, "rotation": 90.0, "rotationType": 0 });
        let out = map(&cue, &outputs());
        assert_eq!(out.geometry, None);
        assert!(out.notes[0].contains("rotation fade not converted"), "{:?}", out.notes);
    }

    #[test]
    fn an_opacity_fade_sets_the_brightness() {
        let cue = json!({ "fadeType": 1, "doOpacity": true, "opacity": 0.25 });
        let out = map(&cue, &outputs());
        assert_eq!((out.fade_opacity, out.brightness_pct), (Some(true), Some(25.0)));
    }

    #[test]
    fn a_file_without_the_opacity_box_keeps_the_default() {
        let out = map(&json!({ "fadeType": 1 }), &outputs());
        assert_eq!((out.fade_opacity, out.brightness_pct), (None, None));
    }

    #[test]
    fn without_a_stage_size_the_fallback_stage_is_used() {
        let cue = json!({ "doTranslation": true, "translationX": 960.0, "translationY": 0.0 });
        let out = map(&cue, &VideoOutputs::default());
        assert_eq!(out.geometry.unwrap()["translation"], json!([0.5, 0.0]));
    }

    // ── Path mode ───────────────────────────────────────────────────────────

    #[test]
    fn a_path_converts_and_ignores_the_curve_mode_leftovers() {
        let out = map(&path_fade(), &outputs());
        let geometry = out.geometry.expect("a path fade converts");
        assert_eq!(geometry["mode"], "path");
        assert_eq!(geometry["points"], json!([[0.5, 0.0], [-0.25, -0.25], [0.0, 0.5]]));
        assert_eq!(geometry["translation"], Value::Null, "Curve-mode translation must not leak in");
        assert_eq!((geometry["smooth"].clone(), geometry["merge"].clone(), geometry["loop_path"].clone()),
                   (json!(false), json!(0.0), json!(false)));
        assert_eq!((geometry["x_param"].as_str(), geometry["y_param"].as_str()), (Some("translation_x"), Some("translation_y")));
        assert_eq!(out.fade_opacity, Some(false), "Path mode ignores the Opacity box");
        assert!(out.notes.is_empty(), "{:?}", out.notes);
    }

    #[test]
    fn smooth_merge_and_loop_come_through() {
        let mut cue = path_fade();
        cue["fade"]["path"]["smoothPath"] = json!(true);
        cue["fade"]["path"]["easeInFraction"] = json!(0.5);
        cue["pathLoop"] = json!(1);
        let geometry = map(&cue, &outputs()).geometry.unwrap();
        assert_eq!((geometry["smooth"].clone(), geometry["merge"].clone(), geometry["loop_path"].clone()),
                   (json!(true), json!(0.5), json!(true)));
    }

    #[test]
    fn path_loop_is_read_as_a_bool_or_an_integer() {
        for (value, expected) in [(json!(false), false), (json!(true), true), (json!(0), false), (json!(1), true)] {
            let mut cue = path_fade();
            cue["pathLoop"] = value.clone();
            assert_eq!(map(&cue, &outputs()).geometry.unwrap()["loop_path"], expected, "pathLoop = {value}");
        }
    }

    #[test]
    fn a_corner_origin_moves_the_points() {
        // Bottom Right (5): the same numbers, read from the bottom-right corner.
        let mut cue = path_fade();
        cue["pathOriginPosition"] = json!(5);
        cue["fade"]["path"]["points"] = json!(["{-1440, 900}", "{-2880, 0}"]);
        let geometry = map(&cue, &outputs()).geometry.unwrap();
        assert_eq!(geometry["points"], json!([[0.0, 0.0], [-0.5, -0.5]]));
    }

    #[test]
    fn every_origin_corner_lands_on_its_corner() {
        let corners = [(2, [-0.5, 0.5]), (3, [0.5, 0.5]), (4, [-0.5, -0.5]), (5, [0.5, -0.5])];
        for (code, corner) in corners {
            let mut cue = path_fade();
            cue["pathOriginPosition"] = json!(code);
            cue["fade"]["path"]["points"] = json!(["{0, 0}", "{0, 0}"]);
            let geometry = map(&cue, &outputs()).geometry.unwrap();
            let p = &geometry["points"][0];
            assert!(close(p[0].as_f64().unwrap(), corner[0]) && close(p[1].as_f64().unwrap(), corner[1]),
                    "origin {code}: {p}");
        }
    }

    #[test]
    fn swapped_axes_scale_by_the_property_they_drive() {
        let mut cue = path_fade();
        cue["pathParameterMap"] = json!({ "geometry": { "translationY": 1, "translationX": 2 } });
        cue["fade"]["path"]["points"] = json!(["{900, 1440}", "{0, 0}"]);
        let geometry = map(&cue, &outputs()).geometry.unwrap();
        // Horizontal axis drives Y (÷ 1800), vertical drives X (÷ 2880).
        assert_eq!(geometry["points"][0], json!([0.5, 0.5]));
        assert_eq!((geometry["x_param"].as_str(), geometry["y_param"].as_str()), (Some("translation_y"), Some("translation_x")));
    }

    #[test]
    fn a_path_driving_rotation_is_named_and_left_out() {
        // As in `custom mapping.qlab5`: Y axis → Rotation Z.
        let mut cue = path_fade();
        cue["pathParameterMap"] = json!({ "geometry": { "translationX": 1, "rotationZ": 2 } });
        let out = map(&cue, &outputs());
        assert_eq!(out.geometry, None);
        assert!(out.notes[0].contains("rotationZ"), "{:?}", out.notes);
    }

    #[test]
    fn unreadable_points_are_counted_and_too_few_points_do_not_convert() {
        let mut cue = path_fade();
        cue["fade"]["path"]["points"] = json!(["{0, 0}", "garbage", "{10, 10}"]);
        let out = map(&cue, &outputs());
        assert!(out.geometry.is_some());
        assert!(out.notes[0].contains("1 QLab path point"), "{:?}", out.notes);

        cue["fade"]["path"]["points"] = json!(["{0, 0}"]);
        let out = map(&cue, &outputs());
        assert_eq!(out.geometry, None);
        assert!(out.notes.iter().any(|n| n.contains("fewer than two points")), "{:?}", out.notes);
    }

    #[test]
    fn nspoint_strings_parse() {
        assert_eq!(parse_point("{412.0923913, -143.6311141}"), Some([412.0923913, -143.6311141]));
        assert_eq!(parse_point("{-192.28940217391278, -525}"), Some([-192.28940217391278, -525.0]));
        assert_eq!(parse_point("412, 10"), None);
    }
}
