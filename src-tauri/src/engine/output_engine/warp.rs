//! Corner-pin warp math for the global output transform.
//!
//! Pure math, no GL: computes the destination quad from an [`OutputTransform`]
//! (scale → fine rotation → pan → per-corner offsets, all in y-down normalized
//! window space) and the **inverse homography** the render thread's warp
//! shader uses to sample the mpv frame texture (inverse mapping: for each
//! window pixel, where in the source frame does it come from?).
//!
//! Matrices are row-major `[a b c; d e f; g h i]` flattened to `[f64; 9]`;
//! a point maps as `(x', y') = ((a·x + b·y + c) / w, (d·x + e·y + f) / w)`
//! with `w = g·x + h·y + i`.

use super::types::OutputTransform;

/// Corner order used throughout this module: TL, TR, BR, BL (y-down space).
/// Note [`OutputTransform::corners`] is stored as TL, TR, BL, BR (reading
/// order, matching the UI) — [`transformed_quad`] reorders.
pub(super) type Quad = [[f64; 2]; 4];

/// Destination quad in y-down normalized window space ([0,1]²), corners in
/// TL, TR, BR, BL order.
pub(super) fn transformed_quad(t: &OutputTransform) -> Quad {
    // Unit square corners, y-down: TL, TR, BR, BL.
    let base: Quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    // Storage order TL, TR, BL, BR → homography order TL, TR, BR, BL.
    let offsets = [t.corners[0], t.corners[1], t.corners[3], t.corners[2]];

    let scale = t.scale.max(0.01);
    let (sin, cos) = t.rotation.to_radians().sin_cos();

    let mut quad = [[0.0; 2]; 4];
    for (i, p) in base.iter().enumerate() {
        // Scale + rotate about the window centre.  In y-down coordinates this
        // rotation matrix turns the picture clockwise on screen.
        let x = (p[0] - 0.5) * scale;
        let y = (p[1] - 0.5) * scale;
        let xr = x * cos - y * sin;
        let yr = x * sin + y * cos;
        quad[i] = [
            xr + 0.5 + t.pan_x + offsets[i][0],
            yr + 0.5 + t.pan_y + offsets[i][1],
        ];
    }
    quad
}

/// Homography mapping the unit square (TL=(0,0) … BL=(0,1)) onto `quad`
/// (TL, TR, BR, BL).  Standard projective fit; falls back to the affine form
/// when the quad is a parallelogram.
pub(super) fn homography_unit_to_quad(quad: &Quad) -> [f64; 9] {
    let [p0, p1, p2, p3] = *quad; // TL, TR, BR, BL

    let sx = p0[0] - p1[0] + p2[0] - p3[0];
    let sy = p0[1] - p1[1] + p2[1] - p3[1];

    if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
        // Parallelogram — plain affine map.
        return [
            p1[0] - p0[0], p3[0] - p0[0], p0[0],
            p1[1] - p0[1], p3[1] - p0[1], p0[1],
            0.0, 0.0, 1.0,
        ];
    }

    let dx1 = p1[0] - p2[0];
    let dy1 = p1[1] - p2[1];
    let dx2 = p3[0] - p2[0];
    let dy2 = p3[1] - p2[1];
    let den = dx1 * dy2 - dy1 * dx2;
    if den.abs() < 1e-12 {
        // Degenerate (collinear edges) — identity keeps the shader sane.
        return IDENTITY;
    }
    let g = (sx * dy2 - sy * dx2) / den;
    let h = (dx1 * sy - dy1 * sx) / den;

    [
        p1[0] - p0[0] + g * p1[0], p3[0] - p0[0] + h * p3[0], p0[0],
        p1[1] - p0[1] + g * p1[1], p3[1] - p0[1] + h * p3[1], p0[1],
        g, h, 1.0,
    ]
}

const IDENTITY: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// Invert a row-major 3×3 matrix.  `None` when singular.
pub(super) fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    Some([
        (e * i - f * h) * inv_det, (c * h - b * i) * inv_det, (b * f - c * e) * inv_det,
        (f * g - d * i) * inv_det, (a * i - c * g) * inv_det, (c * d - a * f) * inv_det,
        (d * h - e * g) * inv_det, (b * g - a * h) * inv_det, (a * e - b * d) * inv_det,
    ])
}

/// Apply a row-major homography to a point.
#[cfg(test)]
pub(super) fn apply(m: &[f64; 9], p: [f64; 2]) -> [f64; 2] {
    let w = m[6] * p[0] + m[7] * p[1] + m[8];
    [
        (m[0] * p[0] + m[1] * p[1] + m[2]) / w,
        (m[3] * p[0] + m[4] * p[1] + m[5]) / w,
    ]
}

