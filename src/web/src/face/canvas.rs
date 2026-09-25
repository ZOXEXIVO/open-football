//! The raster a portrait is painted on, and the shapes it is built from.
//!
//! Everything is laid out in PAGE units — the `200 × 250` box the match
//! viewer's landmarks are written in — and a [`Grid`] is a window onto the
//! page: where it starts and how many pixels each unit gets. A face seen
//! through two windows is the same face, which is what lets the cutout be
//! the portrait with its setting taken away.

use std::ops::Range;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use rayon::prelude::*;

use super::color::Linear;

#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    /// Pixels per page unit
    pub scale: f32,
    /// The page position of the window's top-left corner
    pub x0: f32,
    pub y0: f32,
}

impl Grid {
    pub const PAGE_W: f32 = 200.0;
    pub const PAGE_H: f32 = 250.0;

    /// The whole page.
    pub fn new(scale: f32) -> Grid {
        Self::window(
            scale,
            (0.0, 0.0),
            (
                (Self::PAGE_W * scale).round() as usize,
                (Self::PAGE_H * scale).round() as usize,
            ),
        )
    }

    /// `w × h` pixels of the page from `origin` on — which may lie off the
    /// page, for a picture framed wider than the head.
    pub fn window(scale: f32, (x0, y0): (f32, f32), (w, h): (usize, usize)) -> Grid {
        Grid {
            w,
            h,
            scale,
            x0,
            y0,
        }
    }

    pub fn len(&self) -> usize {
        self.w * self.h
    }

    /// Page position of a pixel's centre.
    pub fn x(&self, i: usize) -> f32 {
        self.x0 + (i as f32 + 0.5) / self.scale
    }

    pub fn y(&self, j: usize) -> f32 {
        self.y0 + (j as f32 + 0.5) / self.scale
    }

    /// A value per pixel from its column, row and page position, rows in
    /// parallel.
    pub fn map<T: Send + Default + Clone>(
        &self,
        f: impl Fn(usize, usize, f32, f32) -> T + Sync,
    ) -> Vec<T> {
        let mut out = vec![T::default(); self.len()];
        out.par_chunks_mut(self.w).enumerate().for_each(|(j, row)| {
            let y = self.y(j);
            for (i, v) in row.iter_mut().enumerate() {
                *v = f(i, j, self.x(i), y);
            }
        });
        out
    }

    /// The columns whose centres fall inside page span `a..b`.
    pub fn cols(&self, a: f32, b: f32) -> Range<usize> {
        Self::span(a - self.x0, b - self.x0, self.scale, self.w)
    }

    pub fn rows(&self, a: f32, b: f32) -> Range<usize> {
        Self::span(a - self.y0, b - self.y0, self.scale, self.h)
    }

    fn span(a: f32, b: f32, scale: f32, n: usize) -> Range<usize> {
        let lo = (a * scale - 0.5).ceil().clamp(0.0, n as f32) as usize;
        let hi = ((b * scale - 0.5).floor() + 1.0).clamp(0.0, n as f32) as usize;
        lo.min(hi)..hi
    }
}

/// A scalar field over the grid: a coverage, a weight, a distance.
#[derive(Clone)]
pub struct Plane {
    pub grid: Grid,
    pub v: Vec<f32>,
}

impl Plane {
    pub fn new(grid: Grid, fill: f32) -> Plane {
        Plane {
            grid,
            v: vec![fill; grid.len()],
        }
    }

    /// Every pixel from its index and page position, rows in parallel.
    pub fn from_fn(grid: Grid, f: impl Fn(usize, f32, f32) -> f32 + Sync) -> Plane {
        Plane {
            grid,
            v: grid.map(|i, j, x, y| f(j * grid.w + i, x, y)),
        }
    }

    /// A new plane from this one pixel by pixel.
    pub fn map(&self, f: impl Fn(usize, f32) -> f32 + Sync) -> Plane {
        let v = self
            .v
            .par_iter()
            .enumerate()
            .map(|(k, &a)| f(k, a))
            .collect();
        Plane { grid: self.grid, v }
    }
}

/// A closed shape, flattened to a polygon in page units.
#[derive(Clone)]
pub struct Outline {
    pts: Vec<(f32, f32)>,
}

impl Outline {
    /// How far outside a shape's own box the distance field bothers to
    /// measure; further out everything is simply [`Self::FAR`] away.
    const MARGIN: f32 = 14.0;
    pub const FAR: f32 = 40.0;

    pub fn polygon(pts: Vec<(f32, f32)>) -> Outline {
        Outline { pts }
    }

