//! The shape of the head as depth: how far each point of the skin stands
//! toward the camera.
//!
//! Page units throughout — x right, y down, z toward the lens, with z = 0
//! on the plane of the widest outline. The skull is a [`Volume`] fitted to
//! the silhouette the landmarks drew, so outline and solid cannot disagree.
//! The face is then modelled onto it in the order a sculptor adds clay:
//! brow ridge and sockets, cheekbones and the hollows under them, the
//! muzzle the teeth push forward, the nose, the lips, the chin, and last
//! the lids round two eyeballs.

use super::canvas::{Foot, Grid, Outline, Path, Plane, Polyline, Ramp};
use super::geometry::{Eye, Landmarks};
use super::identity::Identity;
use super::noise::Noise;

/// A rounded solid raised on an outline: a superellipse across each row,
/// rolled over at the top and tucked under at the bottom. Heads, necks,
/// hair and shoulders are all one of these before they are anything else.
pub struct Volume;

impl Volume {
    /// `depth` is the solid's height on its mid-line; `top` and `bottom` the
    /// radii it rolls away over at its upper and lower edges; `exponent` the
    /// cross-section by height — 2 is round, higher is boxier.
    pub fn raise(
        grid: &Grid,
        outline: &Outline,
        depth: f32,
        (top, bottom): (f32, f32),
        exponent: impl Fn(f32) -> f32 + Sync,
    ) -> Plane {
        let rows: Vec<Option<(f32, f32)>> =
            (0..grid.h).map(|j| outline.row_span(grid.y(j))).collect();
        let cols: Vec<Option<(f32, f32)>> =
            (0..grid.w).map(|i| outline.col_span(grid.x(i))).collect();
        Plane::from_pixels(*grid, |i, j| {
            let (Some((xl, xr)), Some((yt, yb))) = (rows[j], cols[i]) else {
                return 0.0;
            };
            let (x, y) = (grid.x(i), grid.y(j));
            let half = (xr - xl) / 2.0;
            let u = ((x - (xl + xr) / 2.0) / half.max(1e-3)).abs();
            if u >= 1.0 {
                return 0.0;
            }
            let p = exponent(y);
            let across = (1.0 - u.powf(p)).powf(1.0 / p);
            depth * across * Self::roll((y - yt) / top) * Self::roll((yb - y) / bottom)
        })
    }

    /// A quarter circle from the edge (0) to full height (1).
    fn roll(v: f32) -> f32 {
        if v >= 1.0 {
            1.0
        } else if v <= 0.0 {
            0.0
        } else {
            (1.0 - (1.0 - v) * (1.0 - v)).sqrt()
        }
    }
}

/// One soft form added to, or cut from, a surface.
pub enum Form {
    /// (1 − r²)³ inside a rotated ellipse: full at the centre, feathered to
    /// nothing at the rim with no ring for a highlight to catch on.
    Blob {
        x: f32,
        y: f32,
        rx: f32,
        ry: f32,
        cos: f32,
        sin: f32,
        h: f32,
    },
    /// The same profile across a curve: a crease, a fold, a ridge.
    Crease {
        line: Polyline,
        w: f32,
        h: f32,
        /// Fade in and out over this share of the length at each end
        taper: f32,
    },
}

impl Form {
    pub fn blob(x: f32, y: f32, rx: f32, ry: f32, rot_deg: f32, h: f32) -> Form {
        let (sin, cos) = rot_deg.to_radians().sin_cos();
        Form::Blob {
            x,
            y,
            rx: rx.max(0.1),
            ry: ry.max(0.1),
            cos,
            sin,
            h,
        }
    }

    /// A crease through `pts`, as a smooth run rather than a polygon:
    /// creases are read by their highlight, which shows every corner.
    pub fn crease(pts: &[(f32, f32)], w: f32, h: f32, taper: f32) -> Form {
        Form::Crease {
            line: Path::through(pts, 0.0).polyline(),
            w,
            h,
            taper,
        }
    }

    pub fn along(line: Polyline, w: f32, h: f32, taper: f32) -> Form {
        Form::Crease { line, w, h, taper }
    }

