//! Scalp hair.
//!
//! A haircut is a cap that follows the skull — the same outline the head is
//! drawn with, pushed out by however much volume the style has — with a
//! front edge shaped by the man's hairline. Inside the cap the mass is not
//! a flat fill: it darkens at the roots and under the crown, carries one
//! soft band of sheen where the key light crosses it, and is drawn over
//! with strands that run the way the style combs. The edge of every cap is
//! displaced by noise, and wisps cross the hairline onto the forehead, so
//! nowhere does hair meet skin along a vector curve.

use super::canvas::{Blur, Canvas, PathBuilder};
use super::color::Rgb;
use super::geometry::Landmarks;
use super::identity::{HairStyle, Hairline, Identity};
use super::tones::Tones;

/// How a style combs: the direction its strands run.
#[derive(Clone, Copy)]
enum Comb {
    /// Up and a little back, short — a crop, a fade top
    Up,
    /// From the part outward across the crown
    Part,
    /// From the hairline straight back over the crown
    Back,
    /// Small arcs every which way
    Curl,
    /// Down the sides, back over the top
    Down,
    /// Rows converging toward the nape
    Rows,
}

struct Cap {
    /// Lift above the crown, and volume out from the sides
    top: f32,
    side: f32,
    /// Where the crown's high point sits, off centre
    peak_dx: f32,
    comb: Comb,
    /// Strand count
    strands: usize,
}

pub struct Hair;

