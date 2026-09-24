//! What lives on and in the skin of the head: its colouring, the brows,
//! lips and nostrils painted into it, the eyes set in it, the lashes over
//! them, and the ears standing off it.
//!
//! Pigment is kept apart from light. Everything in [`Complexion`] is what
//! the surface is — the studio decides what it looks like — which is why a
//! brow or a freckle keeps its edge when the skin's shading is softened by
//! scattering, exactly as it does in a photograph.

use std::f32::consts::{PI, TAU};

use super::beard::Growth;
use super::canvas::{Grid, Outline, Path, Plane, Polyline, Ramp};
use super::color::Linear;
use super::geometry::{Eye, Landmarks};
use super::identity::Identity;
use super::noise::Noise;
use super::relief::{Eyeball, Form};
use super::shading::{Occlusion, Studio, Vec3};
use super::tones::Tones;

/// The head's skin as pigment and sheen, pixel by pixel.
pub struct Complexion {
    pub albedo: Vec<Linear>,
    /// 0 matte .. 1 oily: how strongly and how tightly it reflects
    pub oil: Vec<f32>,
    /// Surface grain too fine to model, for the specular to catch
    pub micro: Plane,
}

/// One brow: the spine its hairs grow along, and how thick it is.
struct Brow {
    spine: Polyline,
    side: f32,
    thick: f32,
    sparse: f32,
    seed: f32,
}

impl Brow {
    fn new(l: &Landmarks, id: &Identity, eye: &Eye, aggr: f32) -> Brow {
        let bs = &l.brow_shape;
        let side = eye.side;
        // Aggression knits the heads of the brows toward each other and
        // down; no pair sits level, and whose rides higher is the id's
        let inner_x = eye.cx - side * (bs.len - 5.0 - aggr * 1.2);
        let outer_x = eye.cx + side * (bs.len + 3.0);
        let raise = id.jitter_signed(3, 77) * 0.9 * side;
        // Brows lie along the lower edge of the brow ridge, close over the
        // fold of the lid
        let y0 = l.brow + 3.4 + aggr * 2.2 + raise;
        let yc = l.brow + 1.8 - bs.arch * 1.5 * (1.0 - aggr * 0.35) + raise;
        let y1 = l.brow + 2.6 + bs.tilt * 2.4 + raise;
        let peak_x = inner_x + (outer_x - inner_x) * 0.62;
        let spine = Path::from((inner_x, y0))
            .quad((peak_x, yc), (outer_x, y1))
            .polyline();
        Brow {
            spine,
            side,
            thick: bs.thickness,
            sparse: if bs.strands < 30 { 0.72 } else { 1.0 },
            seed: side * 31.7 + id.seed as f32 * 0.013,
        }
    }

    /// Hair cover at a point, 0..1.
    fn density(&self, x: f32, y: f32, noise: &Noise) -> f32 {
        if !self.spine.near(x, y, 4.5) {
            return 0.0;
        }
        let foot = self.spine.foot(x, y);
        let u = foot.u;
        let below = foot.d * self.side;
        let half = self.thick * (3.6 - 2.2 * u.powf(0.8));
        // Hairs grow up and out at the head, flatten across the arch and
        // lie along the tail
        let a: f32 = if u < 0.28 {
            1.25 - u * 1.6
        } else if u < 0.7 {
            0.80 - (u - 0.28) * 0.9
        } else {
            0.42 - (u - 0.7) * 0.8
        };
        let s = u * self.spine.len();
        let q = -below * a.cos() - s * a.sin();
        let strand = 0.5 + 0.5 * noise.at(q * 1.25 + self.seed, s * 0.22 + below * 0.3);
        let fine = 0.5 + 0.5 * noise.at(q * 2.1 - self.seed, s * 0.5);
        let edge = if below > 0.0 {
            below / half
        } else {
            -below / (half * 1.15)
        } - (strand - 0.5) * 0.55;
        let body = 1.0 - Ramp::smooth(0.55, 1.15, edge);
        let ends = Ramp::smooth(0.0, 0.07, u) * (1.0 - Ramp::smooth(0.80, 1.02, u));
        body * ends * (0.6 + 0.26 * strand + 0.14 * fine) * self.sparse
    }
}

