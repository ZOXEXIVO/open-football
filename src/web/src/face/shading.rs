//! The studio, and what its light does to skin, eyes, hair and cloth.
//!
//! One key light stands high on the camera's left, as in every club head
//! shot the site serves; a broad fill lifts the far side, a rim light behind
//! picks the edge of the head out of the card, and a hair light from above
//! puts the sheen on the crown. Directions are page-space unit vectors
//! pointing TOWARD the lamp: x right, y down the page, z toward the lens.
//!
//! Shadows and occlusion are read off one depth map of everything in the
//! picture, so the nose shades the cheek, the jaw the neck and the hair the
//! forehead by the same rule. Skin gets its softness from two things real
//! skin does: light wraps past the terminator further in red than in blue,
//! and it scatters under the surface before it comes back out — the second
//! is a blur of the light arriving, never of the pigment, so pores and
//! stubble stay sharp while the shading goes soft.

use super::canvas::{Layer, Plane};
use super::color::Linear;

#[derive(Clone, Copy)]
pub struct Lamp {
    pub dir: [f32; 3],
    pub color: Linear,
}

pub struct Studio {
    pub key: Lamp,
    pub fill: Lamp,
    pub rim: Lamp,
    pub top: Lamp,
    sky: Linear,
    ground: Linear,
}

impl Studio {
    pub fn portrait() -> Studio {
        Studio {
            key: Lamp {
                dir: Vec3::normalized([-0.48, -0.48, 0.73]),
                color: Linear::new(1.34, 1.32, 1.29),
            },
            fill: Lamp {
                dir: Vec3::normalized([0.72, -0.05, 0.69]),
                color: Linear::new(0.11, 0.12, 0.13),
            },
            rim: Lamp {
                dir: Vec3::normalized([0.86, -0.30, -0.42]),
                color: Linear::new(0.45, 0.46, 0.48),
            },
            top: Lamp {
                dir: Vec3::normalized([-0.15, -0.90, -0.40]),
                color: Linear::new(0.25, 0.25, 0.26),
            },
            sky: Linear::new(0.12, 0.12, 0.13),
            ground: Linear::new(0.04, 0.035, 0.03),
        }
    }

    /// Light arriving at a skin point, before its pigment: key and fill
    /// wrapped per channel, the rim along the edges, the room's ambient.
    pub fn skin_light(&self, n: [f32; 3], key_shadow: f32, ao: f32) -> Linear {
        // Red travels furthest under the skin, so it wraps furthest past
        // the terminator: a shadow edge on skin is warm, never grey
        const WRAP: [f32; 3] = [0.16, 0.11, 0.08];
        let wrapped = |lamp: &Lamp, vis: f32| {
            let d = Vec3::dot(n, lamp.dir);
            let w = |c: usize| ((d + WRAP[c]) / (1.0 + WRAP[c])).max(0.0);
            // The penumbra itself scatters: red leaks furthest into it
            let v = |c: usize| (vis + (1.0 - vis) * WRAP[c] * 0.3).min(1.0);
            Linear::new(
                lamp.color.r * w(0) * v(0),
                lamp.color.g * w(1) * v(1),
                lamp.color.b * w(2) * v(2),
            )
        };
        // The fill is a broad bounce and the room is everywhere: both are
        // crowded out of hollows just as the ambient is. And no light at
        // all reaches the bottom of a crease as well as it reaches a plane
        let mut e = wrapped(&self.key, key_shadow) + wrapped(&self.fill, 1.0) * ao;
        let rim = Vec3::dot(n, self.rim.dir).max(0.0);
        e += self.rim.color * (rim * rim * key_shadow.max(0.4));
        e += self.ambient(n) * ao;
        e * (0.4 + 0.6 * ao)
    }