    pub fn at(&self, px: f32, py: f32) -> f32 {
        match self {
            Form::Blob {
                x,
                y,
                rx,
                ry,
                cos,
                sin,
                h,
            } => {
                let (dx, dy) = (px - x, py - y);
                if dx.abs() > rx.max(*ry) || dy.abs() > rx.max(*ry) {
                    return 0.0;
                }
                let u = (dx * cos + dy * sin) / rx;
                let v = (-dx * sin + dy * cos) / ry;
                let k = 1.0 - (u * u + v * v);
                if k <= 0.0 { 0.0 } else { h * k * k * k }
            }
            Form::Crease { line, w, h, taper } => {
                if !line.near(px, py, *w) {
                    return 0.0;
                }
                let foot = line.foot(px, py);
                let k = 1.0 - (foot.d / w) * (foot.d / w);
                if k <= 0.0 {
                    return 0.0;
                }
                let ends = if *taper > 0.0 {
                    let a = (foot.u / taper).min(1.0);
                    let b = ((1.0 - foot.u) / taper).min(1.0);
                    a * a * (3.0 - 2.0 * a) * b * b * (3.0 - 2.0 * b)
                } else {
                    1.0
                };
                h * k * k * k * ends
            }
        }
    }
}

/// The ball of one eye: a sphere, for the way light falls across it.
#[derive(Clone, Copy, Debug)]
pub struct Eyeball {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

impl Eyeball {
    pub fn normal(&self, px: f32, py: f32) -> [f32; 3] {
        let (dx, dy) = ((px - self.x) / self.r, (py - self.y) / self.r);
        let dz = (1.0 - dx * dx - dy * dy).max(0.0).sqrt();
        [dx, dy, dz]
    }
}

/// The nose, root to nostrils: a saddle between the eyes, a flat-topped
/// bridge whose side walls fall away into the cheeks, a tip of two domes,
/// the wings either side of it and the nostrils tucked under it. Its parts
/// meet in fillets rather than being summed, so the ridge never ripples
/// where one part hands over to the next.
struct Nose {
    /// Mid-line at the root and at the tip — they differ when the head turns
    x_root: f32,
    x_tip: f32,
    root: f32,
    tip: f32,
    base: f32,
    /// How far the tip stands off the face
    project: f32,
    bridge: f32,
    ball: f32,
    hump: f32,
    wing: f32,
    domes: [Form; 2],
    wings: [Form; 2],
    grooves: [Form; 2],
    nostrils: [Form; 2],
    columella: Form,
}

impl Nose {
    fn new(l: &Landmarks, id: &Identity, x_tip: f32) -> Nose {
        let ns = &l.nose_shape;
        let base = l.nose;
        let tip = base - ns.ball * 0.9;
        let project = 17.0 + id.morph.nose_len * 1.2 + l.maturity * 1.5;
        let at = |side: f32, k: f32| x_tip + side * ns.tip * k;
        Nose {
            x_root: l.cx,
            x_tip,
            root: l.eye - 4.0,
            tip,
            base,
            project,
            bridge: ns.bridge,
            ball: ns.ball,
            hump: ns.hump,
            wing: ns.tip,
            domes: [-1.0f32, 1.0].map(|side| {
                Form::blob(
                    x_tip + side * ns.ball * 0.38,
                    tip + 0.3,
                    ns.ball * 0.95,
                    ns.ball * 1.05,
                    0.0,
                    project,
                )
            }),
            wings: [-1.0f32, 1.0].map(|side| {
                Form::blob(
                    at(side, 0.66),
                    base - 3.0,
                    ns.tip * 0.42 + 1.8,
                    5.0,
                    -side * 15.0,
                    project * 0.46,
                )
            }),
            grooves: [-1.0f32, 1.0].map(|side| {
                Form::crease(
                    &[
                        (at(side, 0.45), base - 8.5),
                        (x_tip + side * (ns.tip * 1.05 + 1.0), base - 3.5),
                        (at(side, 0.95), base + 0.8),
                        (at(side, 0.65), base + 2.2),
                    ],
                    1.9,
                    -1.0,
                    0.1,
                )
            }),
            nostrils: [-1.0f32, 1.0].map(|side| {
                Form::blob(
                    at(side, 0.38),
                    base + 0.9,
                    ns.nostril * 0.9,
                    1.4,
                    -side * 25.0,
                    -project * 0.2,
                )
            }),
            columella: Form::blob(x_tip, base - 0.2, 2.0, 3.4, 0.0, project * 0.55),
        }
    }