/// Where colour gathers on a face, as soft weights.
struct Zones {
    flush: Vec<Form>,
    pale: Vec<Form>,
    orbit: Vec<Form>,
    oil: Vec<Form>,
    nostrils: Vec<Form>,
    /// Where pores are coarse — the nose and the cheeks beside it — and,
    /// negatively, where the skin is fine — round the eyes
    pores: Vec<Form>,
}

impl Zones {
    fn new(l: &Landmarks, id: &Identity) -> Zones {
        let cx = l.cx;
        let s = &l.skull;
        let ns = &l.nose_shape;
        let nx = cx + id.turn * 0.35 * 3.0;
        let mut flush = vec![
            Form::blob(nx, l.nose - 5.0, ns.tip * 1.2, 12.0, 0.0, 0.38),
            Form::blob(
                nx,
                l.nose - ns.ball,
                ns.ball * 1.3,
                ns.ball * 1.2,
                0.0,
                0.12,
            ),
            Form::blob(cx, s.chin - 9.0, 10.0, 7.0, 0.0, 0.25),
        ];
        let mut orbit = Vec::new();
        let mut nostrils = Vec::new();
        for eye in &l.eyes {
            let side = eye.side;
            flush.push(Form::blob(
                cx + side * s.zygo * 0.50,
                s.zygo_y + 10.0,
                30.0,
                26.0,
                -side * 10.0,
                0.3,
            ));
            orbit.push(Form::blob(
                eye.cx,
                l.eye + 3.5,
                l.eye_shape.rx * 1.25,
                6.5,
                0.0,
                0.18 + l.maturity * 0.25 + id.morph.lid_heavy * 0.1,
            ));
            orbit.push(Form::blob(
                eye.inner.0 - side * 1.5,
                eye.inner.1,
                3.5,
                4.5,
                0.0,
                0.45,
            ));
            // The upper lid: thin skin, a shade deeper than the brow above
            orbit.push(Form::blob(
                eye.cx + side * 0.5,
                eye.top - 2.8,
                l.eye_shape.rx * 1.1,
                3.6,
                0.0,
                0.3,
            ));
            nostrils.push(Form::blob(
                nx + side * ns.tip * 0.40,
                l.nose + 0.3,
                ns.nostril * 0.95,
                1.5,
                -side * 18.0,
                1.0,
            ));
        }
        let pale = vec![
            Form::blob(cx, l.hairline + 20.0, 34.0, 20.0, 0.0, 0.7),
            Form::blob(nx, l.eye + 4.0, 4.5, 11.0, 0.0, 0.35),
        ];
        let oil = vec![
            Form::blob(cx, l.hairline + 20.0, 30.0, 16.0, 0.0, 0.4),
            Form::blob(nx, l.nose - 12.0, 7.0, 14.0, 0.0, 0.45),
            Form::blob(cx, s.chin - 9.0, 10.0, 7.0, 0.0, 0.40),
        ];
        let mut pores = vec![
            Form::blob(nx, l.nose - 6.0, ns.tip * 1.4, 14.0, 0.0, 0.8),
            Form::blob(cx, l.hairline + 22.0, 26.0, 14.0, 0.0, 0.25),
            Form::blob(cx, s.chin - 9.0, 11.0, 8.0, 0.0, 0.35),
        ];
        for eye in &l.eyes {
            pores.push(Form::blob(
                cx + eye.side * s.zygo * 0.45,
                s.zygo_y + 12.0,
                18.0,
                15.0,
                0.0,
                0.55,
            ));
            pores.push(Form::blob(eye.cx, l.eye, 15.0, 9.0, 0.0, -0.9));
        }
        Zones {
            flush,
            pale,
            orbit,
            oil,
            nostrils,
            pores,
        }
    }

