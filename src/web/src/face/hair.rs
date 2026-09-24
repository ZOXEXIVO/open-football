//! Scalp hair.
//!
//! A haircut is a cap over the skull — the head's own outline pushed out by
//! however much volume the style has — whose front edge is the man's
//! hairline. Inside the cap the hair is a mass of clumps running the way the
//! style is combed: every pixel knows which way its strands run, and the
//! light off them is the light off fibres — two highlights across the grain
//! rather than a shine on a helmet. The edge against the card and the edge
//! on the forehead both break up into strands, so nowhere does hair meet
//! skin or backdrop along a line.
//!
//! Short hair is sparse hair: a buzz cut or the sides of a fade are the same
//! fibres at a density low enough for the scalp to show through.

use super::canvas::{Grid, Layer, Outline, Path, Plane, Polyline, Ramp};
use super::color::Linear;
use super::geometry::Landmarks;
use super::identity::{HairStyle, Hairline, Identity};
use super::noise::Noise;
use super::relief::{Relief, Volume};
use super::shading::{Occlusion, Studio, Vec3};
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
    /// Thickness of the hair at the crown and over the ears
    lift: f32,
    side: f32,
    comb: Comb,
    /// How ragged the silhouette is
    fuzz: f32,
    /// How much of the scalp the hair hides, by height on the page
    density: Density,
    gloss: f32,
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

/// Everything the hair layer needs to be lit.
pub struct HairMass {
    pub cover: Plane,
    pub z: Plane,
    /// Which way the strands run across the page
    pub dir: Vec<(f32, f32)>,
    /// Clump texture, 0..1: light on the clumps, dark in the gaps
    pub tex: Plane,
    pub albedo: Vec<Linear>,
    pub gloss: f32,
    /// How much the clumps ruffle the surface
    pub bump: f32,
}