    fn at(&self, x: f32, y: f32) -> f32 {
        if y < self.root - 12.0 || y > self.base + 6.0 || (x - self.x_tip).abs() > self.wing + 16.0
        {
            return 0.0;
        }
        // The bridge: rising from the saddle to the supratip, fairly
        // straight, a hump where the bone ends on an aquiline nose
        let supra = self.tip - self.ball * 0.6;
        let t = ((y - self.root) / (supra - self.root)).clamp(0.0, 1.0);
        let xc = self.x_root + (self.x_tip - self.x_root) * t;
        let hump = self.hump * 1.8 * (std::f32::consts::PI * (t * 1.4).min(1.0)).sin();
        let h = 2.6 + (self.project * 0.78 - 2.6) * t.powf(1.05) + hump.max(-0.6);
        let half = self.bridge * (1.2 + 0.15 * t) + self.ball * 0.25 * t * t;
        // A flat top, then side walls falling into the cheeks
        let r = (x - xc).abs() / half;
        let across = Ramp::smooth(1.5, 0.3, r).powf(1.3);
        let top = Ramp::smooth(self.root - 10.0, self.root + 1.5, y);
        let under = ((y - supra) / (self.base + 1.5 - supra)).clamp(0.0, 1.0);
        let dorsum = h * across * top * (1.0 - under * under);

        // A p-norm union: exactly the larger part where only one is
        // present, a fillet where two overlap
        let parts = [
            dorsum,
            self.domes[0].at(x, y),
            self.domes[1].at(x, y),
            self.columella.at(x, y),
            self.wings[0].at(x, y),
            self.wings[1].at(x, y),
        ];
        let z = parts
            .iter()
            .map(|p| p.max(0.0).powi(4))
            .sum::<f32>()
            .powf(0.25);
        z + self
            .grooves
            .iter()
            .chain(&self.nostrils)
            .map(|f| f.at(x, y))
            .sum::<f32>()
    }
}

/// The lips in profile, swept across the mouth: skin rising from under
/// the nose to the vermilion border, the upper lip turning down and back
/// into the line where they meet, the lower lip turning up to the light
/// and then under into the hollow above the chin.
struct Mouth {
    x: f32,
    half: f32,
    /// Top of the upper lip at the centre, and the dip of its bow
    border: f32,
    bow: f32,
    /// The line between the lips at the centre, and how far its corners
    /// sit below it
    line: f32,
    corner_dy: f32,
    subnasale: f32,
    /// Fullest point of the lower lip, and where it has turned under
    swell: f32,
    under: f32,
    upper_h: f32,
    lower_h: f32,
}

impl Mouth {
    /// How far the lips fall back from the border to the line.
    const TUCK: f32 = 2.4;
    /// How deep the hollow under the lower lip is.
    const SULCUS: f32 = 1.3;

    fn new(l: &Landmarks, id: &Identity, x: f32) -> Mouth {
        let ms = &l.mouth_shape;
        let lip = 1.0 + id.morph.lip * 0.18;
        let lower = 0.3 + 1.075 * ms.lower;
        Mouth {
            x,
            half: ms.half,
            border: l.mouth - ms.upper,
            bow: ms.bow,
            line: l.mouth + 0.6,
            corner_dy: l.lips.corner_dy,
            subnasale: l.nose + 2.5,
            swell: l.mouth + lower * 0.45,
            under: l.mouth + lower + 1.2,
            upper_h: 3.6 * lip * (ms.upper / 3.6).sqrt(),
            lower_h: 3.9 * lip * (ms.lower / 5.4).powf(0.7) + 0.4,
        }
    }

