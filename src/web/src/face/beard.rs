//! Facial hair, from a day's growth to a full beard.
//!
//! Everything here is clipped to the head, so the sides and the bottom of
//! each region are drawn deliberately outside the silhouette: the clip —
//! not hand-tuned insets — decides where the hair meets the jaw. Nothing
//! can float inside the cheek or spill onto the neck. The cheek line is the
//! one edge a beard shows, and it is never a curve: it is re-stroked with
//! the stubble speckle so it feathers out into skin the way growth does.

use super::canvas::{Blur, Canvas, PathBuilder};
use super::color::Rgb;
use super::geometry::Landmarks;
use super::identity::{Beard, Identity, Moustache};
use super::tones::Tones;

pub struct FacialHair;

/// The lower-face region shared by every style.
struct Region<'a> {
    l: &'a Landmarks,
    lip_hole: String,
}

impl Region<'_> {
    fn new(l: &Landmarks) -> Region<'_> {
        let cx = l.cx;
        let ms = &l.mouth_shape;
        let my = l.mouth;
        let lip_l = cx - ms.half + 0.5;
        let lip_r = cx + ms.half - 0.5;
        let lip_top = my - ms.upper + 0.2;
        // Only the lips stay bare, and the hole is lip-shaped rather than an
        // ellipse: the moustache then lands on the upper lip instead of
        // ringing the whole mouth with a punched-out band of skin
        let lip_hole = format!(
            "M{lip_l:.1} {my:.1} Q{:.1} {:.1} {:.1} {lip_top:.1} Q{cx:.1} {:.1} {:.1} {lip_top:.1} Q{:.1} {:.1} {lip_r:.1} {my:.1} Q{cx:.1} {:.1} {lip_l:.1} {my:.1}Z",
            cx - ms.half * 0.5,
            my - ms.upper - 0.8,
            cx - 3.0,
            my - ms.upper * 0.5,
            cx + 3.0,
            cx + ms.half * 0.5,
            my - ms.upper - 0.8,
            my + ms.lower * 2.2,
        );
        Region { l, lip_hole }
    }

    /// The filled region (lips punched out) plus its top edge on its own,
    /// so the cheek line can be re-stroked as a speckled fringe. `sb` is
    /// the sideburn junction height and `mst` the top of the moustache.
    fn shape(&self, sb: f32, mst: f32) -> (String, String) {
        let l = self.l;
        let cx = l.cx;
        let s = &l.skull;
        let ms = &l.mouth_shape;
        let mw = ms.half;
        let my = l.mouth;
        let bx_l = cx - s.sub - 14.0;
        let bx_r = cx + s.sub + 14.0;
        let bx_b = s.chin + 16.0;
        let gate_l = cx - mw - 3.0;
        let gate_r = cx + mw + 3.0;
        let gate_y = my - 3.0;
        let cheek_c = sb + (gate_y - sb) * 0.5;
        let in_l = bx_l + 10.0;
        let in_r = bx_r - 10.0;
        let out_l = gate_l - 13.0;
        let out_r = gate_r + 13.0;
        // Moustache: wings droop out to the mouth corners and the top dips at
        // the philtrum, following the base of the nose
        let mo_top = mst + 0.5;
        let mo_mid = mst + 4.5;
        let mo_drop = mst + 6.0;
        let mo_out_l = cx - mw * 0.45;
        let mo_out_r = cx + mw * 0.45;
        let mo_sh_l = cx - mw * 0.72;
        let mo_sh_r = cx + mw * 0.72;
        let top = format!(
            "C{in_r:.1} {cheek_c:.1} {out_r:.1} {gate_y:.1} {gate_r:.1} {gate_y:.1} \
             C{gate_r:.1} {mo_drop:.1} {mo_sh_r:.1} {mst:.1} {mo_out_r:.1} {mo_top:.1} \
             Q{cx:.1} {mo_mid:.1} {mo_out_l:.1} {mo_top:.1} \
             C{mo_sh_l:.1} {mst:.1} {gate_l:.1} {mo_drop:.1} {gate_l:.1} {gate_y:.1} \
             C{out_l:.1} {gate_y:.1} {in_l:.1} {cheek_c:.1} {bx_l:.1} {sb:.1}"
        );
        (
            format!(
                "M{bx_l:.1} {sb:.1} C{bx_l:.1} {:.1} {bx_l:.1} {bx_b:.1} {cx:.1} {bx_b:.1} \
                 C{bx_r:.1} {bx_b:.1} {bx_r:.1} {:.1} {bx_r:.1} {sb:.1} {top}Z {}",
                s.jaw_y, s.jaw_y, self.lip_hole
            ),
            format!("M{bx_r:.1} {sb:.1} {top}"),
        )
    }
}