    /// A closed Catmull-Rom curve through `pts`. `tension` 0 is the rounded
    /// fit; toward 1 the curve hugs the polygon.
    pub fn smooth(pts: &[(f32, f32)], tension: f32) -> Outline {
        let n = pts.len();
        let k = (1.0 - tension) / 6.0;
        let mut out = Vec::with_capacity(n * 10);
        for i in 0..n {
            let p0 = pts[(i + n - 1) % n];
            let p1 = pts[i];
            let p2 = pts[(i + 1) % n];
            let p3 = pts[(i + 2) % n];
            let c1 = (p1.0 + (p2.0 - p0.0) * k, p1.1 + (p2.1 - p0.1) * k);
            let c2 = (p2.0 - (p3.0 - p1.0) * k, p2.1 - (p3.1 - p1.1) * k);
            Path::flatten_cubic(&mut out, p1, c1, c2, p2, 10);
        }
        Outline { pts: out }
    }

    /// The same shape moved across the page.
    pub fn shifted(&self, (dx, dy): (f32, f32)) -> Outline {
        Outline {
            pts: self.pts.iter().map(|&(x, y)| (x + dx, y + dy)).collect(),
        }
    }

    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        self.pts.iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        )
    }

    /// Signed distance to the edge in page units, positive inside.
    ///
    /// Measured along the row and the column through each pixel and combined
    /// as the distance to the line through those two crossings — exact for a
    /// straight edge at any angle, and cheap, because the crossings of a row
    /// or a column are worked out once for all the pixels on it.
    pub fn distance(&self, grid: &Grid) -> Plane {
        let (x0, y0, x1, y1) = self.bounds();
        let mut plane = Plane::new(*grid, -Self::FAR);
        let rows = grid.rows(y0 - Self::MARGIN, y1 + Self::MARGIN);
        let cols = grid.cols(x0 - Self::MARGIN, x1 + Self::MARGIN);
        let columns: Vec<Vec<f32>> = cols
            .clone()
            .map(|i| self.crossings(grid.x(i), false))
            .collect();
        let w = grid.w;
        plane
            .v
            .par_chunks_mut(w)
            .enumerate()
            .filter(|(j, _)| rows.contains(j))
            .for_each(|(j, row)| {
                let y = grid.y(j);
                let xs = self.crossings(y, true);
                for (c, i) in cols.clone().enumerate() {
                    let x = grid.x(i);
                    let inside = xs.iter().filter(|&&cx| cx < x).count() % 2 == 1;
                    let dx = Self::nearest(&xs, x);
                    let dy = Self::nearest(&columns[c], y);
                    let d = match (dx.is_finite(), dy.is_finite()) {
                        (true, true) => dx * dy / (dx * dx + dy * dy).sqrt().max(1e-6),
                        (true, false) => dx,
                        (false, true) => dy,
                        (false, false) => Self::FAR,
                    }
                    .min(Self::FAR);
                    row[i] = if inside { d } else { -d };
                }
            });
        plane
    }

    /// Anti-aliased coverage, 0..1.
    pub fn coverage(&self, grid: &Grid) -> Plane {
        let scale = grid.scale;
        self.distance(grid)
            .map(|_, d| (0.5 + d * scale).clamp(0.0, 1.0))
    }

    /// Coverage with an edge `soft` page units wide — a painted edge rather
    /// than a cut one.
    pub fn feathered(&self, grid: &Grid, soft: f32) -> Plane {
        self.distance(grid)
            .map(|_, d| Ramp::smooth(-soft * 0.5, soft * 0.5, d))
    }

    /// The outermost crossings of the row at `y`: where the shape starts and
    /// ends across the page.
    pub fn row_span(&self, y: f32) -> Option<(f32, f32)> {
        let xs = self.crossings(y, true);
        Some((*xs.first()?, *xs.last()?))
    }

    /// Where the edges cross the line `y = at` (rows) or `x = at`
    /// (columns), sorted.
    fn crossings(&self, at: f32, row: bool) -> Vec<f32> {
        let n = self.pts.len();
        let mut out = Vec::new();
        for k in 0..n {
            let a = self.pts[k];
            let b = self.pts[(k + 1) % n];
            let (a0, a1, b0, b1) = if row {
                (a.1, a.0, b.1, b.0)
            } else {
                (a.0, a.1, b.0, b.1)
            };
            if (a0 <= at && b0 > at) || (b0 <= at && a0 > at) {
                out.push(a1 + (at - a0) / (b0 - a0) * (b1 - a1));
            }
        }
        out.sort_by(f32::total_cmp);
        out
    }

    fn nearest(sorted: &[f32], v: f32) -> f32 {
        sorted
            .iter()
            .map(|c| (c - v).abs())
            .fold(f32::INFINITY, f32::min)
    }
}