impl Hair {
    /// Whatever hangs behind the head — painted before the neck.
    pub fn back(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity) {
        if id.hair != HairStyle::Long {
            return;
        }
        let col = t.hair_greyed(id.grey);
        let s = &l.skull;
        let cx = l.cx;
        let w = s.parietal + 4.0;
        let d = PathBuilder::smooth_closed(
            &[
                (cx, s.crown - 8.0),
                (cx + w * 0.7, s.crown - 3.0),
                (cx + w, s.parietal_y + 6.0),
                (cx + w + 2.0, s.sub_y + 14.0),
                (cx + w - 1.0, s.chin + 4.0),
                (cx + w * 0.6, s.chin + 10.0),
                (cx, s.chin + 12.0),
                (cx - w * 0.6, s.chin + 10.0),
                (cx - w + 1.0, s.chin + 4.0),
                (cx - w - 2.0, s.sub_y + 14.0),
                (cx - w, s.parietal_y + 6.0),
                (cx - w * 0.7, s.crown - 3.0),
            ],
            0.1,
        );
        c.raw(r#"<g filter="url(#htx)">"#);
        c.fill(&d, col.shade(0.75), 1.0, Blur::Crisp);
        // Falling strands
        for k in 0..26 {
            let u = id.jitter_signed(k, 60);
            let x0 = cx + u * w * 0.95;
            let y0 = s.temple_y + id.jitter(k, 61) * 40.0;
            let y1 = y0 + 40.0 + id.jitter(k, 62) * 40.0;
            let d = PathBuilder::arc(
                (x0, y0),
                (x0 + u * 3.0, (y0 + y1) / 2.0),
                (x0 + u * 5.0, y1),
            );
            let (colr, op) = if k % 3 == 0 {
                (col.lift(0.18), 0.30)
            } else {
                (col.shade(0.5), 0.35)
            };
            c.stroke(&d, colr, 0.8 + id.jitter(k, 63) * 0.6, op, Blur::Hair);
        }
        c.raw("</g>");
        // In shadow where it passes behind the jaw and neck
        c.ellipse(
            cx,
            s.chin + 4.0,
            w * 0.9,
            22.0,
            Rgb::BLACK,
            0.35,
            Blur::Broad,
            0.0,
        );
    }

    /// The cut, painted last so it overlaps forehead, temples and ears.
    pub fn scalp(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, age: u8) {
        let col = t.hair_greyed(id.grey);
        let hi = col.lift(0.15).toward("#C9A574", 0.06);
        let dk = col.shade(0.55);
        let sheen = col.lift(0.42).toward("#A8B8C8", 0.18);

        // Hair shadow on the forehead and the sideburns come first: they are
        // skin-side, under the cap's edge
        if id.hair != HairStyle::Bald {
            Self::skin_side(c, l, t, id, col, age);
        }

        match id.hair {
            HairStyle::Bald => Self::bald(c, l, t, id),
            HairStyle::Buzz => Self::buzz(c, l, id, col),
            HairStyle::Afro => Self::afro(c, l, id, col, hi, dk),
            HairStyle::Fade => {
                Self::shaved_sides(c, l, id, col, 0.72);
                let cap = Cap {
                    top: 5.0,
                    side: -3.0,
                    peak_dx: 0.0,
                    comb: Comb::Up,
                    strands: 80,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::FauxHawk => {
                Self::shaved_sides(c, l, id, col, 0.70);
                Self::hawk(c, l, id, col, hi, dk, sheen);
            }
            HairStyle::Crop => {
                let cap = Cap {
                    top: 3.0,
                    side: 1.5,
                    peak_dx: 0.0,
                    comb: Comb::Up,
                    strands: 90,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::SidePart => {
                let cap = Cap {
                    top: 5.5,
                    side: 2.0,
                    peak_dx: -id.part_side * 9.0,
                    comb: Comb::Part,
                    strands: 100,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::Medium => {
                let cap = Cap {
                    top: 10.0,
                    side: 5.0,
                    peak_dx: -id.part_side * 3.0,
                    comb: Comb::Down,
                    strands: 110,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::SweptBack => {
                let cap = Cap {
                    top: 9.5,
                    side: 2.0,
                    peak_dx: 4.0,
                    comb: Comb::Back,
                    strands: 100,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::Curly => {
                let cap = Cap {
                    top: 8.5,
                    side: 4.5,
                    peak_dx: 0.0,
                    comb: Comb::Curl,
                    strands: 130,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::Long => {
                let cap = Cap {
                    top: 6.0,
                    side: 5.0,
                    peak_dx: -id.part_side * 6.0,
                    comb: Comb::Down,
                    strands: 120,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
            }
            HairStyle::Cornrows => {
                let cap = Cap {
                    top: 2.0,
                    side: 1.0,
                    peak_dx: 0.0,
                    comb: Comb::Rows,
                    strands: 0,
                };
                Self::cap(c, l, t, id, &cap, col, hi, dk, sheen);
                Self::braids(c, l, id, hi, dk);
            }
        }
    }

    /// Where the sides of the cap stop: the top of the ear.
    fn side_y(l: &Landmarks) -> f32 {
        l.ear.top + 7.0
    }

    /// The front edge, right to left, as points the cap path runs through.
    fn hairline_pts(l: &Landmarks, id: &Identity, age: u8, lift: f32) -> Vec<(f32, f32)> {
        let cx = l.cx;
        let hl = l.hairline - lift;
        let tw = l.skull.temple;
        let sy = Self::side_y(l);
        let recess = match age {
            0..=27 => 0.0,
            28..=32 => 3.0,
            _ => 6.0,
        };
        let mut right: Vec<(f32, f32)> = match id.hairline {
            Hairline::Straight => vec![(cx + tw * 0.80, hl + 4.0), (cx + 16.0, hl + 0.3), (cx, hl)],
            Hairline::Rounded => vec![(cx + tw * 0.82, hl + 9.0), (cx + 22.0, hl + 2.0), (cx, hl)],
            Hairline::WidowsPeak => vec![
                (cx + tw * 0.82, hl + 5.0),
                (cx + 15.0, hl - 0.6),
                (cx, hl + 3.8),
            ],
            Hairline::Receding => vec![
                (cx + tw * 0.86, hl + 3.0),
                (cx + tw * 0.56, hl - 7.0 - recess),
                (cx + 13.0, hl + 0.5),
                (cx, hl + 1.0),
            ],
        };
        let mut pts = vec![
            (l.edge_x(sy, 1.0) - 0.5, sy),
            (l.edge_x(sy - 10.0, 1.0) - 1.0, sy - 10.0),
        ];
        pts.append(&mut right);
        let n = pts.len();
        let left: Vec<(f32, f32)> = pts[..n - 1]
            .iter()
            .rev()
            .map(|(x, y)| (2.0 * cx - x + id.asym.0 * 0.6, *y))
            .collect();
        pts.extend(left);
        // No hairline is a clean curve: every point wanders a little
        for (i, p) in pts.iter_mut().enumerate() {
            if i > 1 && i + 2 < 2 * n - 1 {
                p.1 += id.jitter_signed(i, 85) * 1.4;
                p.0 += id.jitter_signed(i, 86) * 0.8;
            }
        }
        pts
    }

    /// The cap's outer edge from the left ear-top over the crown to the
    /// right, pushed out by the style's volume.
    fn outer_pts(l: &Landmarks, cap: &Cap) -> Vec<(f32, f32)> {
        let cx = l.cx;
        let s = &l.skull;
        let sy = Self::side_y(l);
        let o = cap.side;
        let top = cap.top;
        let right: Vec<(f32, f32)> = vec![
            (l.edge_x(sy, 1.0) + o * 0.6, sy),
            (l.edge_x(s.temple_y - 6.0, 1.0) + o, s.temple_y - 6.0),
            (cx + s.parietal + o * 1.15, s.parietal_y),
            (
                cx + s.parietal * 0.86 + o * 1.2,
                s.crown + 18.0 - top * 0.35,
            ),
            (
                cx + s.parietal * 0.52 + o * 0.8 + cap.peak_dx * 0.3,
                s.crown + 4.5 - top * 0.85,
            ),
        ];
        let mut pts: Vec<(f32, f32)> = right.iter().map(|(x, y)| (2.0 * cx - x, *y)).collect();
        pts.push((cx + cap.peak_dx, s.crown - top));
        pts.extend(right.into_iter().rev());
        pts
    }

    fn cap_path(l: &Landmarks, id: &Identity, age: u8, cap: &Cap, lift: f32) -> String {
        let outer = PathBuilder::smooth_open(&Self::outer_pts(l, cap), 0.1);
        let inner = PathBuilder::smooth_open(&Self::hairline_pts(l, id, age, lift), 0.2);
        // The inner run starts with its own M; joined with an L so the two
        // meet in a corner at the sideburn
        let inner = inner.replacen('M', "L", 1);
        format!("{outer} {inner}Z")
    }

    /// The shadow the cap throws on the forehead, and the sideburns. Both
    /// sit on skin, under the hair's edge.
    fn skin_side(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, col: Rgb, age: u8) {
        let cx = l.cx;
        let sy = Self::side_y(l);
        c.clip("hc");
        let pts = Self::hairline_pts(l, id, age, 0.0);
        let under: Vec<(f32, f32)> = pts.iter().map(|(x, y)| (*x, y + 3.0)).collect();
        let d = PathBuilder::smooth_open(&under, 0.2);
        let op = if id.hair == HairStyle::Buzz {
            0.10
        } else {
            0.22
        };
        c.stroke(&d, t.skin_dk2, 5.0, op, Blur::Broad);
        // Sideburns: cropped hair running from the cap down past the ear
        let burn_end = match id.hair {
            HairStyle::Fade | HairStyle::FauxHawk => l.ear.top + 8.0,
            _ => l.ear.top + 16.0 + l.maturity * 4.0,
        };
        for side in [-1.0f32, 1.0] {
            let d = PathBuilder::smooth_closed(
                &[
                    (l.edge_x(sy - 6.0, side) + side * 2.0, sy - 6.0),
                    (l.edge_x(sy - 6.0, side) - side * 2.4, sy - 4.0),
                    (l.edge_x(burn_end, side) - side * 1.4, burn_end),
                    (l.edge_x(burn_end, side) + side * 1.0, burn_end + 2.0),
                ],
                0.3,
            );
            c.fill(&d, col, 0.38, Blur::Soft);
            c.raw(&format!(
                r#"<path d="{d}" fill="{col}" filter="url(#stb)" opacity="0.42"/>"#
            ));
        }
        let _ = cx;
        c.close("g");
    }

    #[allow(clippy::too_many_arguments)]
    fn cap(
        c: &mut Canvas,
        l: &Landmarks,
        t: &Tones,
        id: &Identity,
        cap: &Cap,
        col: Rgb,
        hi: Rgb,
        dk: Rgb,
        sheen: Rgb,
    ) {
        let cx = l.cx;
        let s = &l.skull;
        let age_lift = 0.0;
        let d = Self::cap_path(l, id, 0, cap, age_lift);
        c.raw(&format!(
            r#"<clipPath id="hcap"><path d="{d}"/></clipPath>"#
        ));
        c.raw(r#"<g filter="url(#htx)">"#);
        c.fill_ref(&d, "hg", 1.0, Blur::Hair);
        c.clip("hcap");
        c.ellipse(
            cx - 6.0,
            s.crown - cap.top * 0.3 + 4.0,
            s.parietal * 0.7,
            12.0,
            hi,
            0.30,
            Blur::Vast,
            -6.0,
        );
        // Roots: dark along the hairline and under the sides, where the
        // hair grows out of shadow
        let pts = Self::hairline_pts(l, id, 0, age_lift);
        let root = PathBuilder::smooth_open(&pts, 0.2);
        c.stroke(&root, dk, 5.0, 0.45, Blur::Soft);
        let fringe: Vec<(f32, f32)> = pts.iter().map(|(x, y)| (*x, y - 0.8)).collect();
        c.stroke(
            &PathBuilder::smooth_open(&fringe, 0.2),
            t.skin,
            3.5,
            0.34,
            Blur::Soft,
        );
        for side in [-1.0f32, 1.0] {
            c.ellipse(
                l.edge_x(s.temple_y, side) + side * 2.0,
                s.temple_y - 4.0,
                6.0,
                16.0,
                dk,
                if side > 0.0 { 0.5 } else { 0.3 },
                Blur::Broad,
                0.0,
            );
        }
        // Sheen: the key light crossing the crown, high and to the left
        c.ellipse(
            cx - 10.0 + cap.peak_dx * 0.5,
            s.crown + 9.0 - cap.top * 0.5,
            24.0,
            8.0,
            sheen,
            0.26,
            Blur::Broad,
            -10.0,
        );
        c.ellipse(
            cx - 12.0 + cap.peak_dx * 0.5,
            s.crown + 8.0 - cap.top * 0.5,
            12.0,
            4.0,
            sheen,
            0.22,
            Blur::Soft,
            -10.0,
        );
        // The far side of the crown turns away
        c.ellipse(
            cx + s.parietal - 4.0,
            s.parietal_y - 6.0,
            12.0,
            30.0,
            dk,
            0.40,
            Blur::Vast,
            0.0,
        );

        Self::strands(c, l, id, cap, col, hi, dk);
        if cap.comb as u8 == Comb::Part as u8 {
            // The part line: a pale line of scalp with a dark edge each side
            let px = cx + id.part_side * 13.0;
            let part = PathBuilder::smooth_open(
                &[
                    (px, l.hairline + 1.0),
                    (px + id.part_side * 2.0, s.crown + 14.0),
                    (px + id.part_side * 6.0, s.crown + 4.0),
                ],
                0.2,
            );
            c.stroke(&part, dk, 2.4, 0.55, Blur::Fine);
            c.stroke(&part, t.skin_dk, 0.8, 0.55, Blur::Hair);
        }
        c.close("g");
        c.raw("</g>");
        Self::wisps(c, l, id, &pts, dk, col);
        Self::grey_temples(c, l, id, t);
    }

    /// Strands over the cap, run the way the style combs.
    fn strands(
        c: &mut Canvas,
        l: &Landmarks,
        id: &Identity,
        cap: &Cap,
        col: Rgb,
        hi: Rgb,
        dk: Rgb,
    ) {
        let cx = l.cx;
        let s = &l.skull;
        let sy = Self::side_y(l);
        let n = cap.strands;
        for k in 0..n {
            let jx = id.jitter_signed(k, 70);
            let jy = id.jitter(k, 71);
            // Seed points spread over the cap, denser toward the front
            let x0 = cx + jx * (s.parietal + cap.side);
            let y0 = s.crown - cap.top + jy * (sy - s.crown + cap.top);
            let len = 6.0 + id.jitter(k, 72) * 8.0;
            let (dx, dy, bend) = match cap.comb {
                Comb::Up => {
                    let a = -1.35 + jx * 0.35;
                    (a.cos() * len * 0.7, a.sin() * len * 0.7, jx * 1.5)
                }
                Comb::Part => {
                    let dir = if x0 < cx + id.part_side * 13.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    let dir = if id.part_side > 0.0 { -dir } else { dir };
                    (dir * len, len * 0.25, -dir * 1.5)
                }
                Comb::Back => (jx * len * 0.3, -len * 0.9, jx * 2.0),
                Comb::Curl => {
                    let a = id.jitter(k, 73) * std::f32::consts::TAU;
                    (a.cos() * len * 0.45, a.sin() * len * 0.45, 3.0)
                }
                Comb::Down => {
                    let side = if jx < 0.0 { -1.0 } else { 1.0 };
                    if jx.abs() > 0.5 {
                        (side * len * 0.15, len * 0.9, side * 1.5)
                    } else {
                        (side * len * 0.6, -len * 0.4, side * 1.0)
                    }
                }
                Comb::Rows => (0.0, 0.0, 0.0),
            };
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            let d = format!(
                "M{x0:.1} {y0:.1} q{:.1} {:.1} {dx:.1} {dy:.1}",
                dx * 0.5 - dy * 0.15 * bend,
                dy * 0.5 + dx * 0.15 * bend
            );
            let roll = id.jitter(k, 74);
            let (colr, op) = if id.grey > 0.0 && roll < id.grey * 0.6 {
                (Rgb::hex("#C9C5BE"), 0.45)
            } else if roll < 0.35 {
                (hi, 0.18 + id.jitter(k, 75) * 0.22)
            } else if roll < 0.8 {
                (dk, 0.30 + id.jitter(k, 75) * 0.30)
            } else {
                (col.lift(0.08), 0.30)
            };
            let width = 0.6 + id.jitter(k, 76) * 0.6;
            c.stroke_butt(&d, colr, width, op, Blur::Hair);
        }
    }

    /// Fine hairs crossing the hairline onto the skin.
    fn wisps(
        c: &mut Canvas,
        l: &Landmarks,
        id: &Identity,
        hairline: &[(f32, f32)],
        dk: Rgb,
        col: Rgb,
    ) {
        let n = hairline.len();
        c.clip("hc");
        for k in 0..34 {
            let u = id.jitter(k, 80) * (n as f32 - 1.001);
            let i = u as usize;
            let f = u - i as f32;
            let (x0, y0) = hairline[i];
            let (x1, y1) = hairline[(i + 1).min(n - 1)];
            let x = x0 + (x1 - x0) * f;
            let y = y0 + (y1 - y0) * f - 0.5;
            let len = 2.5 + id.jitter(k, 81) * 4.5;
            let lean = (x - l.cx) / 60.0;
            let d = format!(
                "M{x:.1} {y:.1} q{:.1} {:.1} {:.1} {len:.1}",
                lean * 1.5,
                len * 0.5,
                lean * 3.0
            );
            let colr = if k % 3 == 0 { col } else { dk };
            c.stroke(
                &d,
                colr,
                0.55 + id.jitter(k, 82) * 0.4,
                0.45 + id.jitter(k, 83) * 0.35,
                Blur::Hair,
            );
        }
        c.close("g");
    }

    /// Grey comes in at the temples first.
    fn grey_temples(c: &mut Canvas, l: &Landmarks, id: &Identity, t: &Tones) {
        if id.grey < 0.05 {
            return;
        }
        let s = &l.skull;
        for side in [-1.0f32, 1.0] {
            c.raw(&format!(
                r#"<ellipse cx="{:.1}" cy="{:.1}" rx="9" ry="14" fill="{}" filter="url(#stb)" opacity="{:.2}"/>"#,
                l.edge_x(s.temple_y - 4.0, side) - side * 3.0,
                s.temple_y - 6.0,
                t.grey_hair,
                (id.grey * 0.9).min(0.6)
            ));
        }
    }

    fn bald(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity) {
        let cx = l.cx;
        let s = &l.skull;
        c.clip("hc");
        // Scalp sheen, and the ghost of where the hair was around the back
        c.ellipse(
            cx - 6.0,
            s.crown + 16.0,
            22.0,
            12.0,
            t.skin_spec,
            0.22,
            Blur::Broad,
            -8.0,
        );
        for side in [-1.0f32, 1.0] {
            c.raw(&format!(
                r#"<ellipse cx="{:.1}" cy="{:.1}" rx="8" ry="18" fill="{}" filter="url(#stb)" opacity="0.35"/>"#,
                l.edge_x(s.temple_y, side),
                s.temple_y - 2.0,
                t.hair_greyed(id.grey)
            ));
        }
        c.close("g");
    }

    fn buzz(c: &mut Canvas, l: &Landmarks, id: &Identity, col: Rgb) {
        let cap = Cap {
            top: 0.8,
            side: 0.4,
            peak_dx: 0.0,
            comb: Comb::Up,
            strands: 0,
        };
        let d = Self::cap_path(l, id, 0, &cap, 0.0);
        c.clip("hc");
        c.fill(&d, col, 0.45, Blur::Soft);
        c.raw(&format!(
            r#"<path d="{d}" fill="{col}" filter="url(#stb)" opacity="0.70"/>"#
        ));
        // The scalp still shows through, lighter where the light hits
        c.ellipse(
            l.cx - 8.0,
            l.skull.crown + 14.0,
            20.0,
            10.0,
            col.lift(0.35),
            0.18,
            Blur::Broad,
            -8.0,
        );
        c.close("g");
    }

    /// Shaved sides for a fade or a hawk: speckle on the scalp, clipped to
    /// the head, densest at the top and fading out toward the ear.
    fn shaved_sides(c: &mut Canvas, l: &Landmarks, id: &Identity, col: Rgb, op: f32) {
        let s = &l.skull;
        let sy = Self::side_y(l);
        let cap = Cap {
            top: 0.5,
            side: 0.3,
            peak_dx: 0.0,
            comb: Comb::Up,
            strands: 0,
        };
        let d = Self::cap_path(l, id, 0, &cap, 0.0);
        c.clip("hc");
        c.raw(&format!(
            r#"<path d="{d}" fill="{col}" filter="url(#stb)" opacity="{op:.2}"/>"#
        ));
        // Fade: lighter toward the ear
        for side in [-1.0f32, 1.0] {
            c.ellipse(
                l.edge_x(sy - 4.0, side),
                sy - 2.0,
                12.0,
                9.0,
                col.lift(0.5).mix(Rgb::WHITE, 0.2),
                0.0,
                Blur::Broad,
                0.0,
            );
        }
        let _ = s;
        c.close("g");
    }

    #[allow(clippy::too_many_arguments)]
    fn hawk(c: &mut Canvas, l: &Landmarks, id: &Identity, col: Rgb, hi: Rgb, dk: Rgb, sheen: Rgb) {
        let cx = l.cx;
        let s = &l.skull;
        let half = 23.0;
        let top = s.crown - 8.0;
        let hl = l.hairline + 1.0;
        let d = PathBuilder::smooth_closed(
            &[
                (cx - half, hl + 2.0),
                (cx - half - 1.0, s.crown + 12.0),
                (cx - half * 0.7, top + 2.0),
                (cx, top),
                (cx + half * 0.7, top + 2.0),
                (cx + half + 1.0, s.crown + 12.0),
                (cx + half, hl + 2.0),
                (cx, hl - 1.0),
            ],
            0.15,
        );
        c.raw(&format!(
            r#"<clipPath id="hcap"><path d="{d}"/></clipPath>"#
        ));
        c.raw(r#"<g filter="url(#htx)">"#);
        c.fill_ref(&d, "hg", 1.0, Blur::Crisp);
        c.clip("hcap");
        c.stroke(
            &PathBuilder::line((cx - half, hl + 2.0), (cx + half, hl + 2.0)),
            dk,
            5.0,
            0.4,
            Blur::Soft,
        );
        c.ellipse(
            cx - 6.0,
            s.crown - 2.0,
            14.0,
            5.0,
            sheen,
            0.28,
            Blur::Soft,
            -10.0,
        );
        let cap = Cap {
            top: 10.0,
            side: 0.0,
            peak_dx: 0.0,
            comb: Comb::Up,
            strands: 70,
        };
        Self::strands(c, l, id, &cap, col, hi, dk);
        c.close("g");
        c.raw("</g>");
    }

    #[allow(clippy::too_many_arguments)]
    fn afro(c: &mut Canvas, l: &Landmarks, id: &Identity, col: Rgb, hi: Rgb, dk: Rgb) {
        let cx = l.cx;
        let s = &l.skull;
        let cap = Cap {
            top: 30.0,
            side: 14.0,
            peak_dx: 0.0,
            comb: Comb::Curl,
            strands: 0,
        };
        let d = Self::cap_path(l, id, 0, &cap, -6.0);
        c.raw(r#"<g filter="url(#hfx)">"#);
        c.fill(&d, col, 1.0, Blur::Crisp);
        c.raw(&format!(
            r#"<path d="{d}" fill="{dk}" filter="url(#stb)" opacity="0.45"/>"#
        ));
        c.raw(&format!(
            r#"<path d="{d}" fill="{hi}" filter="url(#stb)" opacity="0.18"/>"#
        ));
        c.raw("</g>");
        // Lit on the upper-left, dark toward the right and the roots
        c.raw(&format!(
            r#"<clipPath id="hcap"><path d="{d}"/></clipPath>"#
        ));
        c.clip("hcap");
        c.ellipse(
            cx - 18.0,
            s.crown - 12.0,
            26.0,
            16.0,
            hi,
            0.22,
            Blur::Vast,
            -15.0,
        );
        c.ellipse(
            cx + s.parietal + 4.0,
            s.temple_y - 10.0,
            20.0,
            40.0,
            dk,
            0.45,
            Blur::Vast,
            0.0,
        );
        c.ellipse(
            cx,
            l.hairline + 2.0,
            s.temple,
            8.0,
            dk,
            0.45,
            Blur::Broad,
            0.0,
        );
        c.close("g");
        let _ = id;
    }

    fn braids(c: &mut Canvas, l: &Landmarks, id: &Identity, hi: Rgb, dk: Rgb) {
        let cx = l.cx;
        let s = &l.skull;
        c.clip("hcap");
        for k in 0..9 {
            let u = (k as f32 - 4.0) / 4.0;
            let x0 = cx + u * (s.temple - 6.0);
            let x1 = cx + u * s.parietal * 0.5;
            let d = PathBuilder::smooth_open(
                &[
                    (x0, l.hairline + 1.0),
                    (x0 * 0.5 + x1 * 0.5, s.crown + 12.0),
                    (x1, s.crown + 1.0),
                ],
                0.2,
            );
            c.stroke(&d, dk, 1.6, 0.55, Blur::Hair);
            c.stroke(&d, hi, 0.6, 0.35, Blur::Hair);
            // The braid's knots
            for j in 0..6 {
                let f = j as f32 / 6.0;
                let x = x0 + (x1 - x0) * f + id.jitter_signed(j, 90 + k) * 0.3;
                let y = l.hairline + 1.0 + (s.crown + 1.0 - l.hairline - 1.0) * f;
                c.ellipse(x, y, 1.1, 0.7, hi, 0.30, Blur::Hair, u * 30.0);
            }
        }
        c.close("g");
    }
}
