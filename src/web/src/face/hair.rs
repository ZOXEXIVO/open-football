//! Scalp hair.
//!
//! A haircut is a cap over the skull — the head's own outline pushed out by
//! however much volume the style has — whose front edge is the man's
//! hairline. Inside the cap the hair is a mass of clumps running the way the
//! style is combed, painted flat: the clumps lighter, the gaps between them
//! darker. The edge against the card and the edge on the forehead both
//! break up into strands, so nowhere does hair meet skin or backdrop along a
//! line.
//!
//! Short hair is sparse hair: a buzz cut or the sides of a fade are the same
//! fibres at a density low enough for the scalp to show through.

use super::canvas::{Grid, Layer, Outline, Path, Plane, Polyline, Ramp};
use super::color::Linear;
use super::geometry::Landmarks;
use super::identity::{HairStyle, Hairline, Identity};
use super::noise::Noise;
use super::shading::Shade;
use super::tones::Tones;

/// How a style combs: which way its strands run across the page.
#[derive(Clone, Copy, PartialEq)]
enum Comb {
    /// Up from the hairline, fanning a little — a crop, a fade top
    Up,
    /// From a part line out across the crown
    Part,
    /// Straight back from the hairline
    Back,
    /// Coils every which way
    Curl,
    /// Out and down from the whorl at the crown
    Down,
    /// Braided rows running back from the hairline
    Rows,
    /// Cropped to stubble: hairs seen end-on
    Stubble,
}

/// One region of hair with one character. A fade is two: the top and the
/// sides; most cuts are one.
struct Tuft {
    outline: Outline,
    /// The front edge where hair meets forehead, if this tuft has one
    hairline: Option<Polyline>,
    comb: Comb,
    /// How ragged the silhouette is
    fuzz: f32,
    /// How much of the scalp the hair hides, by height on the page
    density: Density,
}

#[derive(Clone, Copy)]
enum Density {
    Full,
    /// A fade: thick above `top`, thinning to `floor` at `bottom`
    Fade {
        top: f32,
        bottom: f32,
        floor: f32,
        peak: f32,
    },
    Even(f32),
}

impl Density {
    fn at(self, y: f32) -> f32 {
        match self {
            Density::Full => 1.0,
            Density::Even(d) => d,
            Density::Fade {
                top,
                bottom,
                floor,
                peak,
            } => floor + (peak - floor) * Ramp::smooth(bottom, top, y),
        }
    }
}

/// One mass of hair, ready to lay down.
pub struct HairMass {
    pub cover: Plane,
    pub albedo: Vec<Linear>,
}

impl HairMass {
    pub fn paint(&self) -> Layer {
        let grid = self.cover.grid;
        Layer::paint(&grid, |i, j, _, _| {
            let k = j * grid.w + i;
            let a = self.cover.v[k];
            (a > 0.0).then(|| (self.albedo[k], a))
        })
    }
}

/// Lift above the crown, volume out from the sides, and where the crown's
/// high point sits.
struct Cap {
    top: f32,
    side: f32,
    peak_dx: f32,
}

pub struct Hair;