    fn at(&self, x: f32, y: f32) -> f32 {
        let t = (x - self.x) / self.half;
        if t.abs() >= 1.25 || y < self.subnasale - 4.0 || y > self.under + 8.0 {
            return 0.0;
        }
        let dx = x - self.x;
        // The border climbs to the corners and dips at the bow; the line
        // follows the corners up or down
        let border = self.border
            + self.bow * (-(dx / 2.0).powi(2)).exp() * 0.8
            + (self.line - self.border) * t.abs().min(1.0).powf(2.2);
        let line = self.line + self.corner_dy * t * t;
        let swell = self.swell + (line - self.line);
        let under = self.under + (line - self.line) * 0.5;
        let (u, lo) = (self.upper_h, self.lower_h);
        // The skin under the nose lies back; only near its border does the
        // lip start to stand out
        let z = if y < self.subnasale {
            0.0
        } else if y < border {
            let s = ((y - self.subnasale) / (border - self.subnasale).max(0.5)).clamp(0.0, 1.0);
            u * s.powf(2.4)
        } else if y < line {
            let v = ((y - border) / (line - border).max(0.3)).clamp(0.0, 1.0);
            u - Self::TUCK * v.powf(1.8)
        } else if y < swell {
            let w = ((y - line) / (swell - line).max(0.3)).clamp(0.0, 1.0);
            let from = u - Self::TUCK;
            from + (lo - from) * (1.0 - (1.0 - w) * (1.0 - w))
        } else if y < under {
            let q = ((y - swell) / (under - swell).max(0.3)).clamp(0.0, 1.0);
            lo - (lo + Self::SULCUS) * q * q
        } else {
            -Self::SULCUS * (1.0 - Ramp::smooth(under, under + 8.0, y))
        };
        // Full in the middle, thinning into the corners; above the lip the
        // skin narrows up to the philtrum rather than standing out as a
        // plate the width of the mouth
        let reach = if y < border {
            let s = ((border - y) / (border - self.subnasale).max(0.5)).clamp(0.0, 1.0);
            1.25 - 0.45 * s
        } else {
            1.25
        };
        // Squared, so the lips meet the cheek with no crease at the edge
        let taper = (1.0 - (t.abs() / reach).powi(2)).max(0.0).powi(2);
        z * taper
    }
}

/// One eye's lids lying over its ball: a band of skin from the lash line up
/// to the fold that follows the ball's curve, a narrower band below, and the
/// bare ball between them. Only where there are lids does the ball show
/// through the skin — wrapped all the way round it, it reads as a goggle.
struct Lids<'a> {
    eye: &'a Eye,
    ball: Eyeball,
    /// Depth of the ball's centre
    zc: f32,
    /// How far the upper lid runs above its margin before it folds in
    fold: f32,
    /// Upper lid thickness — heavier on a hooded eye
    upper: f32,
}

impl<'a> Lids<'a> {
    const LOWER: f32 = 0.5;

    fn new(eye: &'a Eye, ball: Eyeball, skull: &Plane, l: &Landmarks, id: &Identity) -> Lids<'a> {
        let es = &l.eye_shape;
        // The cornea sits a little behind where the skull's surface would
        // be: the brow ridge overhangs it, the cheek lies below it
        let front = skull.sample(eye.cx, eye.cy) - 2.5;
        let fold = if eye.crease.is_some() {
            3.0 + es.lid_extra * 0.4 + id.morph.lid_heavy * 0.6
        } else {
            2.2
        };
        Lids {
            eye,
            ball,
            zc: front - ball.r,
            fold,
            upper: 1.0 + id.morph.lid_heavy * 0.3 + es.lid_extra * 0.2,
        }
    }

    /// The front of the ball at a page position, where it is over it.
    fn front(&self, x: f32, y: f32) -> Option<f32> {
        let d2 = (x - self.ball.x).powi(2) + (y - self.ball.y).powi(2);
        let r2 = self.ball.r * self.ball.r;
        (d2 < r2).then(|| self.zc + (r2 - d2).sqrt())
    }

    /// The surface round this eye, given the skin `core` would have without
    /// it and how open the eye is at the pixel.
    fn at(&self, x: f32, y: f32, core: f32, open: f32) -> f32 {
        let eye = self.eye;
        if !eye.upper.near(x, y, self.fold + 3.0) && !eye.lower.near(x, y, 4.0) {
            return core;
        }
        let Some(front) = self.front(x, y) else {
            // The corners reach round past the ball: deep, and dark for it
            return core - 3.0 * open;
        };
        let along = |f: &Foot| {
            Ramp::smooth(0.0, 0.12, f.u)
                * Ramp::smooth(1.0, 0.88, f.u)
                * Ramp::smooth(0.8, 0.0, f.past)
        };
        let top = eye.upper.foot(x, y);
        let up = -top.d * eye.side;
        let w_up = Ramp::smooth(-0.4, 0.2, up)
            * (1.0 - Ramp::smooth(self.fold * 0.55, self.fold + 1.0, up))
            * along(&top);
        let bottom = eye.lower.foot(x, y);
        let down = bottom.d * eye.side;
        let w_lo =
            Ramp::smooth(-0.4, 0.2, down) * (1.0 - Ramp::smooth(0.3, 2.6, down)) * along(&bottom);
        let lid = front
            + if w_up >= w_lo {
                self.upper
            } else {
                Self::LOWER
            };
        let w = w_up.max(w_lo);
        let skin = core + w * (Relief::smooth_max(core, lid, 1.5) - core);
        skin + (front - skin) * open
    }
}

pub struct Relief {
    /// How much of each pixel the head covers
    pub cover: Plane,
    /// The skin's depth, every feature on it
    pub z: Plane,
    /// The bare skull: what hair and beard are laid over
    pub skull: Plane,
    /// How much of each pixel is open eye rather than lid
    pub opening: Plane,
    pub balls: [Eyeball; 2],
}

impl Relief {
    /// Mid-line height of the skull above the plane of its outline: a head
    /// is about as deep in front of its ears as it is wide.
    pub const DEPTH: f32 = 68.0;

