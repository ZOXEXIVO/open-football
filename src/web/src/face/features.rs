//! Ears, eyes, brows, nose and mouth — the parts drawn over the lit skin.
//!
//! None of it is outlined. A feature is a stack of soft shapes: the plane
//! that faces the light, the plane that turns from it, the crease where two
//! meet, and the one or two crisp marks a camera would resolve — a lash
//! line, a nostril, the line between the lips, a catchlight.

use super::canvas::{Blur, Canvas, PathBuilder};
use super::color::Rgb;
use super::geometry::Landmarks;
use super::identity::Identity;
use super::tones::Tones;

pub struct Features;

impl Features {
    /// Ears go on before the head so the head overlaps their root.
    ///
    /// Seen from the front an ear is not an oval and not a blade: what
    /// reads is the rolled helix standing clear of the skull, the scapha
    /// groove shadowed just inside it, the antihelix ridge inside that, the
    /// concha in deep shadow against the head, and a small soft lobe. All
    /// of that has to live in the strip that is actually OUTSIDE the head
    /// outline — anything drawn further in is painted over by the head a
    /// moment later, which is how an ear ends up a flat pale plate.
    ///
    /// The two references matter separately. The ear HANGS off the side of
    /// the skull and does not follow the jaw: below the cheekbone the face
    /// narrows away in front of it, which is what lets a lobe stand clear
    /// of the cheek, so the outer edge is measured from the widest the head
    /// gets behind it. The root is measured from the outline at its own
    /// height instead, so it is always tucked under the head and no gap of
    /// background can open between the lobe and the jaw.
    pub fn ears(c: &mut Canvas, l: &Landmarks, t: &Tones) {
        let e = &l.ear;
        let top = e.top;
        let bottom = e.bottom;
        let mid = (top + bottom) / 2.0;
        let h = (bottom - top) / 2.0;
        let w = e.width;
        let anchor = l.half_width_at(mid);
        // Cartilage is thin: light comes through it, so an ear is redder
        // than the cheek beside it, and a shade darker because none of it
        // faces the key light square on
        let base = t.skin.mix(t.skin_warm, 0.30).shade(0.93);
        for side in [-1.0f32, 1.0] {
            // `f` is in ear widths out from the head's edge: 0 is the
            // silhouette, 1 the furthest the helix stands off it
            let o = |f: f32, y: f32| {
                l.cx + side * (l.half_width_at(y).max(anchor) + (e.out - 2.4) + w * f)
            };
            // The root, under the head whatever the jaw is doing
            let r = |f: f32, y: f32| l.edge_x(y, side) + side * w * f;
            let outline = PathBuilder::smooth_closed(
                &[
                    (o(-0.04, top), top),
                    (o(0.30, top + h * 0.18), top + h * 0.18),
                    (o(0.56, top + h * 0.46), top + h * 0.46),
                    (o(0.72, top + h * 0.82), top + h * 0.82),
                    (o(0.68, mid + h * 0.14), mid + h * 0.14),
                    (o(0.58, mid + h * 0.46), mid + h * 0.46),
                    (o(0.44, bottom - h * 0.34), bottom - h * 0.34),
                    (o(0.42, bottom - h * 0.16), bottom - h * 0.16),
                    (o(0.20, bottom), bottom),
                    (r(-0.16, bottom - h * 0.10), bottom - h * 0.10),
                    (r(-0.34, mid + h * 0.30), mid + h * 0.30),
                    (r(-0.36, top + h * 0.50), top + h * 0.50),
                    (r(-0.20, top + 2.0), top + 2.0),
                ],
                0.25,
            );
            c.fill(&outline, base, 1.0, Blur::Crisp);
            let id = if side < 0.0 { "earl" } else { "earr" };
            c.raw(&format!(
                r#"<clipPath id="{id}"><path d="{outline}"/></clipPath>"#
            ));
            c.clip(id);
            // The ear stands in the same light as the head, and this is the
            // one piece of skin the form gradients never reached: without
            // them it reads as a pale plate stuck to the silhouette. Held
            // back from the head's own strength, and laid down before the
            // anatomy so the anatomy still reads through it
            c.rect(0.0, 0.0, 200.0, 250.0, "url(#lat)", 0.55);
            c.rect(0.0, 0.0, 200.0, 250.0, "url(#vert)", 0.45);
            // The concha: the hollow against the head, deepest at the
            // canal. It is the darkest thing on an ear and it is what tells
            // the eye the ear stands off the cheek rather than lying on it
            c.ellipse(
                o(0.20, mid),
                mid,
                w * 0.17,
                h * 0.44,
                t.skin_shadow,
                0.46,
                Blur::Fine,
                0.0,
            );
            c.ellipse(
                o(0.13, mid + h * 0.08),
                mid + h * 0.08,
                w * 0.10,
                h * 0.22,
                t.skin_shadow,
                0.48,
                Blur::Hair,
                0.0,
            );
            // The antihelix: the ridge between that hollow and the groove,
            // catching a little light along its crest
            let anti = PathBuilder::smooth_open(
                &[
                    (o(0.18, top + h * 0.62), top + h * 0.62),
                    (o(0.38, mid - h * 0.12), mid - h * 0.12),
                    (o(0.40, mid + h * 0.35), mid + h * 0.35),
                    (o(0.27, mid + h * 0.78), mid + h * 0.78),
                ],
                0.2,
            );
            c.stroke(&anti, t.skin_hi, 1.6, 0.38, Blur::Hair);
            // The scapha: the groove between that ridge and the rolled rim
            let scapha = PathBuilder::smooth_open(
                &[
                    (o(0.24, top + h * 0.16), top + h * 0.16),
                    (o(0.50, top + h * 0.52), top + h * 0.52),
                    (o(0.52, mid + h * 0.06), mid + h * 0.06),
                    (o(0.42, mid + h * 0.50), mid + h * 0.50),
                    (o(0.28, bottom - h * 0.40), bottom - h * 0.40),
                ],
                0.2,
            );
            c.stroke(&scapha, t.skin_dk2, 2.0, 0.42, Blur::Hair);
            // The helix: the rolled rim itself, lit down its outside
            let helix = PathBuilder::smooth_open(
                &[
                    (o(0.15, top + 1.6), top + 1.6),
                    (o(0.50, top + h * 0.24), top + h * 0.24),
                    (o(0.64, top + h * 0.70), top + h * 0.70),
                    (o(0.59, mid + h * 0.16), mid + h * 0.16),
                    (o(0.47, mid + h * 0.46), mid + h * 0.46),
                    (o(0.33, bottom - h * 0.34), bottom - h * 0.34),
                ],
                0.2,
            );
            c.stroke(&helix, t.skin_hi, 1.9, 0.52, Blur::Hair);
            c.stroke(&helix, t.skin_spec, 0.7, 0.15, Blur::Hair);
            // The tragus, the little flap that closes the canal off in
            // front, and the notch under it
            c.ellipse(
                o(0.09, mid - h * 0.04),
                mid - h * 0.04,
                w * 0.08,
                h * 0.16,
                t.skin_dk,
                0.34,
                Blur::Hair,
                0.0,
            );
            // The lobe: small, soft and warm, lit from above
            let ly = bottom - h * 0.26;
            c.ellipse(
                o(0.24, ly),
                ly,
                w * 0.20,
                h * 0.26,
                t.skin_warm,
                0.44,
                Blur::Soft,
                0.0,
            );
            c.ellipse(
                o(0.26, ly - h * 0.06),
                ly - h * 0.06,
                w * 0.10,
                h * 0.11,
                t.skin_hi,
                0.24,
                Blur::Fine,
                0.0,
            );
            // Where the ear tucks under the temple, and where the rim
            // turns away from the light on its outer edge
            c.ellipse(
                o(0.16, top + 2.0),
                top + 2.0,
                w * 0.45,
                3.5,
                t.skin_dk2,
                0.30,
                Blur::Soft,
                0.0,
            );
            let ey = mid - h * 0.2;
            c.ellipse(
                o(0.70, ey),
                ey,
                w * 0.12,
                h * 0.72,
                t.skin_dk2,
                0.30,
                Blur::Fine,
                0.0,
            );
            c.close("g");
        }
    }

