//! The shade of a flat illustration. No light is worked out: the studio's
//! key still stands high on the camera's left, but all it leaves on a head
//! is a few flat shapes of the skin's shadow tone — down the side of the
//! face turned from it and under the jaw, in the sockets under the brows,
//! on the far wall of the nose and under its tip, in the folds of the lids
//! and the line where the lips meet, under the lower lip, and on the neck
//! under the chin. Each is drawn off the man's own landmarks, so every face
//! carries its own.

use std::f32::consts::TAU;

use super::canvas::{Grid, Outline, Path, Plane, Polyline, Ramp};
use super::color::Linear;
use super::geometry::{Eye, Landmarks};
use super::identity::Identity;

pub struct Shade;

impl Shade {
    /// What full shade does to a pigment: darker and redder, as shadow on
    /// skin is — never a grey.
    const TINT: Linear = Linear::new(0.64, 0.52, 0.50);

    /// How much of the shadow tone the side of a head turned from the light
    /// takes.
    pub const SIDE: f32 = 0.42;

    /// A pigment under `w` of the shadow tone.
    pub fn over(albedo: Linear, w: f32) -> Linear {
        albedo * Linear::gray(1.0).mix(Self::TINT, w)
    }

    /// Hair under `w` of the shadow tone: only darker, the light never
    /// having gone into it to come back redder.
    pub fn dim(albedo: Linear, w: f32) -> Linear {
        albedo * (1.0 - 0.7 * w)
    }

    /// The shadow tone over the face, 0..1.
    pub fn face(grid: &Grid, l: &Landmarks, id: &Identity) -> Plane {
        let side = Self::side(l).feathered(grid, 1.6);
        let sockets = l
            .eyes
            .each_ref()
            .map(|e| Self::socket(l, e).feathered(grid, 2.4));
        let wall = Self::nose_wall(l, id).feathered(grid, 1.0);
        let tip = Self::nose_under(l, id).feathered(grid, 1.0);
        let chin = Self::lip_under(l).feathered(grid, 1.6);
        let mouth = Self::mouth(l);
        // A heavy brow ridge sinks the eyes deeper under it
        let deep = 0.14 + 0.06 * id.morph.brow_ridge;
        Plane::from_fn(*grid, |k, x, y| {
            let folds = l
                .eyes
                .iter()
                .filter_map(|e| e.crease.as_ref())
                .map(|crease| Self::line(crease, x, y, 0.35))
                .fold(0.0, f32::max);
            (side.v[k] * Self::SIDE)
                .max(sockets[0].v[k].max(sockets[1].v[k]) * deep)
                .max(wall.v[k] * 0.36)
                .max(tip.v[k] * 0.40)
                .max(chin.v[k] * 0.26)
                .max(folds * 0.34)
                .max(Self::line(&mouth, x, y, 0.45) * 0.62)
        })
    }

    /// A drawn line `half` page units either side of a curve, tapering to
    /// nothing at its ends: 0..1.
    fn line(curve: &Polyline, x: f32, y: f32, half: f32) -> f32 {
        if !curve.near(x, y, half * 2.0) {
            return 0.0;
        }
        let foot = curve.foot(x, y);
        (1.0 - Ramp::smooth(half * 0.5, half * 1.5, foot.d.abs()))
            * Ramp::smooth(0.0, 0.12, foot.u)
            * Ramp::smooth(1.0, 0.88, foot.u)
    }

    /// The shadow tone over the neck, 0..1: what the head throws down onto
    /// it, and the far side of the column turned from the light.
    pub fn neck(grid: &Grid, l: &Landmarks) -> Plane {
        let cast = l.head.shifted((2.5, 7.5)).feathered(grid, 2.0);
        let far = l.cx + l.neck_half * 0.52;
        Plane::from_fn(*grid, |k, x, _| {
            (cast.v[k] * 0.46).max(Ramp::smooth(far - 1.0, far + 1.0, x) * 0.30)
        })
    }