    pub fn build(
        grid: &Grid,
        l: &Landmarks,
        id: &Identity,
        noise: &Noise,
        age: u8,
        heft: f32,
        neck: &Plane,
    ) -> Relief {
        let s = &l.skull;
        let cover = l.head.coverage(grid);
        // Near-elliptical across, a little fuller through the cranium; the
        // forehead rolls back over the radius of the whole vault, the jaw
        // turns under sharply on a lean man and softly on a heavy one
        let (zygo_y, jaw_y) = (s.zygo_y, s.jaw_y);
        let jaw_roll = (9.0 + heft * 1.5).clamp(6.5, 13.0);
        // Where the jaw turns under it meets the neck, not the plane behind
        // it: the chin then stands in front of the throat and shades it
        let skull = Volume::raise(grid, &l.head, Self::DEPTH, (62.0, jaw_roll), move |y| {
            let t = ((y - zygo_y) / (jaw_y - zygo_y)).clamp(0.0, 1.0);
            2.05 - 0.2 * t
        })
        .map(|k, z| if z > 0.0 { z.max(neck.v[k]) } else { 0.0 });

        let forms = Self::forms(l, id, age, heft);
        let nose = Nose::new(l, id, l.cx + id.turn * 0.35 * 3.0);
        let mouth = Mouth::new(l, id, l.cx + id.turn * 0.35 * 1.8);
        let balls = l.eyes.each_ref().map(|eye| Eyeball {
            x: eye.cx,
            y: eye.cy + 0.4,
            r: 10.5,
        });
        let lids = [0, 1].map(|e| Lids::new(&l.eyes[e], balls[e], &skull, l, id));
        // The lids meet the ball over a hair's breadth of wet margin, not
        // along a cut edge
        let opening = {
            let [a, b] = l.eyes.each_ref().map(|e| {
                e.opening
                    .distance(grid)
                    .map(|_, d| Ramp::smooth(-0.35, 0.45, d))
            });
            a.map(|k, v| v.max(b.v[k]))
        };

        let z = Plane::from_pixels(*grid, |i, j| {
            let k = j * grid.w + i;
            let base = skull.v[k];
            if cover.v[k] <= 0.0 && base <= 0.0 {
                return 0.0;
            }
            let (x, y) = (grid.x(i), grid.y(j));
            // No face is modelled from perfect solids: soft lumps of fat
            // and muscle that no landmark names
            let lumps = 0.3 * noise.fbm(x * 0.07 + 31.0, y * 0.07, 2)
                + 0.06 * noise.fbm(x * 0.26, y * 0.26 + 17.0, 2);
            let core = base
                + forms.iter().map(|f| f.at(x, y)).sum::<f32>()
                + nose.at(x, y)
                + mouth.at(x, y)
                + lumps * Ramp::smooth(0.0, 12.0, base);
            lids[usize::from(x > l.cx)]
                .at(x, y, core, opening.v[k])
                .max(0.0)
        });

        Relief {
            cover,
            z,
            skull,
            opening,
            balls,
        }
    }