/// The inverse homography (window uv → frame uv, both y-down [0,1]) the warp
/// shader needs for this transform, as row-major f32.
///
/// `None` = no warp pass needed: either the transform is the identity, or the
/// quad is degenerate (singular homography) and warping would garble the
/// output — the frame then renders straight to the window as usual.
pub(super) fn warp_matrix(t: &OutputTransform) -> Option<[f32; 9]> {
    if t.is_identity() {
        return None;
    }
    let quad = transformed_quad(t);
    let h = homography_unit_to_quad(&quad);
    let inv = invert3(&h)?;
    let mut out = [0.0f32; 9];
    for (o, v) in out.iter_mut().zip(inv.iter()) {
        *o = *v as f32;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: Quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    fn assert_close(a: [f64; 2], b: [f64; 2]) {
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "{a:?} != {b:?}");
    }

    #[test]
    fn identity_transform_needs_no_warp() {
        assert_eq!(warp_matrix(&OutputTransform::default()), None);
    }

    #[test]
    fn quad_pure_pan_shifts_all_corners() {
        let t = OutputTransform { pan_x: 0.1, pan_y: -0.05, ..Default::default() };
        let q = transformed_quad(&t);
        assert_close(q[0], [0.1, -0.05]);
        assert_close(q[2], [1.1, 0.95]);
    }

    #[test]
    fn quad_scale_shrinks_about_centre() {
        let t = OutputTransform { scale: 0.5, ..Default::default() };
        let q = transformed_quad(&t);
        assert_close(q[0], [0.25, 0.25]);
        assert_close(q[2], [0.75, 0.75]);
    }

    #[test]
    fn quad_rotation_90_is_clockwise_on_screen() {
        let t = OutputTransform { rotation: 90.0, ..Default::default() };
        let q = transformed_quad(&t);
        // TL (−.5,−.5 about centre) → cw 90° in y-down space → (.5,−.5) → TR position.
        assert_close(q[0], [1.0, 0.0]);
        assert_close(q[1], [1.0, 1.0]);
    }

    #[test]
    fn quad_fine_rotation_is_supported() {
        let t = OutputTransform { rotation: 0.5, ..Default::default() };
        let q = transformed_quad(&t);
        // Half a degree moves the TL corner slightly but measurably.
        assert!(q[0][0] != 0.0 && (q[0][0]).abs() < 0.02);
    }

    #[test]
    fn quad_corner_offsets_apply_in_storage_order() {
        // Storage: TL, TR, BL, BR.  Only BL moved.
        let mut t = OutputTransform::default();
        t.corners[2] = [0.1, -0.1];
        let q = transformed_quad(&t);
        assert_close(q[3], [0.1, 0.9]); // BL is index 3 in TL,TR,BR,BL order
        assert_close(q[0], [0.0, 0.0]);
        assert_close(q[2], [1.0, 1.0]);
    }

    #[test]
    fn homography_maps_unit_corners_onto_quad() {
        let quad: Quad = [[0.1, 0.05], [0.9, 0.0], [1.0, 0.95], [0.0, 1.0]];
        let h = homography_unit_to_quad(&quad);
        assert_close(apply(&h, [0.0, 0.0]), quad[0]);
        assert_close(apply(&h, [1.0, 0.0]), quad[1]);
        assert_close(apply(&h, [1.0, 1.0]), quad[2]);
        assert_close(apply(&h, [0.0, 1.0]), quad[3]);
        // Centre stays strictly inside.
        let c = apply(&h, [0.5, 0.5]);
        assert!(c[0] > 0.0 && c[0] < 1.0 && c[1] > 0.0 && c[1] < 1.0);
    }

    #[test]
    fn homography_affine_branch_for_parallelogram() {
        let quad: Quad = [[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]];
        let h = homography_unit_to_quad(&quad);
        assert_eq!(h[6], 0.0);
        assert_eq!(h[7], 0.0);
        assert_close(apply(&h, [1.0, 1.0]), quad[2]);
    }

    #[test]
    fn inverse_round_trips_quad_corners_to_unit() {
        let quad: Quad = [[0.05, 0.1], [0.95, 0.02], [0.9, 0.9], [0.1, 0.85]];
        let h = homography_unit_to_quad(&quad);
        let inv = invert3(&h).expect("invertible");
        for (i, unit) in UNIT.iter().enumerate() {
            assert_close(apply(&inv, quad[i]), *unit);
        }
    }

    #[test]
    fn invert3_rejects_singular() {
        assert_eq!(invert3(&[1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 0.0, 0.0, 1.0]), None);
    }

    #[test]
    fn warp_matrix_produced_for_corner_pin() {
        let mut t = OutputTransform::default();
        t.corners[0] = [0.05, 0.05];
        let m = warp_matrix(&t).expect("warp needed");
        assert!(m.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn warp_matrix_degenerate_quad_is_none() {
        // Collapse every corner onto one point — singular homography.
        let t = OutputTransform {
            scale: 0.01,
            corners: [[0.0; 2]; 4],
            ..Default::default()
        };
        // scale=0.01 is a tiny but valid quad; force degeneracy via corners.
        let mut t2 = t;
        t2.corners = [[0.5, 0.5], [-0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]];
        t2.scale = 1.0;
        // All four corners now coincide at the centre.
        let q = transformed_quad(&t2);
        assert_close(q[0], q[2]);
        assert_eq!(warp_matrix(&t2), None);
    }
}