impl FacialHair {
    pub fn paint(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, age: u8) {
        let cx = l.cx;
        let s = &l.skull;
        let ms = &l.mouth_shape;
        let my = l.mouth;
        let ny = l.nose;
        let mw = ms.half;
        let maturity = l.maturity;
        let region = Region::new(l);
        let hair = t.hair_greyed(id.grey);
        let stubble = hair.shade(0.72);
        let shadow_col = t.beard_shadow;
        let hair_hi = hair.lift(0.22);
        let hair_dk = hair.shade(0.55);
        let sb_base = s.zygo_y + 22.0;

        c.clip("hc");

        let speckle = |c: &mut Canvas, d: &str, col: Rgb, op: f32| {
            c.raw(&format!(
                r#"<path d="{d}" fill-rule="evenodd" fill="{col}" filter="url(#stb)" opacity="{op:.2}"/>"#
            ));
        };
        let fringe = |c: &mut Canvas, d: &str, col: Rgb, w: f32, op: f32| {
            c.raw(&format!(
                r#"<path d="{d}" fill="none" stroke="{col}" stroke-width="{w:.1}" filter="url(#stb)" opacity="{op:.2}"/>"#
            ));
        };

        if let Some(beard) = id.beard {
            match beard {
                Beard::Stubble => {
                    let (reg, edge) = region.shape(sb_base + 3.0, ny + 8.0);
                    c.fill_evenodd(&reg, shadow_col, 0.22 + maturity * 0.1, Blur::Broad);
                    speckle(c, &reg, shadow_col, 0.62 + maturity * 0.18);
                    fringe(c, &edge, shadow_col, 5.0, 0.35);
                }
                Beard::Boxed => {
                    let (reg, edge) = region.shape(sb_base - 1.0, ny + 7.0);
                    c.fill_evenodd(&reg, hair, 0.26, Blur::Broad);
                    c.fill_evenodd(&reg, hair, 0.50, Blur::Fine);
                    speckle(c, &reg, stubble, 0.70);
                    fringe(c, &edge, stubble, 4.5, 0.50);
                    Self::grain(c, l, id, &reg, hair_hi, hair_dk, 120);
                }
                Beard::Full => {
                    let (reg, edge) = region.shape(sb_base - 6.0, ny + 4.0);
                    c.fill_evenodd(&reg, hair, 0.40, Blur::Soft);
                    c.fill_evenodd(&reg, hair, 0.62, Blur::Fine);
                    speckle(c, &reg, hair_hi, 0.20);
                    fringe(c, &edge, hair, 5.5, 0.55);
                    Self::grain(c, l, id, &reg, hair_hi, hair_dk, 200);
                    // Volume: the chin front catches the key light, the jaw
                    // underside stays in shadow
                    c.ellipse(
                        cx - 3.0,
                        s.chin - 13.0,
                        mw * 0.95,
                        9.0,
                        hair_hi,
                        0.18,
                        Blur::Broad,
                        0.0,
                    );
                    let under = PathBuilder::arc(
                        (cx - s.jaw, s.jaw_y),
                        (cx, s.chin + 8.0),
                        (cx + s.jaw, s.jaw_y),
                    );
                    c.stroke(&under, hair_dk, 8.0, 0.35, Blur::Broad);
                    c.ellipse(
                        cx + s.sub * 0.7,
                        s.sub_y + 8.0,
                        10.0,
                        16.0,
                        hair_dk,
                        0.30,
                        Blur::Broad,
                        0.0,
                    );
                }
                Beard::Goatee => {
                    // The moustache wraps the mouth corners into a chin patch,
                    // one connected ring; cheeks stay clean
                    let g_l = cx - mw - 4.0;
                    let g_r = cx + mw + 4.0;
                    let g_top = ny + 6.0;
                    let g_bot = s.chin + 1.0;
                    let g_side = my + 2.0;
                    let g_mid = g_side + (g_bot - g_side) * 0.55;
                    let goatee = format!(
                        "M{g_l:.1} {g_side:.1} C{g_l:.1} {:.1} {:.1} {g_top:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} \
                         C{:.1} {g_top:.1} {g_r:.1} {:.1} {g_r:.1} {g_side:.1} \
                         C{:.1} {g_mid:.1} {:.1} {g_bot:.1} {cx:.1} {g_bot:.1} \
                         C{:.1} {g_bot:.1} {:.1} {g_mid:.1} {g_l:.1} {g_side:.1}Z {}",
                        g_top + 7.0,
                        cx - mw * 0.78,
                        cx - mw * 0.42,
                        g_top + 0.5,
                        g_top + 5.0,
                        cx + mw * 0.42,
                        g_top + 0.5,
                        cx + mw * 0.78,
                        g_top + 7.0,
                        cx + mw * 0.95,
                        cx + mw * 0.62,
                        cx - mw * 0.62,
                        cx - mw * 0.95,
                        region.lip_hole,
                    );
                    c.fill_evenodd(&goatee, hair, 0.36, Blur::Soft);
                    c.fill_evenodd(&goatee, hair, 0.62, Blur::Fine);
                    speckle(c, &goatee, stubble, 0.70);
                    Self::grain(c, l, id, &goatee, hair_hi, hair_dk, 40);
                }
                Beard::Chinstrap => {
                    let strap = format!(
                        "M{:.1} {:.1} C{:.1} {:.1} {:.1} {:.1} {cx:.1} {:.1} C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
                        cx - s.sub - 14.0,
                        s.zygo_y + 20.0,
                        cx - s.jaw - 6.0,
                        s.jaw_y,
                        cx - s.chin_half - 4.0,
                        s.chin + 2.0,
                        s.chin + 3.0,
                        cx + s.chin_half + 4.0,
                        s.chin + 2.0,
                        cx + s.jaw + 6.0,
                        s.jaw_y,
                        cx + s.sub + 14.0,
                        s.zygo_y + 20.0,
                    );
                    c.stroke(&strap, hair, 7.0, 0.45, Blur::Fine);
                    fringe(c, &strap, stubble, 8.0, 0.75);
                }
            }
        } else if age >= 22
            && !(id.phenotype.epicanthic() || id.phenotype == shared::Phenotype::Andean)
        {
            // Five o'clock shadow — deepens with maturity; sparse-growth
            // classes never shadow the jaw
            let (reg, _) = region.shape(sb_base + 6.0, ny + 9.0);
            c.fill_evenodd(&reg, shadow_col, 0.10 + maturity * 0.08, Blur::Vast);
            speckle(c, &reg, shadow_col, 0.16 + maturity * 0.22);
        }

        // Every grown style except the chinstrap already carries its own
        // moustache band; a standalone one would only double the ink over
        // the philtrum
        let has_band = matches!(
            id.beard,
            Some(Beard::Stubble | Beard::Boxed | Beard::Full | Beard::Goatee)
        );
        if let (Some(m), false) = (id.moustache, has_band) {
            let (k, h, op): (f32, f32, f32) = match m {
                Moustache::Thin => (0.90, 2.4, 0.32),
                Moustache::Chevron => (1.02, 6.0, 0.75),
                Moustache::Handlebar => (1.20, 5.5, 0.70),
                Moustache::Walrus => (1.08, 7.5, 0.70),
            };
            let w = mw * k;
            let d = format!(
                "M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}Z",
                cx - w,
                my - ms.upper - 0.4,
                my - ms.upper - h - 1.0,
                cx + w,
                my - ms.upper - 0.4,
                my - ms.upper - 1.6,
                cx - w,
                my - ms.upper - 0.4,
            );
            if m == Moustache::Thin {
                // A pencil line of growth, not a mass
                c.fill(&d, hair, op, Blur::Soft);
            } else {
                c.fill(&d, hair, op * 0.55, Blur::Fine);
                c.raw(&format!(
                    r#"<path d="{d}" fill="{stubble}" filter="url(#stb)" opacity="{op:.2}"/>"#
                ));
            }
            if m == Moustache::Handlebar {
                for dir in [-1.0f32, 1.0] {
                    let hx = cx + dir * w;
                    let d = format!(
                        "M{hx:.1} {:.1} q{:.1} 2.0 {:.1} 5.0",
                        my - 1.5,
                        dir * 2.4,
                        dir * 3.2
                    );
                    c.stroke(&d, hair, 1.6, 0.6, Blur::Fine);
                }
            }
        }

        // Grey threads through a beard before it does the hair
        if id.grey > 0.05 && id.beard.is_some() {
            let (reg, _) = region.shape(sb_base - 2.0, ny + 5.0);
            speckle(c, &reg, t.grey_hair, (id.grey * 1.1).min(0.5));
        }

        c.close("g");
    }