    pub fn eyes(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, aggr: f32) {
        let es = &l.eye_shape;
        let ey = l.eye;
        let lash = Rgb::hex("#1A100C");
        for (i, (ex, side)) in [(l.eye_l, -1.0f32), (l.eye_r, 1.0)].into_iter().enumerate() {
            let rx = es.rx;
            let ry = es.ry;
            let inner = (ex - side * rx, ey + es.tilt * 0.35);
            let outer = (ex + side * rx, ey - es.tilt);
            let peak_x = inner.0 + side * 2.0 * rx * es.peak;
            let top = ey - ry;
            let low = (ex + side * rx * 0.1, ey + ry * es.bottom);
            let upper = format!(
                "M{:.1} {:.1} C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1} C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
                inner.0,
                inner.1,
                inner.0 + side * rx * 0.30,
                inner.1 - ry * 0.55,
                peak_x - side * rx * 0.35,
                top,
                peak_x,
                top,
                peak_x + side * rx * 0.40,
                top,
                outer.0 - side * rx * 0.22,
                outer.1 - ry * 0.40,
                outer.0,
                outer.1,
            );
            let lower = format!(
                "C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1} C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
                outer.0 - side * rx * 0.28,
                outer.1 + ry * es.bottom * 0.70,
                low.0 + side * rx * 0.38,
                low.1,
                low.0,
                low.1,
                low.0 - side * rx * 0.40,
                low.1,
                inner.0 + side * rx * 0.28,
                inner.1 + ry * es.bottom * 0.45,
                inner.0,
                inner.1,
            );
            let almond = format!("{upper} {lower}Z");
            let lower_edge = format!("M{:.1} {:.1} {lower}", outer.0, outer.1);

            // Socket floor: the sclera and everything in it
            c.fill_ref(&almond, "scg", 1.0, Blur::Crisp);
            c.raw(&format!(
                r#"<clipPath id="ec{i}"><path d="{almond}"/></clipPath>"#
            ));
            c.clip(&format!("ec{i}"));
            // The sclera is a ball: darker into both corners and under the
            // upper lid, where the lashes throw a shadow
            c.ellipse(
                inner.0,
                ey + 0.5,
                3.5,
                ry * 1.4,
                t.sclera_dk,
                0.45,
                Blur::Fine,
                0.0,
            );
            c.ellipse(
                outer.0,
                ey,
                3.0,
                ry * 1.3,
                t.sclera_dk,
                0.35,
                Blur::Fine,
                0.0,
            );
            // Iris, looking a touch inward like a pair of eyes fixed on the
            // lens
            let ix = ex - side * 0.25;
            let iy = ey - 0.15;
            let ir = es.iris_r;
            c.circle(ix, iy, ir, t.iris_rim, 1.0, Blur::Crisp);
            c.ellipse_ref(ix, iy, ir - 0.35, ir - 0.35, "irg", 1.0, Blur::Crisp, 0.0);
            // Fibres: radial strands of the stroma, alternating light and
            // dark, no two eyes the same
            for k in 0..14 {
                let a = (k as f32 / 14.0) * std::f32::consts::TAU + id.jitter(k, 30 + i) * 0.4;
                let r0 = es.pupil_r + 0.4;
                let r1 = ir - 0.5;
                let d = PathBuilder::line(
                    (ix + a.cos() * r0, iy + a.sin() * r0),
                    (ix + a.cos() * r1, iy + a.sin() * r1),
                );
                let (col, op) = if k % 2 == 0 {
                    (t.iris_hi, 0.22 + id.jitter(k, 40) * 0.2)
                } else {
                    (t.iris_dk, 0.20 + id.jitter(k, 41) * 0.2)
                };
                c.stroke(&d, col, 0.35, op, Blur::Hair);
            }
            // Light passing through the iris lights the side opposite the
            // key, the collarette ring round the pupil
            c.ellipse(
                ix + 1.1,
                iy + 1.3,
                ir * 0.55,
                ir * 0.42,
                t.iris_hi,
                0.32,
                Blur::Fine,
                0.0,
            );
            c.circle(ix, iy, es.pupil_r + 0.8, t.iris_dk, 0.45, Blur::Fine);
            c.circle(
                ix,
                iy,
                es.pupil_r + 0.25,
                Rgb::hex("#0A0806"),
                0.6,
                Blur::Hair,
            );
            c.circle(ix, iy, es.pupil_r, Rgb::hex("#0A0806"), 1.0, Blur::Crisp);
            // The upper lid's shadow across the top of the eye
            let lid_cap = if es.lid_extra > 0.3 { ry } else { ry * 0.62 };
            let lid_ry = (ry * 0.34 + id.morph.lid_heavy * 0.9 + es.lid_extra * 0.7 + aggr * 0.5)
                .clamp(1.0, lid_cap);
            let lid_op =
                (0.30 + id.morph.lid_heavy * 0.14 + es.lid_extra * 0.08 + aggr * 0.08).min(0.55);
            c.ellipse(
                ex,
                top - 0.2,
                rx * 1.05,
                lid_ry + 0.8,
                Rgb::hex("#241713"),
                lid_op,
                Blur::Fine,
                0.0,
            );
            // Catchlights: the studio softbox, upper-left, and its bounce
            c.ellipse(
                ix - ir * 0.42,
                iy - ir * 0.50,
                0.95,
                0.75,
                Rgb::WHITE,
                0.92,
                Blur::Crisp,
                -20.0,
            );
            c.circle(
                ix + ir * 0.38,
                iy + ir * 0.42,
                0.35,
                Rgb::WHITE,
                0.35,
                Blur::Crisp,
            );
            c.close("g");

            // Lash line: a soft dark mass along the upper lid, heavier and
            // thicker toward the outer corner, with a crisp edge under it
            c.stroke(&upper, lash, 1.9, 0.42, Blur::Soft);
            c.stroke(&upper, lash, 1.15, 0.88, Blur::Hair);
            let tail = PathBuilder::arc(
                (peak_x + side * rx * 0.3, top - 0.2),
                (outer.0 - side * 1.5, outer.1 - 1.4),
                (outer.0 + side * 1.6, outer.1 - 0.4),
            );
            c.stroke(&tail, lash, 1.3, 0.70, Blur::Hair);
            // Lower lid: its own thickness catches light, lashes below it
            c.stroke(
                &lower_edge,
                t.skin_hi.mix(t.sclera, 0.4),
                0.7,
                0.50,
                Blur::Hair,
            );
            c.stroke(&lower_edge, t.skin_dk2, 0.9, 0.30, Blur::Fine);
            c.ellipse(ex, low.1 + 1.6, rx * 0.8, 1.1, lash, 0.16, Blur::Fine, 0.0);

            // Crease: the fold above the lid, absent on a monolid, hidden
            // under the hood on a hooded eye
            let crease_y = top - 3.0 - es.lid_extra * 0.4 - id.morph.lid_heavy * 0.6;
            if es.crease > 0.01 {
                let d = format!(
                    "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
                    inner.0 + side * 1.0,
                    inner.1 - 1.0,
                    peak_x,
                    crease_y - 1.2,
                    outer.0 + side * 1.0,
                    outer.1 - 1.6,
                );
                c.stroke(&d, t.skin_dk2, 1.0, es.crease + 0.1, Blur::Fine);
                c.stroke(&d, t.skin_hi, 0.6, 0.2, Blur::Hair);
            }
            if id.eye_st == 1 {
                // Hooded: skin folds over the outer half of the lash line
                let hood = PathBuilder::arc(
                    (peak_x - side * 2.0, top - 1.6),
                    (outer.0 - side * 3.0, top - 0.6),
                    (outer.0 + side * 1.8, outer.1 + 0.4),
                );
                c.stroke(&hood, t.skin, 3.0, 0.90, Blur::Fine);
                c.stroke(&hood, t.skin_dk2, 0.8, 0.30, Blur::Fine);
            }
            if id.eye_st == 3 {
                // Epicanthic fold: skin covers the inner corner
                let fold = PathBuilder::arc(
                    (inner.0 - side * 1.0, inner.1 - 2.6),
                    (inner.0 - side * 0.2, inner.1 + 0.2),
                    (inner.0 + side * 3.0, inner.1 + 1.8),
                );
                c.stroke(&fold, t.skin, 2.0, 0.90, Blur::Fine);
                c.stroke(&fold, t.skin_dk2, 0.6, 0.25, Blur::Hair);
            } else {
                // Tear duct
                c.ellipse(
                    inner.0 + side * 0.6,
                    inner.1 + 0.3,
                    1.1,
                    0.8,
                    t.canthus,
                    0.75,
                    Blur::Hair,
                    0.0,
                );
                c.circle(
                    inner.0 + side * 0.9,
                    inner.1 + 0.1,
                    0.35,
                    Rgb::WHITE,
                    0.35,
                    Blur::Crisp,
                );
            }
        }
    }

