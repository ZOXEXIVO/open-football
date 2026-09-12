//! A thin writer over the SVG string the portrait is built into.
//!
//! The painter lays down hundreds of soft shapes — blurred ellipses and
//! strokes with an opacity — and the whole point of this type is that each of
//! them is one call with the numbers in it, not a `format!` with a template
//! of attributes to get wrong. Every coordinate is written with one decimal:
//! more is invisible and doubles the file.

use std::f32::consts::SQRT_2;
use std::fmt::{Display, Write};

use super::color::Rgb;

/// The softness of an edge. `Crisp` means no softening at all, `Hair` is
/// painted as geometry (see [`Feather`]), and the rest are the Gaussian blur
/// filters the defs block declares.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Blur {
    Crisp,
    /// σ 0.6 — takes the vector edge off a line. Not a filter: the shape is
    /// laid down as concentric copies instead, see [`Feather`].
    Hair,
    /// σ 1.2 — a crease, a lash line, a nostril
    Fine,
    /// σ 2.4 — a fold, a small shadow
    Soft,
    /// σ 4.5 — a shading plane
    Broad,
    /// σ 8 — the turn of the whole head
    Vast,
}

impl Blur {
    fn attr(self) -> &'static str {
        match self {
            Blur::Crisp | Blur::Hair => "",
            Blur::Fine => r#" filter="url(#b1)""#,
            Blur::Soft => r#" filter="url(#b2)""#,
            Blur::Broad => r#" filter="url(#b3)""#,
            Blur::Vast => r#" filter="url(#b4)""#,
        }
    }

    /// The defs the attributes above point at. The regions are the whole
    /// page in user space rather than a margin round each element: a margin
    /// is a percentage of the element's own box, and a wide soft stroke on a
    /// nearly flat path has a box a few units tall, so its blur was cut off
    /// in hard bands above and below.
    pub fn defs() -> &'static str {
        r#"<filter id="b1" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="1.2"/></filter><filter id="b2" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="2.4"/></filter><filter id="b3" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="4.5"/></filter><filter id="b4" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="8"/></filter>"#
    }
}

/// How `Blur::Hair` is painted without a filter.
///
/// WebKit draws an SVG that arrived through `<img>` in a document whose
/// device scale is 1 whatever the screen is, and sizes every filter's buffer
/// from that — so on a phone each filtered element is a 1x bitmap stretched
/// threefold. The wider blurs survive that, being soft anyway, but σ0.6 is
/// the fine detail — strands, brows, stubble, lash and lip lines — and it
/// turned to mush while the unfiltered iris beside it stayed sharp. A blur
/// that small can be faked with geometry: the shape is laid down as
/// concentric copies (a step outside the edge, the edge, a step inside it)
/// whose stacked coverages follow the blurred profile at the middle of each
/// band, and geometry is rasterised at whatever scale the screen has.
pub struct Feather;

impl Feather {
    /// The blur being imitated.
    const SIGMA: f32 = 0.6;
    /// How far each copy reaches past the one inside it — 1.25σ, so the
    /// outer copy ends about where the true blur has faded to a few percent.
    const STEP: f32 = 0.75;

    /// Gauss error function (Abramowitz–Stegun 7.1.26, error under 2e-7).
    fn erf(x: f32) -> f32 {
        let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
        let poly = ((((1.061_405_4 * t - 1.453_152) * t + 1.421_413_8) * t - 0.284_496_72) * t
            + 0.254_829_6)
            * t;
        let y = 1.0 - poly * (-x * x).exp();
        if x >= 0.0 { y } else { -y }
    }

    /// Coverage a full-opacity bar of half-width `r` keeps at distance `x`
    /// from its centre line after the blur.
    fn bar(r: f32, x: f32) -> f32 {
        let k = 1.0 / (Self::SIGMA * SQRT_2);
        0.5 * (Self::erf((x + r) * k) - Self::erf((x - r) * k))
    }

    /// Coverage the blur leaves along a shape's long axis at its centre —
    /// 1 for a stroke, less for an ellipse the blur can reach across.
    fn across(half_extent: f32) -> f32 {
        Self::erf(half_extent / (Self::SIGMA * SQRT_2))
    }