    fn sum(forms: &[Form], x: f32, y: f32) -> f32 {
        forms.iter().map(|f| f.at(x, y)).sum::<f32>().min(1.0)
    }

    /// How coarse the pores are at a point, 0 (fine) .. 1 (open).
    fn coarse(forms: &[Form], x: f32, y: f32) -> f32 {
        (0.4 + forms.iter().map(|f| f.at(x, y)).sum::<f32>()).clamp(0.0, 1.0)
    }
}

pub struct Features;

impl Features {
    /// Pigment and sheen over the whole head, the razor's work included.
    #[allow(clippy::too_many_arguments)]
    pub fn complexion(
        grid: &Grid,
        l: &Landmarks,
        id: &Identity,
        t: &Tones,
        noise: &Noise,
        cover: &Plane,
        growth: &Growth,
        aggr: f32,
        grey: f32,
    ) -> Complexion {
        let zones = Zones::new(l, id);
        let brows = l.eyes.each_ref().map(|eye| Brow::new(l, id, eye, aggr));
        let brow_col = t
            .hair_greyed(grey)
            .mix(Linear::new(0.02, 0.015, 0.012), 0.62);
        let upper = l.lips.upper.distance(grid);
        let lower = l.lips.lower.distance(grid);
        let flush_k = 0.45 + id.morph.redness * 0.45;
        let freckles = if t.fairness > 0.55 {
            id.morph.freckles
        } else {
            0.0
        };
        let mole = (id.marks == 4).then(|| {
            (
                l.cx + id.jitter_signed(11, 5) * 24.0,
                130.0 + id.jitter(7, 9) * 48.0,
            )
        });

        let albedo = grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return Linear::BLACK;
            }
            let mut a = t.skin;
            // Blood shows in clouds, not ellipses
            let cloud = 0.65 + 0.7 * (0.5 + 0.5 * noise.fbm(x * 0.09 - 7.0, y * 0.09, 3));
            a = a.mix(
                t.flush,
                (0.06 + Zones::sum(&zones.flush, x, y) * flush_k * cloud).min(1.0),
            );
            a = a.mix(t.pale, Zones::sum(&zones.pale, x, y));
            a = a.mix(t.orbit, Zones::sum(&zones.orbit, x, y));
            a = a.mix(t.shaved, growth.veil.v[k]);
            // Skin is never one flat colour: broad blotches of warmth and
            // pallor, then the fine mottle of pores
            let blotch = noise.fbm(x * 0.06 + 11.0, y * 0.06, 3);
            let mottle = noise.fbm(x * 0.35 - 4.0, y * 0.35, 2);
            a = a * (1.0 + 0.08 * blotch + 0.05 * mottle);
            a = a.mix(
                t.flush,
                (noise.fbm(x * 0.11, y * 0.11 + 5.0, 2) * 0.22
                    + noise.fbm(x * 0.4 + 9.0, y * 0.4, 2) * 0.06)
                    .max(0.0),
            );
            // Pores hold a little pigment and a little shadow, and melanin
            // lies in faint specks that no complexion is free of
            let pit = Self::pore(noise, x, y, Zones::coarse(&zones.pores, x, y));
            let speck = Ramp::smooth(0.45, 0.8, noise.at(x * 0.85 + 21.0, y * 0.85));
            a = a * (1.0 - 0.10 * pit - 0.05 * speck);
            if freckles > 0.0 {
                let w = Ramp::smooth(10.0, 0.0, (y - (l.eye + 14.0)).abs())
                    * Ramp::smooth(38.0, 10.0, (x - l.cx).abs());
                let spot = Ramp::smooth(0.35, 0.62, noise.at(x * 1.1 + 40.0, y * 1.1));
                a = a.mix(t.mark, spot * w * freckles * 0.55);
            }
            if let Some((mx, my)) = mole {
                let d = ((x - mx).powi(2) + (y - my).powi(2)).sqrt();
                a = a.mix(t.mark.deepen(1.3), Ramp::smooth(1.1, 0.5, d) * 0.85);
            }