    pub fn brows(
        c: &mut Canvas,
        l: &Landmarks,
        t: &Tones,
        id: &Identity,
        aggr: f32,
        hair_col: Rgb,
    ) {
        let bs = &l.brow_shape;
        let brow_col = hair_col.shade(0.85).desaturate(0.1);
        let brow_lt = brow_col.lift(0.18);
        for (bi, (ex, side)) in [(l.eye_l, -1.0f32), (l.eye_r, 1.0)].into_iter().enumerate() {
            let inner_x = ex - side * (bs.len - 5.0);
            let outer_x = ex + side * (bs.len + 3.0);
            let y0 = l.brow + 1.6 + aggr * 2.2;
            let yc = l.brow - bs.arch * 1.5 * (1.0 - aggr * 0.35);
            let y1 = l.brow + 0.8 + bs.tilt * 2.4;
            let peak_x = inner_x + (outer_x - inner_x) * 0.62;
            let along = |u: f32| -> (f32, f32) {
                // Quadratic through head, arch, tail
                let x = inner_x + (outer_x - inner_x) * u;
                let y = (1.0 - u) * (1.0 - u) * y0 + 2.0 * u * (1.0 - u) * yc + u * u * y1;
                (x, y)
            };
            let spine = format!(
                "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
                inner_x, y0, peak_x, yc, outer_x, y1
            );
            // The brow bone's light just above the hair
            let bone = format!(
                "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
                inner_x + side * 2.0,
                y0 - 3.0,
                peak_x,
                yc - 3.2,
                outer_x,
                y1 - 2.4
            );
            c.stroke(
                &bone,
                t.skin_hi,
                2.4,
                0.14 + id.morph.brow_ridge * 0.06,
                Blur::Soft,
            );
            // Mass: thick at the head, thinning to the tail
            let thick = bs.thickness;
            c.stroke(&spine, brow_col, 4.6 * thick, 0.66, Blur::Soft);
            let head = format!(
                "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
                inner_x,
                y0,
                along(0.3).0,
                along(0.3).1 - 0.4,
                along(0.55).0,
                along(0.55).1
            );
            c.stroke(&head, brow_col, 3.2 * thick, 0.72, Blur::Fine);
            // Strands: the head grows upward, the arch outward, the tail
            // down and out, each a short tapered stroke
            let n = bs.strands;
            for k in 0..n {
                let u = k as f32 / (n - 1) as f32;
                let (sx, sy) = along(u);
                let spread = if u < 0.5 { 1.4 } else { 0.9 } * thick;
                let sx = sx + side * (id.jitter(k, bi) - 0.5) * 1.5;
                let sy = sy + (id.jitter(k, bi + 2) - 0.5) * spread * 2.2;
                let angle = if u < 0.30 {
                    -1.15 + u * 1.2
                } else if u < 0.70 {
                    -0.45 + (u - 0.3) * 0.8
                } else {
                    -0.13 + (u - 0.7) * 1.3
                };
                let len = 2.4 + id.jitter(k, bi + 4) * 1.4 - u * 0.6;
                let dx = side * angle.cos() * len;
                let dy = angle.sin() * len;
                let col = if id.jitter(k, bi + 6) > 0.72 {
                    brow_lt
                } else {
                    brow_col
                };
                let op = 0.45 + id.jitter(k, bi + 8) * 0.40;
                let d = format!(
                    "M{sx:.1} {sy:.1} q{:.1} {:.1} {dx:.1} {dy:.1}",
                    dx * 0.4,
                    dy * 0.6 - 0.3
                );
                c.stroke(&d, col, 0.62 * thick, op, Blur::Hair);
            }
        }
        // Glabella: frown lines on a genuinely hard face
        if aggr > 0.5 {
            let gop = (aggr - 0.5) * 0.5 + l.maturity * 0.06;
            for gx in [l.cx - 2.6, l.cx + 2.6] {
                let d =
                    PathBuilder::line((gx, l.brow - 2.5), (gx + (l.cx - gx) * 0.2, l.brow + 4.0));
                c.stroke(&d, t.skin_dk2, 0.9, gop, Blur::Fine);
            }
        }
    }