/// A run of straight and curved segments in page units, flattened as it is
/// built — closed into an [`Outline`] or left open as a [`Polyline`].
pub struct Path {
    pts: Vec<(f32, f32)>,
}

impl Path {
    pub fn from(p: (f32, f32)) -> Path {
        Path { pts: vec![p] }
    }

    /// An open Catmull-Rom run through `pts`; `tension` as for
    /// [`Outline::smooth`].
    pub fn through(pts: &[(f32, f32)], tension: f32) -> Path {
        let n = pts.len();
        let k = (1.0 - tension) / 6.0;
        let mut out = vec![pts[0]];
        for i in 0..n.saturating_sub(1) {
            let p0 = pts[i.saturating_sub(1)];
            let p1 = pts[i];
            let p2 = pts[i + 1];
            let p3 = pts[(i + 2).min(n - 1)];
            let c1 = (p1.0 + (p2.0 - p0.0) * k, p1.1 + (p2.1 - p0.1) * k);
            let c2 = (p2.0 - (p3.0 - p1.0) * k, p2.1 - (p3.1 - p1.1) * k);
            Self::flatten_cubic(&mut out, p1, c1, c2, p2, 10);
        }
        Path { pts: out }
    }

    pub fn line(mut self, p: (f32, f32)) -> Path {
        self.pts.push(p);
        self
    }

    pub fn quad(mut self, c: (f32, f32), p: (f32, f32)) -> Path {
        let a = self.last();
        let c1 = (a.0 + (c.0 - a.0) * 2.0 / 3.0, a.1 + (c.1 - a.1) * 2.0 / 3.0);
        let c2 = (p.0 + (c.0 - p.0) * 2.0 / 3.0, p.1 + (c.1 - p.1) * 2.0 / 3.0);
        Self::flatten_cubic(&mut self.pts, a, c1, c2, p, 12);
        self
    }

    pub fn cubic(mut self, c1: (f32, f32), c2: (f32, f32), p: (f32, f32)) -> Path {
        let a = self.last();
        Self::flatten_cubic(&mut self.pts, a, c1, c2, p, 16);
        self
    }

    pub fn outline(self) -> Outline {
        Outline::polygon(self.pts)
    }

    pub fn polyline(self) -> Polyline {
        Polyline::new(self.pts)
    }

    pub fn points(self) -> Vec<(f32, f32)> {
        self.pts
    }

    fn last(&self) -> (f32, f32) {
        *self.pts.last().expect("a path starts with a point")
    }

    /// Appends `n` points along the cubic, not repeating its start.
    fn flatten_cubic(
        out: &mut Vec<(f32, f32)>,
        p0: (f32, f32),
        c1: (f32, f32),
        c2: (f32, f32),
        p3: (f32, f32),
        n: usize,
    ) {
        for s in 1..=n {
            let t = s as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push((
                a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1,
            ));
        }
    }
}

/// Where a point stands relative to a [`Polyline`].
#[derive(Clone, Copy, Debug)]
pub struct Foot {
    /// How far along the line its nearest point is, 0..1 by arc length
    pub u: f32,
    /// Signed distance, positive on the right hand of travel as drawn on
    /// the page — below a line run left to right
    pub d: f32,
    /// How far beyond either end of the line the point lies, 0 alongside it
    pub past: f32,
}

/// An open curve that features are measured from: a lid margin, a crease,
/// the spine of a brow.
#[derive(Clone)]
pub struct Polyline {
    pts: Vec<(f32, f32)>,
    cum: Vec<f32>,
    bounds: (f32, f32, f32, f32),
}

impl Polyline {
    pub fn new(pts: Vec<(f32, f32)>) -> Polyline {
        let mut cum = vec![0.0];
        for w in pts.windows(2) {
            let len = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
            cum.push(cum.last().copied().unwrap_or(0.0) + len);
        }
        let bounds = Outline::polygon(pts.clone()).bounds();
        Polyline { pts, cum, bounds }
    }

