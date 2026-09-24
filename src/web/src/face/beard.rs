//! Facial hair, from the shadow a razor leaves to a full beard.
//!
//! Two different things. What grows back after a shave lives IN the skin —
//! the blue-grey veil of roots under it and the dark specks of a few days'
//! growth — so it is handed to the complexion as pigment. A beard that is
//! kept is hair: it has volume over the lips and chin, it runs one way, and
//! it catches the light the way scalp hair does.

use super::canvas::{Grid, Outline, Path, Plane, Ramp};
use super::color::Linear;
use super::geometry::Landmarks;
use super::hair::HairMass;
use super::identity::{Beard, Identity, Moustache};
use super::noise::Noise;
use super::relief::Relief;
use super::tones::Tones;

/// What a razor leaves on this man's face.
pub struct Growth {
    /// How strongly the skin over the roots is veiled, 0..1
    pub veil: Plane,
    /// Cover of visible stubble, 0..1
    pub stubble: Plane,
    pub color: Linear,
}

pub struct FacialHair;

impl FacialHair {
    pub fn growth(
        grid: &Grid,
        l: &Landmarks,
        id: &Identity,
        t: &Tones,
        noise: &Noise,
        age: u8,
    ) -> Growth {
        let ph = id.phenotype;
        let (bn, bd) = ph.beard_mul();
        let thick = (bn as f32 / bd as f32).clamp(0.35, 1.3);
        let grown = match age {
            0..=16 => 0.0,
            17..=19 => 0.35,
            20..=23 => 0.7,
            _ => 1.0,
        } * thick;
        let sparse = ph.epicanthic() || ph == shared::Phenotype::Andean;
        let mat = l.maturity;
        // How many days since the razor, as a cover of dark specks
        let stubble_level = match id.beard {
            Some(Beard::Stubble) => 0.62 + mat * 0.18,
            Some(_) => 0.25,
            None if age >= 22 && !sparse => 0.14 + mat * 0.20,
            None => 0.0,
        };
        let region = Self::region(l, l.skull.zygo_y + 25.0, l.nose + 8.0);
        let dist = region.distance(grid);
        let lips = Self::lips(grid, l);
        let seed = (id.seed % 541) as f32 * 0.7;
        let area = dist.map(|k, d| {
            if d < -6.0 {
                return 0.0;
            }
            let (x, y) = (grid.x(k % grid.w), grid.y(k / grid.w));
            let ragged = 2.2 * noise.fbm(x * 0.3 + seed, y * 0.3, 2);
            Ramp::smooth(-2.0, 4.5, d + ragged) * (1.0 - lips.v[k])
        });
        let veil = area.map(|_, a| a * grown * 0.85);
        let stubble = area.map(|k, a| {
            if a <= 0.0 || stubble_level <= 0.0 {
                return 0.0;
            }
            let (x, y) = (grid.x(k % grid.w), grid.y(k / grid.w));
            // One cut hair to a follicle, a dark speck on the skin
            let (d, own) = noise.cells(x * 1.5 + seed, y * 1.5);
            let dots = Ramp::smooth(0.28 + 0.1 * own, 0.08, d);
            a * grown.min(1.0) * stubble_level * (0.25 + 0.75 * dots)
        });
        Growth {
            veil,
            stubble,
            color: t
                .hair_greyed(id.grey)
                .mix(Linear::new(0.02, 0.018, 0.016), 0.2),
        }
    }