    /// A skin surface, lit: key-light shadows off the depth map, light
    /// scattered under the skin by a blur of the irradiance — widest in red
    /// — and the sheen of its oils on top. `scatter` weights which pixels
    /// share their light (the eye openings take none of the lid's).
    #[allow(clippy::too_many_arguments)]
    pub fn skin(
        &self,
        occ: &Occlusion,
        cover: &Plane,
        z: &Plane,
        albedo: &[Linear],
        oil: &[f32],
        micro: Option<&Plane>,
        scatter: &Plane,
    ) -> Layer {
        let grid = cover.grid;
        let lit: Vec<(Linear, Linear)> = grid.map(|i, j, x, y| {
            let k = j * grid.w + i;
            if cover.v[k] <= 0.0 {
                return (Linear::BLACK, Linear::BLACK);
            }
            let depth = z.v[k];
            let (dx, dy) = z.slope(i, j);
            let (mx, my) = micro.map(|m| m.slope(i, j)).unwrap_or((0.0, 0.0));
            // The skin's grain shows mostly in its sheen, a little in its
            // shading
            let n = Vec3::facing(dx + mx * 0.6, dy + my * 0.6);
            let shadow = occ.shadow(x, y, depth, self.key.dir, 0.32);
            let ao = occ.ambient(x, y, depth);
            let e = self.skin_light(n, shadow, ao);
            let n_spec = Vec3::facing(dx + mx, dy + my);
            let oil = oil[k];
            // Two lobes, as skin has: a broad sheen off the skin itself and
            // a tight one off the oil on it, which the pores break up
            let s = self.specular(n_spec, 0.5 - 0.1 * oil, 0.028, shadow) * (0.35 + 0.6 * oil)
                + self.specular(n_spec, 0.2, 0.028, shadow) * (0.9 * oil * oil);
            (e, s * ao.sqrt())
        });
        const REACH: [f32; 3] = [0.9, 0.6, 0.45];
        let weight: Vec<f32> = (0..grid.len())
            .map(|k| scatter.v[k] * cover.v[k].min(1.0))
            .collect();
        let spread = |c: usize, sigma: f32| -> Plane {
            let channel = |l: Linear| [l.r, l.g, l.b][c];
            let own = Plane {
                grid,
                v: lit.iter().map(|(e, _)| channel(*e)).collect(),
            };
            let weighted = own.map(|k, v| v * weight[k]).blurred(sigma);
            let norm = Plane {
                grid,
                v: weight.clone(),
            }
            .blurred(sigma);
            own.map(|k, v| {
                if norm.v[k] > 1e-3 && weight[k] > 0.0 {
                    v + (weighted.v[k] / norm.v[k] - v) * 0.45
                } else {
                    v
                }
            })
        };
        let [r, g, b] = [0, 1, 2].map(|c| spread(c, REACH[c]));
        Layer::shade(&grid, |i, j, _, _| {
            let k = j * grid.w + i;
            let a = cover.v[k];
            (a > 0.0).then(|| {
                (
                    albedo[k] * Linear::new(r.v[k], g.v[k], b.v[k]) + lit[k].1,
                    a,
                )
            })
        })
    }

    /// The room: brighter from above, a little bounce from below and from
    /// the camera side.
    pub fn ambient(&self, n: [f32; 3]) -> Linear {
        let up = 0.5 - 0.5 * n[1];
        self.ground.mix(self.sky, up) * (0.55 + 0.45 * n[2].max(0.0))
    }

    /// Specular reflection of every lamp off a surface of the given
    /// roughness; `f0` is its reflectance head-on (skin ≈ 0.028).
    pub fn specular(&self, n: [f32; 3], rough: f32, f0: f32, key_shadow: f32) -> Linear {
        let mut s = Linear::BLACK;
        for (lamp, vis) in [
            (&self.key, key_shadow),
            (&self.fill, 1.0),
            (&self.rim, key_shadow.max(0.3)),
        ] {
            s += lamp.color * (Self::ggx(n, lamp.dir, rough, f0) * vis);
        }
        s
    }

    /// GGX with Schlick's Fresnel and the Kelemen visibility term.
    fn ggx(n: [f32; 3], l: [f32; 3], rough: f32, f0: f32) -> f32 {
        let nl = Vec3::dot(n, l);
        if nl <= 0.0 {
            return 0.0;
        }
        let h = Vec3::normalized([l[0], l[1], l[2] + 1.0]);
        let nh = Vec3::dot(n, h).max(0.0);
        let vh = h[2].max(1e-3);
        let a2 = (rough * rough).powi(2);
        let denom = nh * nh * (a2 - 1.0) + 1.0;
        let d = a2 / (std::f32::consts::PI * denom * denom);
        let f = f0 + (1.0 - f0) * (1.0 - vh).powi(5);
        d * f * nl * 0.25 / (vh * vh)
    }

    /// Hair: Kajiya–Kay, with the two highlights real hair has — one off
    /// the surface of the strand in the light's colour, and one that has
    /// passed through it and come back tinted by the pigment, shifted
    /// toward the tip.
    #[allow(clippy::too_many_arguments)]
    pub fn hair(
        &self,
        n: [f32; 3],
        t: [f32; 3],
        albedo: Linear,
        shift: f32,
        gloss: f32,
        key_shadow: f32,
        ao: f32,
    ) -> Linear {
        let mut c = Linear::BLACK;
        let t1 = Vec3::normalized(Vec3::add(t, Vec3::scale(n, shift - 0.08)));
        let t2 = Vec3::normalized(Vec3::add(t, Vec3::scale(n, shift + 0.12)));
        for (lamp, vis) in [
            (&self.key, key_shadow),
            (&self.fill, 1.0),
            (&self.rim, key_shadow.max(0.35)),
            (&self.top, key_shadow.max(0.5)),
        ] {
            let nl = Vec3::dot(n, lamp.dir);
            let diffuse = ((nl + 0.35) / 1.35).max(0.0);
            let h = Vec3::normalized([lamp.dir[0], lamp.dir[1], lamp.dir[2] + 1.0]);
            let s1 = Self::strand(t1, h).powf(150.0 * gloss + 60.0);
            let s2 = Self::strand(t2, h).powf(40.0 * gloss + 20.0);
            // Light from behind comes through the fringe rather than off it
            let through = (-nl).max(0.0) * (1.0 - n[2].max(0.0)) * 0.6;
            let lit = albedo * (diffuse + through)
                + Linear::gray(s1 * 0.06 * gloss)
                + albedo.deepen(0.8) * (s2 * 0.25 * gloss);
            c += lit * lamp.color * vis;
        }
        c + albedo * self.ambient(n) * ao
    }