    pub fn len(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// True when `(x, y)` is within `reach` of the line's box — the cheap
    /// rejection every per-pixel caller does first.
    pub fn near(&self, x: f32, y: f32, reach: f32) -> bool {
        let (x0, y0, x1, y1) = self.bounds;
        x > x0 - reach && x < x1 + reach && y > y0 - reach && y < y1 + reach
    }

    pub fn foot(&self, x: f32, y: f32) -> Foot {
        let total = self.len().max(1e-6);
        let last = self.pts.len().saturating_sub(2);
        let mut best = (f32::MAX, 0.0f32, 0.0f32, 0.0f32);
        for (k, w) in self.pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let (sx, sy) = (b.0 - a.0, b.1 - a.1);
            let seg = (sx * sx + sy * sy).max(1e-9);
            let raw = ((x - a.0) * sx + (y - a.1) * sy) / seg;
            let t = raw.clamp(0.0, 1.0);
            let (px, py) = (a.0 + sx * t, a.1 + sy * t);
            let dist = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
            if dist < best.0 {
                let side = sx * (y - a.1) - sy * (x - a.0);
                let along = self.cum[k] + seg.sqrt() * t;
                let past = if k == 0 && raw < 0.0 {
                    -raw * seg.sqrt()
                } else if k == last && raw > 1.0 {
                    (raw - 1.0) * seg.sqrt()
                } else {
                    0.0
                };
                best = (dist, along, if side >= 0.0 { 1.0 } else { -1.0 }, past);
            }
        }
        Foot {
            u: best.1 / total,
            d: best.0 * best.2,
            past: best.3,
        }
    }

    /// The point `u` of the way along, by arc length.
    pub fn at(&self, u: f32) -> (f32, f32) {
        let target = u.clamp(0.0, 1.0) * self.len();
        for (k, w) in self.pts.windows(2).enumerate() {
            let (s0, s1) = (self.cum[k], self.cum[k + 1]);
            if target <= s1 || k + 2 == self.pts.len() {
                let t = if s1 > s0 {
                    (target - s0) / (s1 - s0)
                } else {
                    0.0
                };
                return (
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t,
                );
            }
        }
        self.pts[0]
    }
}

/// One soft weight laid on the page: where colour gathers on a face, where
/// a fold of the ear sinks in.
pub enum Form {
    /// (1 − r²)³ inside a rotated ellipse: full at the centre, feathered to
    /// nothing at the rim with no ring at its edge.
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