impl Hair {
    /// Whatever hangs behind the head — painted before the neck.
    pub fn back(
        grid: &Grid,
        l: &Landmarks,
        t: &Tones,
        id: &Identity,
        noise: &Noise,
    ) -> Option<HairMass> {
        if id.hair != HairStyle::Long {
            return None;
        }
        let s = &l.skull;
        let cx = l.cx;
        let w = s.parietal + 4.0;
        let outline = Outline::smooth(
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
        let tuft = Tuft {
            outline,
            hairline: None,
            comb: Comb::Down,
            fuzz: 2.5,
            density: Density::Full,
        };
        Some(Self::mass(grid, l, t, id, noise, &tuft))
    }

    /// The cut itself, as the tufts it is made of, back to front.
    pub fn scalp(
        grid: &Grid,
        l: &Landmarks,
        t: &Tones,
        id: &Identity,
        noise: &Noise,
        age: u8,
    ) -> Vec<HairMass> {
        let s = &l.skull;
        let sy = Self::side_y(l);
        let cap = |top: f32, side: f32, peak_dx: f32| Cap { top, side, peak_dx };
        // Afro-textured hair coils whatever it is cut to; only a braid or a
        // clipper keeps its own texture
        let coily = id.phenotype.afro_hair();
        let tuft = |c: Cap, lift_hl: f32, comb: Comb, fuzz: f32, density: Density| Tuft {
            outline: Self::cap_outline(l, id, age, &c, lift_hl),
            hairline: Some(Self::hairline(l, id, age, lift_hl)),
            comb: if coily && !matches!(comb, Comb::Stubble | Comb::Rows) {
                Comb::Curl
            } else {
                comb
            },
            fuzz,
            density,
        };
        // Clipped hair on the sides, fading out toward the ear
        let sides = |peak: f32, floor: f32| {
            tuft(
                cap(0.5, 0.3, 0.0),
                0.0,
                Comb::Stubble,
                0.4,
                Density::Fade {
                    top: s.parietal_y - 8.0,
                    bottom: sy + 2.0,
                    floor,
                    peak,
                },
            )
        };

        let mut tufts: Vec<Tuft> = Vec::new();
        match id.hair {
            HairStyle::Bald => tufts.push(sides(0.28, 0.0)),
            HairStyle::Buzz => tufts.push(tuft(
                cap(0.8, 0.4, 0.0),
                0.0,
                Comb::Stubble,
                0.5,
                Density::Even(0.82),
            )),
            HairStyle::Fade => {
                tufts.push(sides(0.75, 0.06));
                tufts.push(tuft(cap(5.0, -3.0, 0.0), 0.0, Comb::Up, 4.0, Density::Full));
            }
            HairStyle::FauxHawk => {
                tufts.push(sides(0.7, 0.05));
                tufts.push(Self::hawk(l, id));
            }
            HairStyle::Crop => {
                tufts.push(tuft(cap(3.0, 1.5, 0.0), 0.0, Comb::Up, 1.4, Density::Full))
            }
            HairStyle::SidePart => tufts.push(tuft(
                cap(5.5, 2.0, -id.part_side * 9.0),
                0.0,
                Comb::Part,
                1.2,
                Density::Full,
            )),
            HairStyle::Medium => tufts.push(tuft(
                cap(10.0, 5.0, -id.part_side * 3.0),
                0.0,
                Comb::Down,
                2.2,
                Density::Full,
            )),
            HairStyle::SweptBack => tufts.push(tuft(
                cap(9.5, 2.0, 4.0),
                0.0,
                Comb::Back,
                1.4,
                Density::Full,
            )),
            HairStyle::Curly => tufts.push(tuft(
                cap(8.5, 4.5, 0.0),
                0.0,
                Comb::Curl,
                3.0,
                Density::Full,
            )),
            HairStyle::Long => tufts.push(tuft(
                cap(6.0, 5.0, -id.part_side * 6.0),
                0.0,
                Comb::Down,
                2.0,
                Density::Full,
            )),
            HairStyle::Afro => tufts.push(tuft(
                cap(30.0, 14.0, 0.0),
                -6.0,
                Comb::Curl,
                5.0,
                Density::Full,
            )),
            HairStyle::Cornrows => tufts.push(tuft(
                cap(2.0, 1.0, 0.0),
                0.0,
                Comb::Rows,
                0.6,
                Density::Full,
            )),
        }
        if !matches!(id.hair, HairStyle::Bald) {
            tufts.extend(Self::sideburns(l, id));
        }
        tufts
            .iter()
            .map(|tuft| Self::mass(grid, l, t, id, noise, tuft))
            .collect()
    }

    /// Lays one tuft over the skull: coverage, and pigment with the clumps
    /// in it.
    fn mass(
        grid: &Grid,
        l: &Landmarks,
        t: &Tones,
        id: &Identity,
        noise: &Noise,
        tuft: &Tuft,
    ) -> HairMass {
        let cx = l.cx;
        let s = &l.skull;
        let dist = tuft.outline.distance(grid);
        let short = tuft.comb == Comb::Stubble;
        let seed = (id.seed % 997) as f32 * 0.61;
        let whorl = (cx - id.part_side * 4.0, s.crown + 14.0);
        let part_x = cx + id.part_side * 13.0;
        let hl = l.hairline;
        // The strand field as a coordinate that is constant along a strand:
        // its contours are the strands, its gradient points across them
        let stream = |x: f32, y: f32| -> (f32, f32) {
            match tuft.comb {
                Comb::Up | Comb::Rows => ((x - cx) * (0.012 * (y - hl)).exp(), y),
                Comb::Back => ((x - cx) * (0.024 * (y - hl)).exp(), y),
                Comb::Part => {
                    let away = if (x - part_x) * id.part_side > 0.0 {
                        1.0
                    } else {
                        -1.0
                    };
                    (y - 0.010 * (x - part_x).powi(2) + away * 40.0, x)
                }
                Comb::Down => {
                    let (dx, dy) = (x - whorl.0, y - whorl.1);
                    (dx.atan2(dy + 6.0) * 30.0, (dx * dx + dy * dy).sqrt())
                }
                Comb::Curl | Comb::Stubble => (x, y),
            }
        };
        // Clumps wander: the strand coordinate is warped by a slow field so
        // no two parallel strands stay parallel for long
        let warp = |x: f32, y: f32| 2.8 * noise.fbm(x * 0.045 - seed, y * 0.045, 2);
        let texture = |x: f32, y: f32| -> f32 {
            match tuft.comb {
                Comb::Curl => {
                    let coil = noise.fbm(x * 0.75 + seed, y * 0.75, 3);
                    let fine = noise.at(x * 1.9 - seed, y * 1.9);
                    0.5 + 0.38 * coil + 0.2 * fine
                }
                Comb::Stubble => 0.5 + 0.5 * noise.at(x * 2.1 + seed, y * 2.1),
                Comb::Rows => {
                    let (q, s) = stream(x, y);
                    let row = (q / 7.0 * std::f32::consts::PI).sin().abs();
                    let plait = 0.5 + 0.5 * (s * 1.6 + (q / 7.0).floor() * 1.7 + q * 0.4).sin();
                    Ramp::smooth(0.25, 0.75, row) * (0.6 + 0.4 * plait)
                }
                // Cropped hair: short tufts standing up, not long strands
                Comb::Up => {
                    let (q, s) = stream(x, y);
                    let q = q + warp(x, y);
                    let tuft = noise.at(q * 0.6 + seed, s * 0.22);
                    let fine = noise.at(q * 1.1 - seed, s * 0.4);
                    0.5 + 0.26 * tuft + 0.12 * fine
                }
                _ => {
                    let (q, s) = stream(x, y);
                    let q = q + warp(x, y);
                    let clump = noise.at(q * 0.45 + seed, s * 0.04);
                    let strand = noise.at(q * 1.0 - seed, s * 0.09);
                    0.5 + 0.3 * clump + 0.2 * strand
                }
            }
        };

        // Read only where the cover is worked out, four units past the cut
        let tex = Plane::from_fn(*grid, |k, x, y| {
            if dist.v[k] < -4.0 || (y - s.crown).abs() > 200.0 {
                0.0
            } else {
                texture(x, y)
            }
        });
        let fronts: Vec<f32> = match &tuft.hairline {
            Some(line) => grid.map(|i, j, x, y| {
                let k = j * grid.w + i;
                if dist.v[k] < -3.0 {
                    50.0
                } else {
                    line.foot(x, y).d
                }
            }),
            None => vec![50.0; grid.len()],
        };

        let cover = dist.map(|k, d| {
            if d < -4.0 {
                return 0.0;
            }
            let y = grid.y(k / grid.w);
            let tex = tex.v[k];
            let edge = Ramp::smooth(-0.7, 1.1 + tuft.fuzz * 0.5, d + (tex - 0.5) * tuft.fuzz);
            // The front edge thins over a few units, broken up by single
            // strands running out onto the skin
            let front = Ramp::smooth(-0.8, 3.2, fronts[k] + (tex - 0.45) * 3.0);
            // A short cut thins over the temples and ears, where the scalp
            // shows through
            let temples = if matches!(tuft.comb, Comb::Up | Comb::Part | Comb::Back) {
                1.0 - 0.4 * Ramp::smooth(s.temple_y - 4.0, Self::side_y(l), y)
            } else {
                1.0
            };
            let density = tuft.density.at(y) * temples;
            let grain = if short || tuft.comb == Comb::Rows {
                Ramp::smooth(0.25, 0.7, tex)
            } else {
                1.0
            };
            edge * front * density * grain
        });
        // The side turned from the light takes the shade, from two thirds of
        // the way out across each row of the tuft
        let turn: Vec<Option<f32>> = (0..grid.h)
            .map(|j| {
                tuft.outline
                    .row_span(grid.y(j))
                    .map(|(_, far)| cx + (far - cx) * 0.66)
            })
            .collect();
        let base = t.hair_greyed(id.grey);
        let albedo = grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return Linear::BLACK;
            }
            let tex = tex.v[k];
            // Grey comes in as whole strands, first at the temples
            let temples = Ramp::smooth(22.0, 45.0, (x - cx).abs()) * 0.6 + 0.4;
            let white = Ramp::smooth(0.55, 0.8, 0.5 + 0.5 * noise.at(x * 1.3 + 50.0, y * 0.4))
                * (id.grey * 1.6 * temples).min(1.0);
            let side = turn[j].map_or(0.0, |at| Ramp::smooth(at - 1.0, at + 1.0, x));
            // The undersides of clumps are darker, their crowns lighter
            Shade::dim(
                base.mix(t.grey_hair, white) * (0.62 + 0.72 * tex),
                side * Shade::SIDE,
            )
        });
        HairMass { cover, albedo }
    }

    /// Where the sides of the cap stop: the top of the ear.
    fn side_y(l: &Landmarks) -> f32 {
        l.ear.top + 7.0
    }

    /// The front edge, right to left, as the points the cap runs through.
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
        let last = pts.len() - 1;
        for (i, p) in pts.iter_mut().enumerate() {
            if i > 1 && i + 2 <= last {
                p.1 += id.jitter_signed(i, 85) * 1.4;
                p.0 += id.jitter_signed(i, 86) * 0.8;
            }
        }
        pts
    }

    fn hairline(l: &Landmarks, id: &Identity, age: u8, lift: f32) -> Polyline {
        Path::through(&Self::hairline_pts(l, id, age, lift), 0.2).polyline()
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

    fn cap_outline(l: &Landmarks, id: &Identity, age: u8, cap: &Cap, lift: f32) -> Outline {
        let mut pts = Path::through(&Self::outer_pts(l, cap), 0.1).points();
        pts.extend(Path::through(&Self::hairline_pts(l, id, age, lift), 0.2).points());
        Outline::polygon(pts)
    }

    /// Sideburns: cropped hair from the cap down past the top of the ear.
    fn sideburns(l: &Landmarks, id: &Identity) -> Vec<Tuft> {
        let sy = Self::side_y(l);
        let end = match id.hair {
            HairStyle::Fade | HairStyle::FauxHawk => l.ear.top + 8.0,
            _ => l.ear.top + 16.0 + l.maturity * 4.0,
        };
        [-1.0f32, 1.0]
            .map(|side| Tuft {
                outline: Outline::smooth(
                    &[
                        (l.edge_x(sy - 8.0, side) + side * 2.0, sy - 8.0),
                        (l.edge_x(sy - 8.0, side) - side * 2.6, sy - 6.0),
                        (l.edge_x(end, side) - side * 1.6, end),
                        (l.edge_x(end, side) + side * 1.0, end + 2.0),
                    ],
                    0.3,
                ),
                hairline: None,
                comb: Comb::Stubble,
                fuzz: 1.2,
                density: Density::Even(0.75),
            })
            .into_iter()
            .collect()
    }

    /// The strip of a faux-hawk down the middle of the head.
    fn hawk(l: &Landmarks, id: &Identity) -> Tuft {
        let cx = l.cx;
        let s = &l.skull;
        let half = 23.0;
        let top = s.crown - 8.0;
        let hl = l.hairline + 1.0;
        Tuft {
            outline: Outline::smooth(
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
            ),
            hairline: Some(
                Path::through(
                    &[(cx + half, hl + 2.0), (cx, hl - 1.0), (cx - half, hl + 2.0)],
                    0.2,
                )
                .polyline(),
            ),
            comb: if id.phenotype.afro_hair() {
                Comb::Curl
            } else {
                Comb::Up
            },
            fuzz: 4.0,
            density: Density::Full,
        }
    }
}