    pub fn nose(c: &mut Canvas, l: &Landmarks, t: &Tones) {
        let cx = l.cx;
        let ns = &l.nose_shape;
        let ny = l.nose;
        let dark = Rgb::hex("#1A0E0A");
        for side in [-1.0f32, 1.0] {
            let nx = cx + side * ns.tip * 0.55;
            // Nostril: a dark comma angled with the ala, softened at the rim
            c.ellipse(
                nx,
                ny - 0.2,
                ns.nostril * 1.25,
                2.2,
                dark,
                0.32,
                Blur::Soft,
                side * -22.0,
            );
            c.ellipse(
                nx,
                ny - 0.2,
                ns.nostril * 1.1,
                1.7,
                dark,
                0.80,
                Blur::Fine,
                side * -22.0,
            );
            // Alar groove: the crease where the wing meets the cheek
            let groove = PathBuilder::arc(
                (cx + side * ns.tip * 0.85, ny - 6.5),
                (cx + side * (ns.tip + 1.4), ny - 1.5),
                (cx + side * ns.tip * 0.72, ny + 2.6),
            );
            let op = if side > 0.0 { 0.55 } else { 0.38 };
            c.stroke(&groove, t.skin_dk2, 1.6, op, Blur::Fine);
            // The wing itself, lit on the near side
            if side < 0.0 {
                c.ellipse(
                    cx + side * ns.tip * 0.62,
                    ny - 3.6,
                    2.6,
                    1.8,
                    t.skin_hi,
                    0.30,
                    Blur::Fine,
                    0.0,
                );
            }
        }
        // Columella between the nostrils, and the shadow under the tip
        c.ellipse(cx, ny + 0.8, 1.7, 1.5, t.skin_hi, 0.30, Blur::Fine, 0.0);
        c.ellipse(cx, ny + 2.9, 2.6, 1.2, t.skin_dk2, 0.34, Blur::Fine, 0.0);
    }