    /// The opacities of three same-paint strokes straddling a long straight
    /// edge, each half a step wider than the last, that together carry the
    /// blur's tail outside a shape too big to be copied at other sizes.
    /// Painted widest first; the halves inside the shape are hidden by its
    /// fill.
    fn rim(opacity: f32) -> [f32; 3] {
        let k = 1.0 / (Self::SIGMA * SQRT_2);
        let spill = |x: f32| 0.5 * (1.0 - Self::erf(x * k)) * opacity;
        let near = spill(Self::STEP * 0.25);
        let mid = spill(Self::STEP * 0.75);
        let far = spill(Self::STEP * 1.25);
        [far, Self::on_top(far, mid), Self::on_top(mid, near)]
    }

    /// The opacities of the outer, edge and inner copies of a shape whose
    /// thinnest half-extent is `r` (a stroke's half-width, an ellipse's minor
    /// radius) and whose blurred coverage along the other axis is `across`.
    /// Painted in that order, the stack reproduces the blurred coverage at
    /// the middle of each band; the inner copy is `None` when the shape is
    /// too thin to have one.
    fn layers(r: f32, across: f32, opacity: f32) -> (f32, f32, Option<f32>) {
        let peak = across * Self::bar(r, 0.0) * opacity;
        let outer = across * Self::bar(r, r + Self::STEP / 2.0) * opacity;
        if r > Self::STEP + 0.05 {
            let band = across * Self::bar(r, r - Self::STEP / 2.0) * opacity;
            (
                outer,
                Self::on_top(outer, band),
                Some(Self::on_top(band, peak)),
            )
        } else {
            (outer, Self::on_top(outer, peak), None)
        }
    }

    /// The opacity a same-coloured layer needs over coverage `below` to
    /// bring it up to `target`.
    fn on_top(below: f32, target: f32) -> f32 {
        if below >= 0.999 {
            return 0.0;
        }
        ((target - below) / (1.0 - below)).clamp(0.0, 1.0)
    }
}

