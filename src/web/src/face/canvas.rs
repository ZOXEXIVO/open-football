//! A thin writer over the SVG string the portrait is built into.
//!
//! The painter lays down hundreds of soft shapes — blurred ellipses and
//! strokes with an opacity — and the whole point of this type is that each of
//! them is one call with the numbers in it, not a `format!` with a template
//! of attributes to get wrong. Every coordinate is written with one decimal:
//! more is invisible and doubles the file.

use std::fmt::Write;

use super::color::Rgb;

/// The softness of an edge, as one of the Gaussian blur filters the defs
/// block declares. `Crisp` means no filter at all.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Blur {
    Crisp,
    /// σ 0.6 — takes the vector edge off a line
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
            Blur::Crisp => "",
            Blur::Hair => r#" filter="url(#b0)""#,
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
        r#"<filter id="b0" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="0.6"/></filter><filter id="b1" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="1.2"/></filter><filter id="b2" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="2.4"/></filter><filter id="b3" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="4.5"/></filter><filter id="b4" x="-40" y="-40" width="280" height="330" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="8"/></filter>"#
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
        if opacity <= 0.002 || rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let _ = write!(
            self.s,
            r#"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{rx:.1}" ry="{ry:.1}" fill="{fill}""#,
        );
        self.tail(opacity, soft, rotate, cx, cy);
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
        let _ = write!(
            self.s,
            r#"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{rx:.1}" ry="{ry:.1}" fill="url(#{fill_ref})""#,
        );
        self.tail(opacity, soft, rotate, cx, cy);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Rgb, opacity: f32, soft: Blur) {
        self.ellipse(cx, cy, r, r, fill, opacity, soft, 0.0);
    }

    /// A filled path.
    pub fn fill(&mut self, d: &str, fill: Rgb, opacity: f32, soft: Blur) {
        if opacity <= 0.002 {
            return;
        }
        let _ = write!(self.s, r#"<path d="{d}" fill="{fill}""#);
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    /// A filled path with an even-odd rule (for punched-out holes).
    pub fn fill_evenodd(&mut self, d: &str, fill: Rgb, opacity: f32, soft: Blur) {
        if opacity <= 0.002 {
            return;
        }
        let _ = write!(self.s, r#"<path d="{d}" fill-rule="evenodd" fill="{fill}""#);
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    /// A filled path whose fill is a gradient/pattern/filter reference.
    pub fn fill_ref(&mut self, d: &str, fill_ref: &str, opacity: f32, soft: Blur) {
        let _ = write!(self.s, r#"<path d="{d}" fill="url(#{fill_ref})""#);
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    /// A stroked, unfilled path with round caps.
    pub fn stroke(&mut self, d: &str, color: Rgb, width: f32, opacity: f32, soft: Blur) {
        if opacity <= 0.002 || width <= 0.0 {
            return;
        }
        let _ = write!(
            self.s,
            r#"<path d="{d}" fill="none" stroke="{color}" stroke-width="{width:.2}" stroke-linecap="round""#,
        );
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    /// A stroked path where the caps stay square — for hair strands that
    /// should end sharp.
    pub fn stroke_butt(&mut self, d: &str, color: Rgb, width: f32, opacity: f32, soft: Blur) {
        if opacity <= 0.002 || width <= 0.0 {
            return;
        }
        let _ = write!(
            self.s,
            r#"<path d="{d}" fill="none" stroke="{color}" stroke-width="{width:.2}""#,
        );
        self.tail(opacity, soft, 0.0, 0.0, 0.0);
    }

    /// A rectangle, filled with a colour or a reference.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &str, opacity: f32) {
        let _ = write!(
            self.s,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="{fill}""#
        );
        self.tail(opacity, Blur::Crisp, 0.0, 0.0, 0.0);
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