    /// A crease through `pts`, as a smooth run rather than a polygon.
    pub fn crease(pts: &[(f32, f32)], w: f32, h: f32, taper: f32) -> Form {
        Form::Crease {
            line: Path::through(pts, 0.0).polyline(),
            w,
            h,
            taper,
        }
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

/// One layer of the picture: straight colour and coverage per pixel.
pub struct Layer {
    pub color: Vec<Linear>,
    pub alpha: Vec<f32>,
}

impl Layer {
    /// Paints every pixel that `f` says is covered, rows in parallel.
    pub fn paint(
        grid: &Grid,
        f: impl Fn(usize, usize, f32, f32) -> Option<(Linear, f32)> + Sync,
    ) -> Layer {
        let w = grid.w;
        let mut color = vec![Linear::BLACK; grid.len()];
        let mut alpha = vec![0.0; grid.len()];
        color
            .par_chunks_mut(w)
            .zip(alpha.par_chunks_mut(w))
            .enumerate()
            .for_each(|(j, (crow, arow))| {
                let y = grid.y(j);
                for i in 0..w {
                    if let Some((c, a)) = f(i, j, grid.x(i), y) {
                        crow[i] = c;
                        arow[i] = a.clamp(0.0, 1.0);
                    }
                }
            });
        Layer { color, alpha }
    }
}

/// The picture being built: premultiplied linear light, back to front.
pub struct Canvas {
    pub grid: Grid,
    color: Vec<Linear>,
    alpha: Vec<f32>,
}

impl Canvas {
    pub fn new(grid: Grid) -> Canvas {
        Canvas {
            grid,
            color: vec![Linear::BLACK; grid.len()],
            alpha: vec![0.0; grid.len()],
        }
    }

    pub fn over(&mut self, layer: &Layer) {
        self.color
            .par_iter_mut()
            .zip(self.alpha.par_iter_mut())
            .zip(layer.color.par_iter().zip(layer.alpha.par_iter()))
            .for_each(|((dst, da), (src, &sa))| {
                if sa > 0.0 {
                    *dst = *src * sa + *dst * (1.0 - sa);
                    *da = sa + *da * (1.0 - sa);
                }
            });
    }

    /// Lays another picture of the same window over this one.
    pub fn lay(&mut self, top: &Canvas) {
        self.color
            .par_iter_mut()
            .zip(self.alpha.par_iter_mut())
            .zip(top.color.par_iter().zip(top.alpha.par_iter()))
            .for_each(|((dst, da), (src, &sa))| {
                if sa > 0.0 {
                    *dst = *src + *dst * (1.0 - sa);
                    *da = sa + *da * (1.0 - sa);
                }
            });
    }

    /// The picture turned `degrees` clockwise about a page position,
    /// resampled bilinearly.
    pub fn rotated(&self, degrees: f32, (px, py): (f32, f32)) -> Canvas {
        let g = self.grid;
        let (sin, cos) = degrees.to_radians().sin_cos();
        let mut out = Canvas::new(g);
        out.color
            .par_chunks_mut(g.w)
            .zip(out.alpha.par_chunks_mut(g.w))
            .enumerate()
            .for_each(|(j, (crow, arow))| {
                for i in 0..g.w {
                    // Where this pixel came from before the turn
                    let (dx, dy) = (g.x(i) - px, g.y(j) - py);
                    let sx = px + dx * cos + dy * sin;
                    let sy = py - dx * sin + dy * cos;
                    let fx = (sx - g.x0) * g.scale - 0.5;
                    let fy = (sy - g.y0) * g.scale - 0.5;
                    if fx < -1.0 || fy < -1.0 || fx > g.w as f32 || fy > g.h as f32 {
                        continue;
                    }
                    let (i0, j0) = (fx.floor(), fy.floor());
                    let (tx, ty) = (fx - i0, fy - j0);
                    let mut c = Linear::BLACK;
                    let mut a = 0.0;
                    for (di, dj, w) in [
                        (0, 0, (1.0 - tx) * (1.0 - ty)),
                        (1, 0, tx * (1.0 - ty)),
                        (0, 1, (1.0 - tx) * ty),
                        (1, 1, tx * ty),
                    ] {
                        let (si, sj) = (i0 as isize + di, j0 as isize + dj);
                        if si < 0 || sj < 0 || si >= g.w as isize || sj >= g.h as isize {
                            continue;
                        }
                        let k = sj as usize * g.w + si as usize;
                        c += self.color[k] * w;
                        a += self.alpha[k] * w;
                    }
                    crow[i] = c;
                    arow[i] = a;
                }
            });
        out
    }

    /// The same picture at half the resolution, averaged 2×2.
    pub fn halved(&self) -> Canvas {
        let g = self.grid;
        let half = Grid::window(g.scale / 2.0, (g.x0, g.y0), (g.w / 2, g.h / 2));
        let mut out = Canvas::new(half);
        for j in 0..half.h {
            for i in 0..half.w {
                let mut c = Linear::BLACK;
                let mut a = 0.0;
                for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let k = (2 * j + dj) * g.w + 2 * i + di;
                    c += self.color[k];
                    a += self.alpha[k];
                }
                out.color[j * half.w + i] = c * 0.25;
                out.alpha[j * half.w + i] = a * 0.25;
            }
        }
        out
    }

    pub fn jpeg(&self, quality: u8) -> Vec<u8> {
        let rgb: Vec<u8> = (0..self.grid.len())
            .flat_map(|k| self.developed(k))
            .collect();
        let mut out = Vec::with_capacity(64 * 1024);
        JpegEncoder::new_with_quality(&mut out, quality)
            .write_image(
                &rgb,
                self.grid.w as u32,
                self.grid.h as u32,
                ExtendedColorType::Rgb8,
            )
            .expect("an in-memory JPEG cannot fail to encode");
        out
    }

    pub fn png(&self) -> Vec<u8> {
        let rgba: Vec<u8> = (0..self.grid.len())
            .flat_map(|k| {
                let [r, g, b] = self.developed(k);
                [r, g, b, (self.alpha[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]
            })
            .collect();
        let mut out = Vec::with_capacity(96 * 1024);
        PngEncoder::new(&mut out)
            .write_image(
                &rgba,
                self.grid.w as u32,
                self.grid.h as u32,
                ExtendedColorType::Rgba8,
            )
            .expect("an in-memory PNG cannot fail to encode");
        out
    }

    #[cfg(test)]
    pub fn alpha(&self, k: usize) -> f32 {
        self.alpha[k]
    }

    /// A pixel as straight (un-premultiplied) sRGB.
    pub fn developed(&self, k: usize) -> [u8; 3] {
        let a = self.alpha[k];
        if a <= 1e-4 {
            return [0, 0, 0];
        }
        (self.color[k] * (1.0 / a)).encode()
    }
}

/// Smooth transitions between two values — the way every edge in the
/// picture is softened rather than cut.
pub struct Ramp;

impl Ramp {
    /// 0 at or below `a`, 1 at or beyond `b`, a smooth S between. `a` may be
    /// the larger for a falling ramp.
    pub fn smooth(a: f32, b: f32, v: f32) -> f32 {
        let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}