    /// A kept beard and moustache, as hair laid over the lower face.
    pub fn kept(
        grid: &Grid,
        l: &Landmarks,
        relief: &Relief,
        id: &Identity,
        t: &Tones,
        noise: &Noise,
    ) -> Option<HairMass> {
        let cx = l.cx;
        let s = &l.skull;
        let ms = &l.mouth_shape;
        let (my, ny, mw) = (l.mouth, l.nose, ms.half);
        let sb = s.zygo_y + 22.0;
        // (region, volume, density, fuzz)
        let mut parts: Vec<(Outline, f32, f32, f32)> = Vec::new();
        match id.beard {
            Some(Beard::Boxed) => parts.push((Self::region(l, sb - 1.0, ny + 7.0), 1.8, 0.8, 2.2)),
            Some(Beard::Full) => parts.push((Self::region(l, sb - 6.0, ny + 4.0), 4.0, 1.0, 3.2)),
            Some(Beard::Goatee) => parts.push((Self::goatee(l), 2.0, 0.85, 1.8)),
            Some(Beard::Chinstrap) => parts.push((Self::chinstrap(l), 1.4, 0.9, 1.6)),
            Some(Beard::Stubble) | None => {}
        }
        // Every grown style but the chinstrap carries its own moustache
        let banded = matches!(
            id.beard,
            Some(Beard::Stubble | Beard::Boxed | Beard::Full | Beard::Goatee)
        );
        if let (Some(m), false) = (id.moustache, banded) {
            let (k, h, density) = match m {
                Moustache::Thin => (0.90, 2.4, 0.55),
                Moustache::Chevron => (1.02, 6.0, 0.95),
                Moustache::Handlebar => (1.20, 5.5, 0.9),
                Moustache::Walrus => (1.08, 7.5, 0.95),
            };
            let w = mw * k;
            let base = my - ms.upper - 0.4;
            let mut path =
                Path::from((cx - w, base)).quad((cx, my - ms.upper - h - 1.0), (cx + w, base));
            if m == Moustache::Handlebar {
                path = path
                    .quad((cx + w + 2.4, my + 1.0), (cx + w + 3.0, my + 3.5))
                    .line((cx + w, base + 1.2));
            }
            path = path.quad((cx, my - ms.upper - 1.6), (cx - w, base));
            if m == Moustache::Handlebar {
                path = path
                    .quad((cx - w - 2.4, my + 1.0), (cx - w - 3.0, my + 3.5))
                    .line((cx - w, base + 1.2));
            }
            parts.push((path.outline(), 1.2, density, 1.2));
        }
        if parts.is_empty() {
            return None;
        }

        let lips = Self::lips(grid, l);
        let seed = (id.seed % 389) as f32 * 0.9;
        let fields: Vec<(Plane, f32, f32, f32)> = parts
            .iter()
            .map(|(o, v, d, f)| (o.distance(grid), *v, *d, *f))
            .collect();
        let stream = |x: f32, y: f32| (x - cx) * (1.0 + 0.018 * (y - ny)).max(0.3);
        let tex = Plane::from_fn(*grid, |x, y| {
            let q = stream(x, y);
            0.5 + 0.28 * noise.at(q * 1.3 + seed, y * 0.35)
                + 0.24 * noise.fbm(x * 0.9, y * 0.9 - seed, 2)
        });
        let mut cover = Plane::new(*grid, 0.0);
        let mut volume = Plane::new(*grid, 0.0);
        for (k, c) in cover.v.iter_mut().enumerate() {
            if relief.cover.v[k] <= 0.0 {
                continue;
            }
            for (dist, vol, density, fuzz) in &fields {
                let d = dist.v[k];
                if d < -4.0 {
                    continue;
                }
                let edge = Ramp::smooth(-1.2, 2.4, d + (tex.v[k] - 0.5) * fuzz * 2.2);
                // Skin shows between the hairs, the more so the shorter they are
                let body = 0.2 + 0.8 * Ramp::smooth(0.25, 0.8, tex.v[k]);
                let short = (*vol / 6.0).min(1.0);
                let a = edge
                    * density
                    * relief.cover.v[k]
                    * (1.0 - lips.v[k])
                    * (body + (1.0 - body) * short);
                if a > *c {
                    *c = a;
                    volume.v[k] = vol * Ramp::smooth(-0.5, 4.0, d);
                }
            }
        }
        let z = relief.z.blurred(0.8).map(|k, zs| {
            if cover.v[k] <= 0.0 {
                0.0
            } else {
                zs + 0.3 + volume.v[k] * (0.6 + 0.5 * tex.v[k])
            }
        });
        let dir = grid.map(|_, _, x, y| {
            let e = 0.3;
            let gx = stream(x + e, y) - stream(x - e, y);
            let gy = stream(x, y + e) - stream(x, y - e);
            let len = (gx * gx + gy * gy).sqrt().max(1e-6);
            (-gy / len, gx / len)
        });
        let base = t.hair_greyed(id.grey);
        let albedo = grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return Linear::BLACK;
            }
            // A beard greys before the hair does
            let white = Ramp::smooth(0.5, 0.75, 0.5 + 0.5 * noise.at(x * 1.5 + 90.0, y * 0.5))
                * (id.grey * 2.0).min(1.0);
            base.mix(t.grey_hair, white) * (0.6 + 0.75 * tex.v[k])
        });
        Some(HairMass {
            cover,
            z,
            dir,
            tex,
            albedo,
            gloss: 0.35,
            bump: 1.4,
        })
    }

    /// The lips, which no beard covers, a little enlarged.
    fn lips(grid: &Grid, l: &Landmarks) -> Plane {
        let upper = l.lips.upper.distance(grid);
        let lower = l.lips.lower.distance(grid);
        upper.map(|k, u| Ramp::smooth(-1.0, 0.3, u.max(lower.v[k])))
    }

    /// The lower face every full style shares, drawn out past the jaw: the
    /// head's own silhouette decides where it stops. `sb` is where it meets
    /// the sideburns, `mst` the top of the moustache.
    fn region(l: &Landmarks, sb: f32, mst: f32) -> Outline {
        let cx = l.cx;
        let s = &l.skull;
        let mw = l.mouth_shape.half;
        let my = l.mouth;
        let (bx_l, bx_r, bx_b) = (cx - s.sub - 14.0, cx + s.sub + 14.0, s.chin + 16.0);
        let (gate_l, gate_r, gate_y) = (cx - mw - 3.0, cx + mw + 3.0, my - 3.0);
        let cheek_c = sb + (gate_y - sb) * 0.5;
        let (in_l, in_r) = (bx_l + 10.0, bx_r - 10.0);
        let (out_l, out_r) = (gate_l - 13.0, gate_r + 13.0);
        // The moustache droops out to the corners and dips at the philtrum
        let (mo_top, mo_mid, mo_drop) = (mst + 0.5, mst + 4.5, mst + 6.0);
        Path::from((bx_l, sb))
            .cubic((bx_l, s.jaw_y), (bx_l, bx_b), (cx, bx_b))
            .cubic((bx_r, bx_b), (bx_r, s.jaw_y), (bx_r, sb))
            .cubic((in_r, cheek_c), (out_r, gate_y), (gate_r, gate_y))
            .cubic(
                (gate_r, mo_drop),
                (cx + mw * 0.72, mst),
                (cx + mw * 0.45, mo_top),
            )
            .quad((cx, mo_mid), (cx - mw * 0.45, mo_top))
            .cubic((cx - mw * 0.72, mst), (gate_l, mo_drop), (gate_l, gate_y))
            .cubic((out_l, gate_y), (in_l, cheek_c), (bx_l, sb))
            .outline()
    }

    /// Moustache wrapped round the corners of the mouth into a chin patch;
    /// the cheeks stay clean.
    fn goatee(l: &Landmarks) -> Outline {
        let cx = l.cx;
        let mw = l.mouth_shape.half;
        let (g_l, g_r) = (cx - mw - 4.0, cx + mw + 4.0);
        let g_top = l.nose + 6.0;
        let g_bot = l.skull.chin + 1.0;
        let g_side = l.mouth + 2.0;
        let g_mid = g_side + (g_bot - g_side) * 0.55;
        Path::from((g_l, g_side))
            .cubic(
                (g_l, g_top + 7.0),
                (cx - mw * 0.78, g_top),
                (cx - mw * 0.42, g_top + 0.5),
            )
            .quad((cx, g_top + 5.0), (cx + mw * 0.42, g_top + 0.5))
            .cubic((cx + mw * 0.78, g_top), (g_r, g_top + 7.0), (g_r, g_side))
            .cubic(
                (cx + mw * 0.95, g_mid),
                (cx + mw * 0.62, g_bot),
                (cx, g_bot),
            )
            .cubic(
                (cx - mw * 0.62, g_bot),
                (cx - mw * 0.95, g_mid),
                (g_l, g_side),
            )
            .outline()
    }

    /// A band along the jaw from sideburn to sideburn.
    fn chinstrap(l: &Landmarks) -> Outline {
        let cx = l.cx;
        let s = &l.skull;
        let run = |inset: f32| {
            Path::from((cx - s.sub - 14.0 + inset, s.zygo_y + 20.0))
                .cubic(
                    (cx - s.jaw - 6.0 + inset, s.jaw_y),
                    (cx - s.chin_half - 4.0 + inset * 0.5, s.chin + 2.0 - inset),
                    (cx, s.chin + 3.0 - inset),
                )
                .cubic(
                    (cx + s.chin_half + 4.0 - inset * 0.5, s.chin + 2.0 - inset),
                    (cx + s.jaw + 6.0 - inset, s.jaw_y),
                    (cx + s.sub + 14.0 - inset, s.zygo_y + 20.0),
                )
                .points()
        };
        let mut pts = run(-3.0);
        pts.extend(run(7.0).into_iter().rev());
        Outline::polygon(pts)
    }
}