impl HairMass {
    pub fn shade(&self, studio: &Studio, occ: &Occlusion) -> Layer {
        let grid = self.cover.grid;
        Layer::shade(&grid, |i, j, x, y| {
            let k = j * grid.w + i;
            let a = self.cover.v[k];
            if a <= 0.0 {
                return None;
            }
            let z = self.z.v[k];
            let (dx, dy) = self.z.slope(i, j);
            let (tx, ty) = self.tex.slope(i, j);
            let n = Vec3::facing(dx - tx * self.bump, dy - ty * self.bump);
            let (fx, fy) = self.dir[k];
            let fz = -(n[0] * fx + n[1] * fy) / n[2].max(0.25);
            let t = Vec3::normalized([fx, fy, fz]);
            let tex = self.tex.v[k];
            let shadow = occ.shadow(x, y, z, studio.key.dir, 0.32);
            // Light reaches the gaps between clumps last
            let ao = occ.ambient(x, y, z) * (0.55 + 0.45 * tex);
            // Thin hair at an edge has no sheen to speak of
            let gloss = self.gloss * a * a;
            let c = studio.hair(n, t, self.albedo[k], (tex - 0.5) * 0.6, gloss, shadow, ao);
            Some((c, a))
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
            lift: 0.0,
            side: 0.0,
            comb: Comb::Down,
            fuzz: 2.5,
            density: Density::Full,
            gloss: 0.9,
        };
        let floor = Plane::new(*grid, 0.0);
        Some(Self::mass(grid, l, t, id, noise, &tuft, &floor, Some(14.0)))
    }

    /// The cut itself, as the tufts it is made of, back to front.
    pub fn scalp(
        grid: &Grid,
        l: &Landmarks,
        relief: &Relief,
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
        let tuft =
            |c: Cap, lift_hl: f32, comb: Comb, fuzz: f32, density: Density, gloss: f32| Tuft {
                outline: Self::cap_outline(l, id, age, &c, lift_hl),
                hairline: Some(Self::hairline(l, id, age, lift_hl)),
                lift: c.top,
                side: c.side,
                comb: if coily && !matches!(comb, Comb::Stubble | Comb::Rows) {
                    Comb::Curl
                } else {
                    comb
                },
                fuzz,
                density,
                gloss: if coily { gloss.min(0.45) } else { gloss },
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
                0.3,
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
                0.35,
            )),
            HairStyle::Fade => {
                tufts.push(sides(0.75, 0.06));
                tufts.push(tuft(
                    cap(5.0, -3.0, 0.0),
                    0.0,
                    Comb::Up,
                    4.0,
                    Density::Full,
                    0.9,
                ));
            }
            HairStyle::FauxHawk => {
                tufts.push(sides(0.7, 0.05));
                tufts.push(Self::hawk(l, id));
            }
            HairStyle::Crop => tufts.push(tuft(
                cap(3.0, 1.5, 0.0),
                0.0,
                Comb::Up,
                1.4,
                Density::Full,
                0.85,
            )),
            HairStyle::SidePart => tufts.push(tuft(
                cap(5.5, 2.0, -id.part_side * 9.0),
                0.0,
                Comb::Part,
                1.2,
                Density::Full,
                1.0,
            )),
            HairStyle::Medium => tufts.push(tuft(
                cap(10.0, 5.0, -id.part_side * 3.0),
                0.0,
                Comb::Down,
                2.2,
                Density::Full,
                0.95,
            )),
            HairStyle::SweptBack => tufts.push(tuft(
                cap(9.5, 2.0, 4.0),
                0.0,
                Comb::Back,
                1.4,
                Density::Full,
                1.1,
            )),
            HairStyle::Curly => tufts.push(tuft(
                cap(8.5, 4.5, 0.0),
                0.0,
                Comb::Curl,
                3.0,
                Density::Full,
                0.45,
            )),
            HairStyle::Long => tufts.push(tuft(
                cap(6.0, 5.0, -id.part_side * 6.0),
                0.0,
                Comb::Down,
                2.0,
                Density::Full,
                1.0,
            )),
            HairStyle::Afro => tufts.push(tuft(
                cap(30.0, 14.0, 0.0),
                -6.0,
                Comb::Curl,
                5.0,
                Density::Full,
                0.25,
            )),
            HairStyle::Cornrows => tufts.push(tuft(
                cap(2.0, 1.0, 0.0),
                0.0,
                Comb::Rows,
                0.6,
                Density::Full,
                0.7,
            )),
        }
        if !matches!(id.hair, HairStyle::Bald) {
            tufts.extend(Self::sideburns(l, id));
        }
        tufts
            .iter()
            .map(|tuft| {
                // A mass as big as an afro is a dome of its own; everything
                // else is a shell laid over the scalp
                let dome = (tuft.lift > 14.0).then_some(Relief::DEPTH + tuft.lift * 0.9);
                Self::mass(grid, l, t, id, noise, tuft, &relief.skull, dome)
            })
            .collect()
    }

    /// Lays one tuft over the skull: coverage, depth, strand direction and
    /// texture, pigment.
    #[allow(clippy::too_many_arguments)]
    fn mass(
        grid: &Grid,
        l: &Landmarks,
        t: &Tones,
        id: &Identity,
        noise: &Noise,
        tuft: &Tuft,
        skull: &Plane,
        dome: Option<f32>,
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

        let tex = Plane::from_fn(*grid, |x, y| {
            if (y - s.crown).abs() > 200.0 {
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
        let z = if short {
            skull.map(|k, s| if cover.v[k] > 0.0 { s + 0.35 } else { 0.0 })
        } else if let Some(depth) = dome {
            let dome = Volume::raise(
                grid,
                &tuft.outline,
                depth,
                (38.0 + tuft.lift * 0.6, 1.0),
                |_| 2.6,
            );
            dome.map(|k, d| {
                if cover.v[k] <= 0.0 {
                    return 0.0;
                }
                let bare = skull.v[k] + 0.35;
                bare + (d.max(bare) - bare) * Ramp::smooth(0.0, 10.0, fronts[k])
            })
        } else {
            // Thick at the crown, thinner over the ears, thinnest where it
            // grows out of the forehead — unless it is swept up off it
            let sy = Self::side_y(l);
            let thick = Plane::from_pixels(*grid, |i, j| {
                let k = j * grid.w + i;
                if skull.v[k] <= 0.5 || dist.v[k] < -0.5 {
                    return 0.0;
                }
                let up = Ramp::smooth(sy + 4.0, s.crown + 12.0, grid.y(j));
                let t = (tuft.side.max(0.0) + 1.2) * (1.0 - up) + tuft.lift.max(0.8) * up;
                let front = if tuft.comb == Comb::Back {
                    0.6 + 0.4 * Ramp::smooth(0.0, 6.0, fronts[k])
                } else {
                    0.3 + 0.7 * Ramp::smooth(0.0, 9.0, fronts[k])
                };
                // Hair thins out toward the edge of its own cut, so a top
                // left long runs down into clipped sides rather than
                // stopping on them
                t * front * Ramp::smooth(-0.5, 5.0, dist.v[k])
            });
            let reach = tuft.lift.max(tuft.side + 1.2) + 0.5;
            Self::shell(grid, skull, &thick, reach).map(|k, z| {
                if cover.v[k] > 0.0 {
                    z.max(skull.v[k] + 0.35)
                } else {
                    0.0
                }
            })
        };
        let dir = grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return (0.0, 1.0);
            }
            if matches!(tuft.comb, Comb::Curl | Comb::Stubble) {
                let a = noise.at(x * 0.5 - seed, y * 0.5) * std::f32::consts::TAU;
                return (a.cos(), a.sin());
            }
            let e = 0.3;
            let gx = stream(x + e, y).0 - stream(x - e, y).0;
            let gy = stream(x, y + e).0 - stream(x, y - e).0;
            let len = (gx * gx + gy * gy).sqrt().max(1e-6);
            let (fx, fy) = (-gy / len, gx / len);
            let twist = 0.3 * noise.fbm(x * 0.09 + seed, y * 0.09, 2);
            let (sn, cs) = twist.sin_cos();
            (fx * cs - fy * sn, fx * sn + fy * cs)
        });
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
            // The undersides of clumps are darker, their crowns lighter
            base.mix(t.grey_hair, white) * (0.62 + 0.72 * tex)
        });
        let bump = match tuft.comb {
            Comb::Curl => 2.2,
            Comb::Rows => 1.4,
            Comb::Stubble => 0.3,
            Comb::Up => 0.5,
            _ => 0.9,
        };
        HairMass {
            cover,
            z,
            dir,
            tex,
            albedo,
            gloss: tuft.gloss,
            bump,
        }
    }

    /// The outer surface of hair of the given thickness over the scalp:
    /// every scalp point pushed out by the hair growing from it in every
    /// direction, so the shell follows the skull's curvature and rolls over
    /// its silhouette the way hair does instead of standing on it as a lid.
    fn shell(grid: &Grid, skull: &Plane, thick: &Plane, reach: f32) -> Plane {
        let r = (reach * grid.scale).ceil() as isize;
        let px = grid.px();
        let disc: Vec<(isize, isize, f32)> = (-r..=r)
            .flat_map(|dj| (-r..=r).map(move |di| (di, dj)))
            .map(|(di, dj)| (di, dj, ((di * di + dj * dj) as f32).sqrt() * px))
            .filter(|&(_, _, d)| d <= reach)
            .collect();
        let (w, h) = (grid.w as isize, grid.h as isize);
        Plane::from_pixels(*grid, |i, j| {
            let mut best = 0.0f32;
            for &(di, dj, d) in &disc {
                let (si, sj) = (i as isize + di, j as isize + dj);
                if si < 0 || sj < 0 || si >= w || sj >= h {
                    continue;
                }
                let k = (sj * w + si) as usize;
                let t = thick.v[k];
                if t > d {
                    best = best.max(skull.v[k] + (t * t - d * d).sqrt());
                }
            }
            best
        })
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
                lift: 0.0,
                side: 0.0,
                comb: Comb::Stubble,
                fuzz: 1.2,
                density: Density::Even(0.75),
                gloss: 0.4,
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
            lift: 10.0,
            side: 3.0,
            comb: if id.phenotype.afro_hair() {
                Comb::Curl
            } else {
                Comb::Up
            },
            fuzz: 4.0,
            density: Density::Full,
            gloss: 0.9,
        }
    }
}
