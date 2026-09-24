//! The neck under the head — all of the body a portrait carries. The man is
//! fitted onto a body elsewhere, so the neck stops as a column rather than
//! spreading into shoulders that would never sit on anybody else's.

use super::canvas::{Grid, Plane};
use super::geometry::Landmarks;
use super::relief::{Form, Volume};

/// A shape to be shaded: how much of each pixel it covers and how far it
/// stands toward the lens.
pub struct Solid {
    pub cover: Plane,
    pub z: Plane,
}

pub struct Body;

impl Body {
    /// Front of the neck, under the chin.
    pub const NECK_DEPTH: f32 = 31.0;

    /// The neck: a column with the two sternocleidomastoids running from
    /// behind the ears toward the pit of the throat, the larynx between them.
    pub fn neck(grid: &Grid, l: &Landmarks) -> Solid {
        let cx = l.cx;
        let nh = l.neck_half;
        let chin = l.skull.chin;
        let cover = l.neck.coverage(grid);
        let base = Volume::raise(grid, &l.neck, Self::NECK_DEPTH, (8.0, 2.0), |_| 2.5);
        let mut forms = vec![Form::blob(
            cx,
            chin + 3.0,
            4.0,
            6.5,
            0.0,
            1.2 + l.maturity * 0.8,
        )];
        for side in [-1.0f32, 1.0] {
            forms.push(Form::crease(
                &[
                    (cx + side * (nh - 3.0), chin - 20.0),
                    (cx + side * (nh - 8.0), chin - 2.0),
                    (cx + side * 7.0, chin + 30.0),
                ],
                7.0,
                0.45,
                0.25,
            ));
        }
        let z = base.map(|k, b| {
            if b <= 0.0 {
                return 0.0;
            }
            let (x, y) = (grid.x(k % grid.w), grid.y(k / grid.w));
            b + forms.iter().map(|f| f.at(x, y)).sum::<f32>()
        });
        Solid { cover, z }
    }
}