    pub fn mouth(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, aggr: f32) {
        let cx = l.cx;
        let ms = &l.mouth_shape;
        let my = l.mouth;
        let half = ms.half;
        let lip_top = my - ms.upper;
        let corner_dy = aggr * 1.9 - 0.7;
        let upper = format!(
            "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}Z",
            cx - half,
            my + corner_dy,
            cx - half * 0.45,
            lip_top - 0.6,
            cx - 3.4,
            lip_top,
            lip_top + ms.bow,
            cx + 3.4,
            lip_top,
            cx + half * 0.45,
            lip_top - 0.6,
            cx + half,
            my + corner_dy,
            my + 1.0,
            cx - half,
            my + corner_dy,
        );
        let lower = format!(
            "M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}Z",
            cx - half + 1.2,
            my + 0.6 + corner_dy * 0.6,
            my + ms.lower * 2.15,
            cx + half - 1.2,
            my + 0.6 + corner_dy * 0.6,
            my + 1.2,
            cx - half + 1.2,
            my + 0.6 + corner_dy * 0.6,
        );
        // The upper lip faces down, away from the light; the lower faces up
        c.fill(&upper, t.lip_dk, 0.80, Blur::Fine);
        c.fill(&lower, t.lip, 0.88, Blur::Fine);
        c.fill(&lower, t.lip_dk, 0.22, Blur::Soft);
        // White roll: the vermilion border catching light
        let roll = format!(
            "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
            cx - half + 1.0,
            my - 0.4,
            cx - half * 0.45,
            lip_top - 1.1,
            cx - 3.4,
            lip_top - 0.5,
            lip_top + ms.bow - 0.5,
            cx + 3.4,
            lip_top - 0.5,
            cx + half * 0.45,
            lip_top - 1.1,
            cx + half - 1.0,
            my - 0.4,
        );
        c.stroke(&roll, t.skin_hi, 0.7, 0.40, Blur::Hair);
        // Lower lip: a highlight band, and the vertical grain of lip skin
        c.ellipse(
            cx - 1.5,
            my + ms.lower * 0.95,
            half * 0.42,
            ms.lower * 0.48,
            t.lip_hi,
            0.36,
            Blur::Fine,
            0.0,
        );
        c.ellipse(
            cx - 3.0,
            my + ms.lower * 0.8,
            half * 0.2,
            ms.lower * 0.28,
            t.lip_hi.lift(0.25),
            0.28,
            Blur::Fine,
            0.0,
        );
        c.ellipse(
            cx,
            my + ms.lower * 1.7,
            half * 0.7,
            ms.lower * 0.5,
            t.lip_dk,
            0.30,
            Blur::Fine,
            0.0,
        );
        for k in 0..7 {
            let u = (k as f32 - 3.0) / 3.0;
            let lx = cx + u * half * 0.7 + id.jitter_signed(k, 50) * 1.0;
            let d = PathBuilder::line((lx, my + 1.4), (lx + u * 0.6, my + ms.lower * 1.9));
            c.stroke(
                &d,
                t.lip_dk,
                0.35,
                0.18 + id.jitter(k, 51) * 0.15,
                Blur::Hair,
            );
        }
        // The line between the lips: the darkest mark on the lower face,
        // its corners tucked into small shadows
        let line = format!(
            "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
            cx - half,
            my + corner_dy,
            cx - half * 0.5,
            my + 1.0,
            cx - 3.6,
            my + 0.2,
            my + 1.3,
            cx + 3.6,
            my + 0.2,
            cx + half * 0.5,
            my + 1.0,
            cx + half,
            my + corner_dy,
        );
        c.stroke(&line, t.mouth_line, 1.8, 0.30, Blur::Fine);
        c.stroke(&line, t.mouth_line, 0.8, 0.72, Blur::Hair);
        let mid = format!(
            "M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}",
            cx - half * 0.55,
            my + 0.7,
            my + 0.7,
            cx + half * 0.55,
            my + 0.7
        );
        c.stroke(&mid, t.mouth_line, 1.0, 0.55, Blur::Hair);
        for side in [-1.0f32, 1.0] {
            c.ellipse(
                cx + side * (half + 0.8),
                my + corner_dy + 0.2,
                1.6,
                1.1,
                t.skin_shadow,
                0.40,
                Blur::Fine,
                0.0,
            );
            c.ellipse(
                cx + side * (half + 2.4),
                my + corner_dy - 0.6,
                1.8,
                1.0,
                t.skin_hi,
                0.18,
                Blur::Fine,
                0.0,
            );
        }
        // Under the lower lip
        c.ellipse(
            cx + 1.0,
            my + ms.lower * 2.15 + 1.5,
            half * 0.62,
            2.6,
            t.skin_dk2,
            0.46,
            Blur::Soft,
            0.0,
        );
        c.ellipse(
            cx - 1.0,
            my + ms.lower * 2.15 + 5.0,
            half * 0.45,
            2.4,
            t.skin_hi,
            0.16,
            Blur::Soft,
            0.0,
        );
    }
}