pub struct Canvas {
    s: String,
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas {
            s: String::with_capacity(64 * 1024),
        }
    }

    pub fn raw(&mut self, text: &str) {
        self.s.push_str(text);
    }

    pub fn finish(self) -> String {
        self.s
    }

    pub fn close(&mut self, tag: &str) {
        self.s.push_str("</");
        self.s.push_str(tag);
        self.s.push('>');
    }

    /// `<g clip-path="url(#id)">` — pair with `close("g")`.
    pub fn clip(&mut self, id: &str) {
        let _ = write!(self.s, r#"<g clip-path="url(#{id})">"#);
    }

    /// A filled ellipse, optionally blurred and rotated about its centre.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: Rgb,
        opacity: f32,
        soft: Blur,
        rotate: f32,
    ) {
        self.ellipse_paint(cx, cy, rx, ry, fill, opacity, soft, rotate);
    }

    /// A filled ellipse with a gradient or pattern reference as its fill.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse_ref(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill_ref: &str,
        opacity: f32,
        soft: Blur,
        rotate: f32,
    ) {
        let paint = format!("url(#{fill_ref})");
        self.ellipse_paint(cx, cy, rx, ry, paint, opacity, soft, rotate);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Rgb, opacity: f32, soft: Blur) {
        self.ellipse(cx, cy, r, r, fill, opacity, soft, 0.0);
    }

    /// A filled path.
    pub fn fill(&mut self, d: &str, fill: Rgb, opacity: f32, soft: Blur) {
        self.fill_paint(d, "", fill, opacity, soft);
    }

    /// A filled path with an even-odd rule (for punched-out holes).
    pub fn fill_evenodd(&mut self, d: &str, fill: Rgb, opacity: f32, soft: Blur) {
        self.fill_paint(d, r#" fill-rule="evenodd""#, fill, opacity, soft);
    }

    /// A filled path whose fill is a gradient/pattern/filter reference.
    pub fn fill_ref(&mut self, d: &str, fill_ref: &str, opacity: f32, soft: Blur) {
        let paint = format!("url(#{fill_ref})");
        self.fill_paint(d, "", paint, opacity, soft);
    }

    /// A stroked, unfilled path with round caps.
    pub fn stroke(&mut self, d: &str, color: Rgb, width: f32, opacity: f32, soft: Blur) {
        self.stroke_paint(d, color, width, opacity, soft, r#" stroke-linecap="round""#);
    }

    /// A stroked path where the caps stay square — for hair strands that
    /// should end sharp.
    pub fn stroke_butt(&mut self, d: &str, color: Rgb, width: f32, opacity: f32, soft: Blur) {
        self.stroke_paint(d, color, width, opacity, soft, "");
    }

    /// A rectangle, filled with a colour or a reference.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &str, opacity: f32) {
        let _ = write!(
            self.s,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="{fill}""#
        );
        self.tail(opacity, Blur::Crisp, 0.0, 0.0, 0.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipse_paint(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        paint: impl Display,
        opacity: f32,
        soft: Blur,
        rotate: f32,
    ) {
        if opacity <= 0.002 || rx <= 0.0 || ry <= 0.0 {
            return;
        }
        if soft != Blur::Hair {
            self.ellipse_layer(cx, cy, rx, ry, &paint, opacity, soft, rotate);
            return;
        }
        let (rmin, rmax) = if rx < ry { (rx, ry) } else { (ry, rx) };
        let (outer, edge, inner) = Feather::layers(rmin, Feather::across(rmax), opacity);
        let step = Feather::STEP;
        self.ellipse_layer(
            cx,
            cy,
            rx + step,
            ry + step,
            &paint,
            outer,
            Blur::Crisp,
            rotate,
        );
        self.ellipse_layer(cx, cy, rx, ry, &paint, edge, Blur::Crisp, rotate);
        if let Some(inner) = inner {
            self.ellipse_layer(
                cx,
                cy,
                rx - step,
                ry - step,
                &paint,
                inner,
                Blur::Crisp,
                rotate,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipse_layer(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        paint: &impl Display,
        opacity: f32,
        soft: Blur,
        rotate: f32,
    ) {
        if opacity <= 0.002 || rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let _ = write!(
            self.s,
            r#"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{rx:.1}" ry="{ry:.1}" fill="{paint}""#,
        );
        self.tail(opacity, soft, rotate, cx, cy);
    }

    /// A path cannot be grown or shrunk, so a `Blur::Hair` fill is softened
    /// with a same-paint stroke straddling its edge: the half outside
    /// carries the blur's tail, the half inside darkens the rim slightly
    /// past what the blur would, which at the opacities this is used at is
    /// invisible.
    /// A path cannot be copied at other sizes the way an ellipse can, so a
    /// `Blur::Hair` fill gets its tail from same-paint strokes straddling
    /// the edge, laid down before the fill covers their inner halves. At an
    /// opacity below 1 the rim just inside the edge ends up a little darker
    /// than the blur would leave it, which nothing paints today.
    fn fill_paint(&mut self, d: &str, rule: &str, paint: impl Display, opacity: f32, soft: Blur) {
        if opacity <= 0.002 {
            return;
        }
        if soft == Blur::Hair {
            let closed = d.ends_with(['Z', 'z']);
            let close = if closed { "" } else { "Z" };
            for (reach, alpha) in [3.0, 2.0, 1.0].into_iter().zip(Feather::rim(opacity)) {
                if alpha <= 0.002 {
                    continue;
                }
                let _ = write!(
                    self.s,
                    r#"<path d="{d}{close}" fill="none" stroke="{paint}" stroke-width="{:.2}" stroke-opacity="{alpha:.3}" stroke-linejoin="round"/>"#,
                    reach * Feather::STEP
                );
            }
        }
        let _ = write!(self.s, r#"<path d="{d}"{rule} fill="{paint}""#);
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    fn stroke_paint(
        &mut self,
        d: &str,
        color: Rgb,
        width: f32,
        opacity: f32,
        soft: Blur,
        cap: &str,
    ) {
        if opacity <= 0.002 || width <= 0.0 {
            return;
        }
        if soft != Blur::Hair {
            self.stroke_layer(d, color, width, opacity, soft, cap);
            return;
        }
        let (outer, edge, inner) = Feather::layers(width / 2.0, 1.0, opacity);
        let step = 2.0 * Feather::STEP;
        self.stroke_layer(d, color, width + step, outer, Blur::Crisp, cap);
        self.stroke_layer(d, color, width, edge, Blur::Crisp, cap);
        if let Some(inner) = inner {
            self.stroke_layer(d, color, width - step, inner, Blur::Crisp, cap);
        }
    }

    fn stroke_layer(
        &mut self,
        d: &str,
        color: Rgb,
        width: f32,
        opacity: f32,
        soft: Blur,
        cap: &str,
    ) {
        if opacity <= 0.002 || width <= 0.0 {
            return;
        }
        let _ = write!(
            self.s,
            r#"<path d="{d}" fill="none" stroke="{color}" stroke-width="{width:.2}"{cap}"#,
        );
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    fn tail(&mut self, opacity: f32, soft: Blur, rotate: f32, cx: f32, cy: f32) {
        if opacity < 0.999 {
            let _ = write!(self.s, r#" opacity="{opacity:.3}""#);
        }
        self.s.push_str(soft.attr());
        if rotate.abs() > 0.01 {
            let _ = write!(
                self.s,
                r#" transform="rotate({rotate:.1} {cx:.1} {cy:.1})""#
            );
        }
        self.s.push_str("/>");
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas::new()
    }
}

/// Path-string helpers: the painter thinks in points, the SVG wants text.
pub struct PathBuilder;

impl PathBuilder {
    /// A closed, C1-smooth curve through `pts` (Catmull-Rom converted to
    /// cubic Béziers). `tension` 0 is the classic rounded fit; toward 1 the
    /// curve hugs the polygon. This is how every organic silhouette here is
    /// made — the head, the neck, the hair — so a morph is a matter of moving
    /// points, never of re-deriving control handles by hand.
    pub fn smooth_closed(pts: &[(f32, f32)], tension: f32) -> String {
        let n = pts.len();
        if n < 3 {
            return String::new();
        }
        let k = (1.0 - tension) / 6.0;
        let mut d = String::with_capacity(n * 40);
        let _ = write!(d, "M{:.1} {:.1}", pts[0].0, pts[0].1);
        for i in 0..n {
            let p0 = pts[(i + n - 1) % n];
            let p1 = pts[i];
            let p2 = pts[(i + 1) % n];
            let p3 = pts[(i + 2) % n];
            let c1 = (p1.0 + (p2.0 - p0.0) * k, p1.1 + (p2.1 - p0.1) * k);
            let c2 = (p2.0 - (p3.0 - p1.0) * k, p2.1 - (p3.1 - p1.1) * k);
            let _ = write!(
                d,
                " C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
                c1.0, c1.1, c2.0, c2.1, p2.0, p2.1
            );
        }
        d.push('Z');
        d
    }

    /// An open C1-smooth curve through `pts`.
    pub fn smooth_open(pts: &[(f32, f32)], tension: f32) -> String {
        let n = pts.len();
        if n < 2 {
            return String::new();
        }
        let k = (1.0 - tension) / 6.0;
        let mut d = String::with_capacity(n * 40);
        let _ = write!(d, "M{:.1} {:.1}", pts[0].0, pts[0].1);
        for i in 0..n - 1 {
            let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
            let p1 = pts[i];
            let p2 = pts[i + 1];
            let p3 = if i + 2 < n { pts[i + 2] } else { pts[n - 1] };
            let c1 = (p1.0 + (p2.0 - p0.0) * k, p1.1 + (p2.1 - p0.1) * k);
            let c2 = (p2.0 - (p3.0 - p1.0) * k, p2.1 - (p3.1 - p1.1) * k);
            let _ = write!(
                d,
                " C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
                c1.0, c1.1, c2.0, c2.1, p2.0, p2.1
            );
        }
        d
    }

    /// A quadratic arc from `a` to `b` bowing through control `c`.
    pub fn arc(a: (f32, f32), c: (f32, f32), b: (f32, f32)) -> String {
        format!(
            "M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}",
            a.0, a.1, c.0, c.1, b.0, b.1
        )
    }

    /// A straight segment.
    pub fn line(a: (f32, f32), b: (f32, f32)) -> String {
        format!("M{:.1} {:.1} L{:.1} {:.1}", a.0, a.1, b.0, b.1)
    }

    /// Mirrors a point list across `cx`, reversed so a right-side outline
    /// continues into its left-side twin.
    pub fn mirrored(cx: f32, right: &[(f32, f32)]) -> Vec<(f32, f32)> {
        right
            .iter()
            .rev()
            .map(|(x, y)| (2.0 * cx - x, *y))
            .collect()
    }
}
