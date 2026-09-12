//! The neck under the head and the shirt under the neck.
//!
//! A head shot is mostly shoulders: the reference library crops at the
//! chest with the shoulders running out of frame on both sides, so the
//! shirt here does the same rather than sitting inside the picture like a
//! bust on a plinth.

use super::canvas::{Blur, Canvas, PathBuilder};
use super::color::Rgb;
use super::geometry::Landmarks;
use super::tones::Tones;

pub struct Body;

impl Body {
    /// The neck, painted before the head so the jaw overlaps it.
    pub fn neck(c: &mut Canvas, l: &Landmarks, t: &Tones) {
        let cx = l.cx;
        let nh = l.neck_half;
        let top = l.neck_top;
        c.fill_ref(&l.neck_path, "ng", 1.0, Blur::Crisp);
        c.clip("nc");
        // The head's cast shadow: the strongest single photographic cue in
        // the lower half of a portrait — deep just under the jaw, gone by
        // the collar
        c.ellipse(
            cx,
            top + 10.0,
            nh + 6.0,
            13.0,
            t.skin_shadow,
            0.55,
            Blur::Broad,
            0.0,
        );
        c.ellipse(
            cx + 2.0,
            top + 5.0,
            nh + 4.0,
            7.0,
            t.skin_shadow,
            0.45,
            Blur::Soft,
            0.0,
        );
        // The cylinder: the far side turns into shadow, the near side has a
        // rim of light along the sternocleidomastoid
        c.ellipse(
            cx + nh - 2.0,
            top + 30.0,
            7.0,
            36.0,
            t.skin_dk2,
            0.42,
            Blur::Broad,
            0.0,
        );
        c.ellipse(
            cx - nh + 4.0,
            top + 30.0,
            5.0,
            34.0,
            t.skin_hi,
            0.22,
            Blur::Broad,
            0.0,
        );
        // Sternocleidomastoids: two ridges from behind the ear to the pit
        // of the throat
        for side in [-1.0f32, 1.0] {
            let d = PathBuilder::smooth_open(
                &[
                    (cx + side * (nh - 4.0), top + 6.0),
                    (cx + side * (nh - 8.0), top + 22.0),
                    (cx + side * 7.0, top + 40.0),
                ],
                0.2,
            );
            let (col, op) = if side < 0.0 {
                (t.skin_hi, 0.16)
            } else {
                (t.skin_dk2, 0.14)
            };
            c.stroke(&d, col, 4.0, op, Blur::Soft);
        }
        // Throat hollow and the larynx above it
        c.ellipse(cx, top + 40.0, 6.0, 4.0, t.skin_dk2, 0.16, Blur::Soft, 0.0);
        c.ellipse(
            cx - 1.0,
            top + 26.0,
            4.0,
            5.0,
            t.skin_hi,
            0.10,
            Blur::Soft,
            0.0,
        );
        c.close("g");
    }

    /// Shoulders in a jersey, with the collar hugging the neck. Everything
    /// here is the setting rather than the man, so a cutout never gets it.
    pub fn jersey(c: &mut Canvas, l: &Landmarks, _t: &Tones, jersey: (Rgb, Rgb, Rgb)) {
        let cx = l.cx;
        let nh = l.neck_half;
        let (light, base, dark) = jersey;
        // Collar-front height sits a fixed distance below the chin: a long
        // face pushes the shirt down so a real stretch of neck shows
        let ct = l.skull.chin + 7.0;
        let d = format!(
            "M-10 250 L-10 238 C28 226 58 216 {:.1} {ct:.1} Q{cx:.1} {:.1} {:.1} {ct:.1} C142 216 172 226 210 238 L210 250Z",
            cx - nh - 7.0,
            ct + 10.0,
            cx + nh + 7.0,
        );
        c.raw(&format!(
            r#"<defs><clipPath id="jc"><path d="{d}"/></clipPath></defs>"#
        ));
        c.fill_ref(&d, "jg", 1.0, Blur::Crisp);
        c.clip("jc");
        // Head and neck cast onto the chest
        c.ellipse(
            cx + 4.0,
            ct + 12.0,
            nh + 18.0,
            11.0,
            Rgb::BLACK,
            0.28,
            Blur::Broad,
            0.0,
        );
        // The shoulders roll away from the light on both sides
        c.ellipse(cx + 80.0, 250.0, 60.0, 34.0, dark, 0.45, Blur::Vast, 0.0);
        c.ellipse(cx - 84.0, 252.0, 56.0, 30.0, dark, 0.22, Blur::Vast, 0.0);
        c.ellipse(cx - 40.0, 226.0, 28.0, 8.0, light, 0.28, Blur::Broad, -12.0);
        // Fabric: a few soft folds pulling from the collar
        for (k, (fx, fy, rot)) in [
            (cx - 30.0, 236.0, 20.0f32),
            (cx + 26.0, 238.0, -16.0),
            (cx - 8.0, 244.0, 4.0),
        ]
        .into_iter()
        .enumerate()
        {
            let op = 0.30 - k as f32 * 0.05;
            c.ellipse(fx, fy, 3.0, 12.0, dark, op, Blur::Soft, rot);
        }
        c.close("g");
        // Crew collar — a ribbed band that IS the shirt's top edge
        let band = format!(
            "M{:.1} {ct:.1} Q{cx:.1} {:.1} {:.1} {ct:.1}",
            cx - nh - 7.0,
            ct + 10.0,
            cx + nh + 7.0,
        );
        c.stroke_butt(&band, dark, 6.0, 0.92, Blur::Crisp);
        c.stroke_butt(&band, base.mix(light, 0.5), 1.2, 0.55, Blur::Hair);
        // The band's own shadow onto the shirt below it
        let under = format!(
            "M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}",
            cx - nh - 5.0,
            ct + 4.0,
            ct + 14.0,
            cx + nh + 5.0,
            ct + 4.0,
        );
        c.stroke(&under, Rgb::BLACK, 3.0, 0.22, Blur::Soft);
    }
}