            // Lips: the vermilion with its fine vertical creases, the upper
            // a shade deeper than the lower
            let lip_u = Ramp::smooth(-0.8, 0.8, upper.v[k]);
            let lip_l = Ramp::smooth(-0.8, 0.8, lower.v[k]);
            if lip_u + lip_l > 0.0 {
                let creases = Ramp::smooth(0.1, 0.7, noise.at(x * 1.7 + 9.0, y * 0.28));
                let lip = t.lip * (1.0 - 0.14 * creases);
                a = a.mix(lip * 0.78, lip_u).mix(lip, lip_l);
            }
            let nostril = Zones::sum(&zones.nostrils, x, y);
            a = a * (1.0 - 0.55 * nostril);

            a = a.mix(growth.color, growth.stubble.v[k]);
            let brow = brows
                .iter()
                .map(|b| b.density(x, y, noise))
                .fold(0.0, f32::max);
            a.mix(brow_col, brow * 0.94)
        });

        let oil = grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return 0.0;
            }
            let lips = Ramp::smooth(-0.3, 0.6, upper.v[k].max(lower.v[k]));
            (0.18 + Zones::sum(&zones.oil, x, y))
                .max(lips * 0.85)
                .min(1.0)
        });
        // The skin's own surface: pores sunk into it, a fine grain across
        // it and a gentle unevenness under both
        let micro = Plane::from_fn(*grid, |x, y| {
            let pit = Self::pore(noise, x, y, Zones::coarse(&zones.pores, x, y));
            -0.09 * pit
                + 0.02 * noise.at(x * 2.2 + 5.0, y * 2.2)
                + 0.07 * noise.fbm(x * 0.55 + 71.0, y * 0.55, 2)
        });

        Complexion { albedo, oil, micro }
    }

    /// How deep a pore pit is at a point, 0..1: one pore to a cell of
    /// about eight tenths of a unit, opening wider where the skin is coarse.
    fn pore(noise: &Noise, x: f32, y: f32, coarse: f32) -> f32 {
        let (d, own) = noise.cells(x * 1.25, y * 1.25);
        let r = (0.16 + 0.22 * coarse) * (0.7 + 0.6 * own);
        Ramp::smooth(r, r * 0.3, d) * (0.3 + 0.7 * coarse)
    }

    /// Skin with nothing on it but its own mottle — the neck, the ears.
    /// `flush` is how much blood shows through: ear cartilage is thin.
    pub fn plain_skin(
        grid: &Grid,
        cover: &Plane,
        t: &Tones,
        noise: &Noise,
        flush: f32,
    ) -> Vec<Linear> {
        let base = t.skin.mix(t.flush, flush);
        grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return Linear::BLACK;
            }
            let blotch = noise.fbm(x * 0.06 + 11.0, y * 0.06, 3);
            let pit = Self::pore(noise, x, y, 0.3);
            base * ((1.0 + 0.07 * blotch) * (1.0 - 0.08 * pit))
        })
    }

    /// The eyeball at a point inside the opening, lit.
    #[allow(clippy::too_many_arguments)]
    pub fn eye(
        x: f32,
        y: f32,
        z: f32,
        eye: &Eye,
        ball: &Eyeball,
        t: &Tones,
        noise: &Noise,
        studio: &Studio,
        occ: &Occlusion,
        seed: f32,
    ) -> Linear {
        let n = ball.normal(x, y);
        let (ix, iy) = eye.iris;
        let (dx, dy) = (x - ix, y - iy);
        let r = (dx * dx + dy * dy).sqrt();
        let ir = eye.iris_r;

        // The sclera: not white, and redder and greyer into the corners
        let reach = (eye.outer.0 - eye.cx).abs().max(1.0);
        let across = ((x - eye.cx) / reach).abs();
        let veins = Ramp::smooth(0.45, 0.8, noise.at(x * 1.6 + seed, y * 1.6));
        let mut albedo = t.sclera.mix(
            Linear::new(0.52, 0.26, 0.24),
            Ramp::smooth(0.35, 1.05, across) * (0.40 + 0.25 * veins),
        );
        let caruncle = Ramp::smooth(
            2.6,
            1.2,
            ((x - eye.inner.0).powi(2) + (y - eye.inner.1).powi(2)).sqrt(),
        );
        albedo = albedo.mix(t.caruncle, caruncle);

        if r < ir + 0.5 {
            let iris = Self::iris(dx, dy, r, eye, t, noise, seed);
            albedo = albedo.mix(iris, Ramp::smooth(ir + 0.35, ir - 0.2, r));
        }

        // The iris is a disc facing out under the cornea, not the ball's
        // curve: flatten the normal over it
        let over_iris = Ramp::smooth(ir + 0.4, ir - 0.4, r);
        let n_lit = Vec3::normalized([
            n[0] * (1.0 - 0.6 * over_iris),
            n[1] * (1.0 - 0.6 * over_iris),
            n[2],
        ]);
        let shadow = occ.shadow(x, y, z, studio.key.dir, 0.32);
        let ao = occ.ambient(x, y, z);
        // The lid and its lashes shade the top of the ball
        let below_lid = eye.upper.foot(x, y).d * eye.side;
        let lash_shade = 0.45 + 0.55 * Ramp::smooth(0.0, 3.0, below_lid);
        let mut c = albedo * studio.skin_light(n_lit, shadow, ao) * lash_shade;

        // A wet surface: the broad sheen of the ball, then the sharp image
        // of the soft box in the cornea
        c += studio.specular(n, 0.14, 0.03, shadow) * 1.4;
        let rc = ir * 1.5;
        if r < rc {
            let (cx_, cy_) = (dx / rc, dy / rc);
            let cz = (1.0 - cx_ * cx_ - cy_ * cy_).max(0.0).sqrt();
            let refl = [2.0 * cz * cx_, 2.0 * cz * cy_, 2.0 * cz * cz - 1.0];
            for (lamp, (wx, wy), gain, vis) in [
                (&studio.key, (0.11, 0.13), 2.6, shadow.max(0.25)),
                (&studio.fill, (0.07, 0.09), 1.0, 1.0),
            ] {
                let bx = ((refl[0] - lamp.dir[0]) / wx).abs();
                let by = ((refl[1] - lamp.dir[1]) / wy).abs();
                let hit = 1.0 - Ramp::smooth(0.75, 1.0, bx.max(by));
                c += lamp.color * (hit * gain * vis * lash_shade.max(0.7));
            }
        }
        // The tear line along the lower lid catches the light
        let above_lower = -eye.lower.foot(x, y).d * eye.side;
        let wet = Ramp::smooth(0.0, 0.25, above_lower) * Ramp::smooth(0.75, 0.3, above_lower);
        c + Linear::new(0.5, 0.36, 0.34) * (wet * 0.10 * shadow.max(0.3))
    }

    fn iris(dx: f32, dy: f32, r: f32, eye: &Eye, t: &Tones, noise: &Noise, seed: f32) -> Linear {
        let ir = eye.iris_r;
        let rn = r / ir;
        let pupil = eye.pupil_r / ir;
        let ang = dy.atan2(dx);
        // Radial fibres: a handful of angular frequencies with this eye's
        // own phases, so the stroma never repeats and never seams
        let mut fib = 0.0;
        for (m, w) in [(7.0f32, 0.30f32), (13.0, 0.25), (23.0, 0.25), (41.0, 0.20)] {
            fib += w * (m * ang + seed * m * 0.37 + rn * 2.0 * (m * 0.11).sin()).sin();
        }
        let crypt = Ramp::smooth(0.35, 0.7, noise.at(dx * 1.4 + seed, dy * 1.4));
        let base = t.iris.saturate(-0.2);
        let light = base.saturate(0.25) * 1.9 + Linear::new(0.02, 0.015, 0.0);
        let dark = base.deepen(1.35) * 0.55;
        let mut c = base
            .mix(light, (0.5 + 0.5 * fib) * 0.55)
            .mix(dark, crypt * 0.5);
        // The collarette ring round the pupil, and the dark limbal ring at
        // the rim that makes an iris read as a lens rather than a spot
        let collar = pupil + (1.0 - pupil) * 0.28;
        c = c.mix(
            light.mix(Linear::new(0.20, 0.13, 0.05), 0.35),
            Ramp::smooth(0.14, 0.0, (rn - collar).abs()) * 0.45,
        );
        c = c * (1.0 - 0.72 * Ramp::smooth(0.74, 1.0, rn));
        let black = Linear::new(0.006, 0.005, 0.005);
        c.mix(black, Ramp::smooth(pupil + 0.05, pupil - 0.04, rn))
    }

    /// Lashes along both lids, laid over the shaded head: `(colour, cover)`.
    pub fn lashes(
        x: f32,
        y: f32,
        eye: &Eye,
        t: &Tones,
        noise: &Noise,
        seed: f32,
    ) -> Option<(Linear, f32)> {
        if !eye.upper.near(x, y, 4.0) {
            return None;
        }
        let top = eye.upper.foot(x, y);
        let up = -top.d * eye.side;
        let u = top.u;
        let mut cover = 0.0f32;
        if (-0.7..3.4).contains(&up) {
            // The root line thickens toward the outer corner; its top edge
            // is lashes, not a line
            let th = 0.30 + 0.28 * (u * (1.3 - u)).max(0.0);
            let ragged = 0.35 * noise.line(u * 23.0 + seed);
            let root = Ramp::smooth(-0.55, -0.1, up)
                * (1.0 - Ramp::smooth(th * 0.35, th * 1.25 + ragged, up));
            // Lashes sweep up and out, longest past the middle
            let len = 0.7 + 1.9 * u * (1.2 - u * 0.5);
            let s = u * eye.upper.len();
            let q = s - up * (0.2 + 1.0 * u);
            let strand = Ramp::smooth(0.05, 0.55, noise.line(q * 1.9 + seed));
            let lash =
                strand * (1.0 - Ramp::smooth(len * 0.25, len, up)) * Ramp::smooth(-0.2, 0.2, up);
            // The lash line ends at the corner, it does not flick past it
            cover = (root * 0.68).max(lash * 0.55)
                * Ramp::smooth(0.0, 0.12, u)
                * Ramp::smooth(0.5, 0.0, top.past);
        }
        let bottom = eye.lower.foot(x, y);
        let down = bottom.d * eye.side;
        if (-0.3..1.4).contains(&down) {
            let s = bottom.u * eye.lower.len();
            let strand = Ramp::smooth(0.1, 0.6, noise.line(s * 1.7 - seed));
            let lash =
                strand * Ramp::smooth(-0.2, 0.1, down) * (1.0 - Ramp::smooth(0.2, 1.1, down));
            cover = cover.max(lash * 0.40 * Ramp::smooth(0.15, 0.4, bottom.u));
        }
        (cover > 0.002).then_some((t.lash, cover))
    }

    /// Both ears: `(cover, depth, how far out along the ear)` — the ear
    /// hangs off the side of the skull and stands forward of the plane
    /// behind it, angled so its bowl faces the camera.
    pub fn ears(grid: &Grid, l: &Landmarks) -> (Plane, Plane) {
        let e = &l.ear;
        let (top, bottom) = (e.top, e.bottom);
        let mid = (top + bottom) / 2.0;
        let h = (bottom - top) / 2.0;
        let w = e.width;
        let anchor = l.half_width_at(mid);
        let mut cover = Plane::new(*grid, 0.0);
        let mut depth = Plane::new(*grid, 0.0);
        for side in [-1.0f32, 1.0] {
            // `f` is in ear widths out from the head's edge: 0 is the
            // silhouette, 1 the furthest the helix stands off it
            let off = |y: f32| l.half_width_at(y).max(anchor) + (e.out - 2.4);
            let o = |f: f32, y: f32| (l.cx + side * (off(y) + w * f), y);
            let r = |f: f32, y: f32| (l.edge_x(y, side) + side * w * f, y);
            let outline = Outline::smooth(
                &[
                    o(-0.04, top),
                    o(0.30, top + h * 0.18),
                    o(0.56, top + h * 0.46),
                    o(0.72, top + h * 0.82),
                    o(0.68, mid + h * 0.14),
                    o(0.58, mid + h * 0.46),
                    o(0.44, bottom - h * 0.34),
                    o(0.42, bottom - h * 0.16),
                    o(0.20, bottom),
                    r(-0.16, bottom - h * 0.10),
                    r(-0.34, mid + h * 0.30),
                    r(-0.36, top + h * 0.50),
                    r(-0.20, top + 2.0),
                ],
                0.25,
            );
            let run = |pts: &[(f32, f32)]| pts.to_vec();
            let forms = [
                // The concha, the bowl against the head
                Form::blob(
                    o(0.20, mid).0,
                    mid + h * 0.02,
                    w * 0.22,
                    h * 0.46,
                    0.0,
                    -3.6,
                ),
                Form::blob(
                    o(0.24, top + h * 0.72).0,
                    top + h * 0.72,
                    w * 0.18,
                    h * 0.20,
                    0.0,
                    -1.8,
                ),
                // Antihelix ridge, scapha groove, the rolled helix rim
                Form::crease(
                    &run(&[
                        o(0.18, top + h * 0.62),
                        o(0.38, mid - h * 0.12),
                        o(0.40, mid + h * 0.35),
                        o(0.27, mid + h * 0.78),
                    ]),
                    1.2,
                    1.2,
                    0.2,
                ),
                Form::crease(
                    &run(&[
                        o(0.24, top + h * 0.16),
                        o(0.50, top + h * 0.52),
                        o(0.52, mid + h * 0.06),
                        o(0.42, mid + h * 0.50),
                        o(0.28, bottom - h * 0.40),
                    ]),
                    1.1,
                    -1.2,
                    0.2,
                ),
                Form::crease(
                    &run(&[
                        o(0.15, top + 1.6),
                        o(0.50, top + h * 0.24),
                        o(0.64, top + h * 0.70),
                        o(0.59, mid + h * 0.16),
                        o(0.47, mid + h * 0.46),
                        o(0.33, bottom - h * 0.34),
                    ]),
                    1.3,
                    1.5,
                    0.15,
                ),
                // Tragus and lobe
                Form::blob(o(0.08, mid).0, mid - h * 0.04, w * 0.10, h * 0.17, 0.0, 1.4),
                Form::blob(
                    o(0.24, bottom - h * 0.26).0,
                    bottom - h * 0.26,
                    w * 0.22,
                    h * 0.26,
                    0.0,
                    1.1,
                ),
            ];
            let c = outline.coverage(grid);
            let (x0, y0, x1, y1) = outline.bounds();
            for j in grid.rows(y0 - 1.0, y1 + 1.0) {
                for i in grid.cols(x0 - 1.0, x1 + 1.0) {
                    let k = j * grid.w + i;
                    if c.v[k] <= 0.0 {
                        continue;
                    }
                    let (x, y) = (grid.x(i), grid.y(j));
                    let f = (side * (x - l.cx) - off(y)) / w;
                    let z = 12.0 * (1.0 - 0.85 * f.clamp(-0.5, 1.0))
                        + forms.iter().map(|m| m.at(x, y)).sum::<f32>();
                    cover.v[k] = cover.v[k].max(c.v[k]);
                    depth.v[k] = z.max(0.5);
                }
            }
        }
        (cover, depth)
    }

    /// The per-eye seed for strand and fibre noise.
    pub fn eye_seed(id: &Identity, side: f32) -> f32 {
        (id.seed as f32 * 0.37 + side * 17.0).rem_euclid(TAU * 40.0) + PI
    }
}