    /// Short downward strokes over a beard mass so it reads as hair, not
    /// paint. Clipped to the region.
    #[allow(clippy::too_many_arguments)]
    fn grain(c: &mut Canvas, l: &Landmarks, id: &Identity, reg: &str, hi: Rgb, dk: Rgb, n: usize) {
        let cx = l.cx;
        let s = &l.skull;
        c.raw(&format!(
            r#"<clipPath id="bcl" clip-rule="evenodd"><path d="{reg}"/></clipPath>"#
        ));
        c.clip("bcl");
        for k in 0..n {
            let u = id.jitter_signed(k, 100);
            let v = id.jitter(k, 101);
            let x = cx + u * (s.sub + 4.0);
            let y = l.nose + 4.0 + v * (s.chin + 10.0 - l.nose);
            let len = 2.5 + id.jitter(k, 102) * 3.0;
            let d = format!(
                "M{x:.1} {y:.1} q{:.1} {:.1} {:.1} {len:.1}",
                u * 1.2,
                len * 0.5,
                u * 2.0
            );
            let (col, op) = if id.jitter(k, 103) < 0.4 {
                (hi, 0.25 + id.jitter(k, 104) * 0.25)
            } else {
                (dk, 0.30 + id.jitter(k, 104) * 0.30)
            };
            c.stroke_butt(&d, col, 0.45 + id.jitter(k, 105) * 0.4, op, Blur::Hair);
        }
        c.close("g");
    }
}