    /// Polynomial smooth maximum: a fillet of width `k` where two surfaces
    /// meet instead of a crease.
    fn smooth_max(a: f32, b: f32, k: f32) -> f32 {
        let h = (k - (a - b).abs()).max(0.0) / k;
        a.max(b) + h * h * k * 0.25
    }

    /// Everything modelled onto the skull.
    fn forms(l: &Landmarks, id: &Identity, age: u8, heft: f32) -> Vec<Form> {
        let cx = l.cx;
        let s = &l.skull;
        let m = &id.morph;
        let es = &l.eye_shape;
        let ns = &l.nose_shape;
        let ms = &l.mouth_shape;
        let mat = l.maturity;
        let bone = 1.0 + m.brow_ridge * 0.35 + mat * 0.25;
        let wrinkle = match age {
            0..=25 => 0.0f32,
            26..=29 => 0.25,
            30..=33 => 0.5,
            34..=36 => 0.8,
            _ => 1.0,
        };
        // A slight turn of the head moves what stands furthest forward
        // furthest across
        let turn = id.turn * 0.35;
        let nx = cx + turn * 3.0;
        let mx = cx + turn * 1.8;

        let mut f = Vec::with_capacity(64);

        // ── Brow, sockets, forehead ──────────────────────────
        f.push(Form::blob(cx, l.brow + 3.0, 12.0, 9.0, 0.0, 1.4 * bone));
        f.push(Form::blob(
            cx + 13.0,
            l.hairline + 17.0,
            13.0,
            12.0,
            0.0,
            1.5,
        ));
        f.push(Form::blob(
            cx - 13.0,
            l.hairline + 17.0,
            13.0,
            12.0,
            0.0,
            1.5,
        ));
        let deep = if id.eye_st == 4 { 1.8 } else { 0.0 };
        for eye in &l.eyes {
            let side = eye.side;
            f.push(Form::blob(
                eye.cx + side * 1.5,
                l.brow + 0.5,
                17.0,
                6.5,
                side * 7.0,
                3.0 * bone,
            ));
            // The orbit: deepest just under the brow, where the lid folds
            // in under it
            f.push(Form::blob(
                eye.cx,
                l.eye - 1.0,
                15.0,
                11.0,
                0.0,
                -(8.0 + m.lid_heavy * 1.2 + deep),
            ));
            // The orbit is deepest on the nose side of the eye
            f.push(Form::blob(
                eye.inner.0 - side * 2.0,
                eye.inner.1 - 1.5,
                4.5,
                6.5,
                0.0,
                -2.6,
            ));
            // The lower lid's own little shelf, and the trough under it
            f.push(Form::blob(
                eye.cx + side * 0.5,
                l.eye + es.ry + 4.8,
                es.rx * 0.85,
                3.0,
                0.0,
                0.35 + mat * 0.6 + m.lid_heavy * 0.3,
            ));
            f.push(Form::crease(
                &[
                    (eye.inner.0 - eye.side * 0.5, eye.inner.1 + 3.5),
                    (eye.cx, l.eye + es.ry + 7.8),
                    (eye.outer.0 + eye.side * 1.0, eye.outer.1 + 5.5),
                ],
                3.2,
                -(0.05 + mat * 0.3),
                0.3,
            ));
            if let Some(crease) = &eye.crease {
                // The fold: a groove with the skin above it rolling over
                f.push(Form::along(crease.clone(), 1.3, -1.1, 0.2));
                f.push(Form::blob(
                    eye.cx + side * 1.0,
                    eye.top - 5.2,
                    es.rx * 0.95,
                    2.4,
                    0.0,
                    0.6,
                ));
            }
            // Temples
            f.push(Form::blob(
                cx + side * (s.temple - 7.0),
                s.temple_y - 4.0,
                7.0,
                13.0,
                0.0,
                -2.6,
            ));
            // Cheekbone, the fat pad under the eye, and the hollow the
            // cheekbone overhangs
            f.push(Form::blob(
                cx + side * s.zygo * 0.62,
                s.zygo_y + 1.0,
                21.0,
                14.0,
                -side * 15.0,
                2.6 + m.cheek * 0.7,
            ));
            f.push(Form::blob(
                eye.cx + side * 2.0,
                l.eye + 17.0,
                16.0,
                11.0,
                -side * 20.0,
                0.8 + heft.max(0.0) * 0.3,
            ));
            // The hollow runs down and in under the cheekbone, toward the
            // corner of the mouth
            f.push(Form::blob(
                cx + side * s.sub * 0.86,
                s.sub_y + 12.0,
                9.0,
                16.0,
                side * 18.0,
                -0.4 - mat * 0.5 + heft * 0.8 + m.cheek.min(0.0) * 0.3,
            ));
            // The corner of the jaw, and the masseter over it
            f.push(Form::blob(
                cx + side * (s.jaw - 4.0),
                s.jaw_y - 5.0,
                7.0,
                11.0,
                0.0,
                1.2 + mat * 1.0,
            ));
        }

        // ── Muzzle, lips, chin ───────────────────────────────
        let my = l.mouth;
        let half = ms.half;
        let lip_top = my - ms.upper;
        f.push(Form::blob(mx, my - 2.0, half + 16.0, 22.0, 0.0, 4.5));
        // Philtrum: two ridges and the groove between them
        for side in [-1.0f32, 1.0] {
            f.push(Form::crease(
                &[
                    (mx + side * 1.9, l.nose + 3.0),
                    (mx + side * 3.0, lip_top - 0.3),
                ],
                1.8,
                0.35,
                0.25,
            ));
        }
        for side in [-1.0f32, 1.0] {
            let corner = (mx + side * half, my + l.lips.corner_dy);
            f.push(Form::blob(corner.0, corner.1, 1.8, 1.6, 0.0, -1.4));
            f.push(Form::blob(
                corner.0 + side * 2.6,
                corner.1 - 0.4,
                3.4,
                4.6,
                0.0,
                0.9,
            ));
            // Nasolabial fold: the cheek's fat overhangs the muzzle
            let fold = [
                (nx + side * (ns.tip + 2.5), l.nose - 1.5),
                (mx + side * (half + 3.5), my - 3.5),
                (mx + side * (half + 5.5), my + 7.0),
            ];
            // A hint on a young face; it only becomes a line with the years
            f.push(Form::crease(
                &fold,
                5.0,
                -(0.05 + wrinkle * 0.9 + mat * mat * 0.3),
                0.25,
            ));
            let outside = fold.map(|(x, y)| (x + side * 5.0, y));
            f.push(Form::crease(
                &outside,
                8.0,
                0.1 + mat * mat * 0.4 + heft.max(0.0) * 0.3,
                0.3,
            ));
            if age >= 33 {
                f.push(Form::crease(
                    &[
                        (mx + side * (half + 1.0), my + 2.0),
                        (mx + side * (half + 2.5), l.sulcus + 4.0),
                        (mx + side * (half + 1.5), s.chin - 8.0),
                    ],
                    1.6,
                    -0.5 * wrinkle,
                    0.3,
                ));
            }
        }
        f.push(Form::blob(
            cx + turn * 1.2,
            s.chin - 9.0,
            s.chin_half * 0.95 + 3.0,
            8.5,
            0.0,
            4.0 + m.chin_w * 0.3,
        ));
        if id.jitter(1, 200) < 0.18 {
            f.push(Form::crease(
                &[(cx, s.chin - 13.0), (cx, s.chin - 4.0)],
                1.6,
                -0.7,
                0.3,
            ));
        }

        // ── The lines a face earns ───────────────────────────
        if wrinkle > 0.2 {
            let top = l.hairline + 8.0;
            let span = (l.brow - 12.0) - top;
            for k in 0..3 {
                let y = top + span * (0.25 + k as f32 * 0.28) + id.jitter_signed(k, 3) * 1.2;
                f.push(Form::crease(
                    &[(cx - 21.0, y + 1.5), (cx, y - 1.0), (cx + 21.0, y + 1.5)],
                    0.9,
                    -0.2 * wrinkle,
                    0.25,
                ));
            }
        }
        if age >= 29 {
            for eye in &l.eyes {
                let x0 = eye.outer.0 + eye.side * 1.5;
                for k in 0..3 {
                    let dy = (k as f32 - 1.0) * 2.2;
                    f.push(Form::crease(
                        &[
                            (x0, l.eye + dy),
                            (x0 + eye.side * 5.5, l.eye + dy * 2.4 + 1.0),
                        ],
                        0.8,
                        -0.22 * wrinkle.max(0.3),
                        0.3,
                    ));
                }
            }
        }
        f
    }
}