    /// The side of the face turned from the light: from the crown down the
    /// far cheek, round under the jaw and the chin, and back up under the
    /// near jaw. Past the head's own edge it runs on outside, where the
    /// head's cover cuts it.
    fn side(l: &Landmarks) -> Outline {
        let s = &l.skull;
        let cx = l.cx;
        let edge = [
            (cx + s.parietal * 1.04, s.crown + 12.0),
            (cx + s.parietal * 0.86, s.parietal_y),
            (cx + s.temple * 0.78, s.temple_y),
            (cx + s.zygo * 0.70, s.zygo_y),
            (cx + s.sub * 0.64, s.sub_y),
            (cx + s.jaw * 0.66, s.jaw_y - 1.0),
            (cx + s.chin_half * 0.62, s.chin - 6.5),
            (cx + s.chin_half * 0.05, s.chin - 3.2),
            (cx - s.chin_half * 0.75, s.chin - 4.2),
            (cx - s.jaw * 0.92, s.jaw_y + 1.5),
            (cx - s.sub * 1.06, s.sub_y + 4.0),
        ];
        let mut pts = Path::through(&edge, 0.0).points();
        pts.extend([
            (cx - s.sub - 30.0, s.sub_y + 4.0),
            (cx - s.sub - 30.0, s.chin + 30.0),
            (cx + s.parietal + 30.0, s.chin + 30.0),
            (cx + s.parietal + 30.0, s.crown + 12.0),
        ]);
        Outline::polygon(pts)
    }

    /// The hollow between the brow and the lid.
    fn socket(l: &Landmarks, eye: &Eye) -> Outline {
        Self::ellipse(
            (eye.cx + eye.side * 1.0, eye.top - 2.8),
            (l.eye_shape.rx * 1.05, 3.0),
        )
    }

    /// The wall of the nose turned from the light, from beside the ridge at
    /// the root down to the wing.
    fn nose_wall(l: &Landmarks, id: &Identity) -> Outline {
        let ns = &l.nose_shape;
        let (root, base) = (l.eye - 4.0, l.nose);
        let (x_root, x_tip) = (l.cx, Self::tip_x(l, id));
        let (mid, xm) = ((root + base) / 2.0, (x_root + x_tip) / 2.0);
        let tip = base - ns.ball * 0.9;
        Outline::smooth(
            &[
                (x_root + ns.bridge * 0.45, root - 1.0),
                (xm + ns.bridge * 0.55, mid),
                (x_tip + ns.ball * 0.45, tip - 1.0),
                (x_tip + ns.tip * 0.55, base - 2.0),
                (x_tip + ns.tip * 0.95, base - 0.5),
                (x_tip + ns.tip * 1.05, base - 3.5),
                (xm + ns.bridge * 1.6, mid + 2.0),
                (x_root + ns.bridge * 1.5, root + 3.0),
            ],
            0.2,
        )
    }

    /// Under the tip of the nose, thrown down and away from the light onto
    /// the top of the lip.
    fn nose_under(l: &Landmarks, id: &Identity) -> Outline {
        let ns = &l.nose_shape;
        Self::ellipse(
            (Self::tip_x(l, id) + 1.2, l.nose + 2.2),
            (ns.tip * 0.72, 1.7),
        )
    }

    /// The hollow under the lower lip.
    fn lip_under(l: &Landmarks) -> Outline {
        let ms = &l.mouth_shape;
        Self::ellipse(
            (l.cx + 0.8, l.mouth + ms.lower * 1.1 + 2.0),
            (ms.half * 0.45, 1.5),
        )
    }

    /// The line where the lips meet, corner to corner.
    fn mouth(l: &Landmarks) -> Polyline {
        let (half, my, dy) = (l.mouth_shape.half, l.mouth, l.lips.corner_dy);
        Path::from((l.cx - half, my + dy))
            .quad((l.cx, my + 1.0), (l.cx + half, my + dy))
            .polyline()
    }

    /// Where the tip of the nose stands: a slight turn of the head moves it
    /// furthest across.
    fn tip_x(l: &Landmarks, id: &Identity) -> f32 {
        l.cx + id.turn * 0.35 * 3.0
    }

    fn ellipse((x, y): (f32, f32), (rx, ry): (f32, f32)) -> Outline {
        let pts: Vec<(f32, f32)> = (0..12)
            .map(|k| {
                let a = k as f32 / 12.0 * TAU;
                (x + rx * a.cos(), y + ry * a.sin())
            })
            .collect();
        Outline::smooth(&pts, 0.0)
    }
}