    fn strand(t: [f32; 3], h: [f32; 3]) -> f32 {
        let th = Vec3::dot(t, h);
        (1.0 - th * th).max(0.0).sqrt()
    }
}

/// Everything standing in the picture, as one depth map: what casts
/// shadows and what crowds the light out of hollows.
pub struct Occlusion {
    depth: Plane,
    top: f32,
}

impl Occlusion {
    /// How far round a point the horizon is searched, and at what steps.
    const REACH: [f32; 7] = [0.8, 1.6, 3.0, 5.0, 8.0, 12.0, 17.0];

    pub fn new(depth: Plane) -> Occlusion {
        let top = depth.v.iter().copied().fold(0.0, f32::max);
        Occlusion { depth, top }
    }

    /// How open a point at depth `z` is to the room, 0..1: the share of the
    /// sky above it not hidden behind the horizon, found by looking out in
    /// eight directions for how high the surface round it rises. A tall
    /// thin thing — a nose — hides only the slice of sky it stands in.
    pub fn ambient(&self, x: f32, y: f32, z: f32) -> f32 {
        let turn =
            ((x * 91.345 + y * 47.853).sin() * 23_421.63).fract() * std::f32::consts::FRAC_PI_4;
        let mut hidden = 0.0;
        for d in 0..8 {
            let a = turn + d as f32 * std::f32::consts::FRAC_PI_4;
            let (s, c) = a.sin_cos();
            let mut rise = 0.0f32;
            for r in Self::REACH {
                let (sx, sy) = (x + c * r, y + s * r);
                if !self.depth.grid.holds(sx, sy) {
                    break;
                }
                // Far walls count for less: the room's light comes in over
                // them from beyond
                let slope = (self.depth.sample(sx, sy) - z - 0.2) / r;
                rise = rise.max(slope * (1.0 - r / 22.0));
            }
            hidden += rise / (1.0 + rise * rise).sqrt();
        }
        (1.0 - hidden / 8.0 * 1.1).clamp(0.0, 1.0)
    }

    /// How much of a lamp reaches a point. The lamp is a soft box `spread`
    /// radians across, so the answer is the share of it the point can see:
    /// rays to its centre and round its rim, each marched over the depth
    /// map, averaged — a penumbra with the true shape of the occluder
    /// rather than a guess at it.
    pub fn shadow(&self, x: f32, y: f32, z: f32, lamp: [f32; 3], spread: f32) -> f32 {
        let u = Vec3::normalized([lamp[2], 0.0, -lamp[0]]);
        let v = [
            lamp[1] * u[2] - lamp[2] * u[1],
            lamp[2] * u[0] - lamp[0] * u[2],
            lamp[0] * u[1] - lamp[1] * u[0],
        ];
        // The ring is turned a little from pixel to pixel so its seven
        // samples never band
        let turn = ((x * 12.9898 + y * 78.233).sin() * 43_758.547).fract() * std::f32::consts::TAU;
        let mut seen = self.ray(x, y, z, lamp);
        for k in 0..6 {
            let a = turn + k as f32 * std::f32::consts::TAU / 6.0;
            let r = spread * if k % 2 == 0 { 0.55 } else { 0.95 };
            let (s, c) = a.sin_cos();
            let dir = Vec3::normalized(Vec3::add(
                lamp,
                Vec3::add(Vec3::scale(u, c * r), Vec3::scale(v, s * r)),
            ));
            seen += self.ray(x, y, z, dir);
        }
        seen / 7.0
    }

    /// One ray toward a direction: 1 if nothing stands in its way.
    fn ray(&self, x: f32, y: f32, z: f32, dir: [f32; 3]) -> f32 {
        let across = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-4);
        let (dx, dy) = (dir[0] / across, dir[1] / across);
        let rise = dir[2] / across;
        let mut vis = 1.0f32;
        let mut t = 0.5;
        for _ in 0..48 {
            let (sx, sy) = (x + dx * t, y + dy * t);
            let zr = z + 0.3 + t * rise;
            if zr > self.top || !self.depth.grid.holds(sx, sy) {
                break;
            }
            let blocked = (self.depth.sample(sx, sy) - zr) / (0.06 * t + 0.4);
            vis = vis.min(1.0 - blocked.clamp(0.0, 1.0));
            if vis <= 0.0 {
                return 0.0;
            }
            t *= 1.1;
        }
        vis
    }
}

/// The few vector operations shading needs, on plain arrays.
pub struct Vec3;

impl Vec3 {
    pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    pub fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
        [a[0] * k, a[1] * k, a[2] * k]
    }

    pub fn normalized(a: [f32; 3]) -> [f32; 3] {
        let len = Self::dot(a, a).sqrt().max(1e-9);
        [a[0] / len, a[1] / len, a[2] / len]
    }

    /// The surface normal of a depth map with slope `(dx, dy)`.
    pub fn facing(dx: f32, dy: f32) -> [f32; 3] {
        Self::normalized([-dx, -dy, 1.0])
    }
}
