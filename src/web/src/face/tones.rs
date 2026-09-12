//! The palette one player is painted in, derived from his three base
//! colours.
//!
//! Skin under studio light is not one colour lightened and darkened: its
//! shadows go warm and red where blood shows through, its highlights go
//! toward the light's own colour, the lower face on a man is cooler than the
//! forehead, and the lips are the skin with more blood in them. Every tone
//! here is written as a departure from the base in that direction, so a
//! change of complexion takes the whole set with it.

use super::color::Rgb;

pub struct Tones {
    pub skin: Rgb,
    /// The plane facing the key light
    pub skin_hi: Rgb,
    /// The specular sheen on the bridge, forehead and chin
    pub skin_spec: Rgb,
    /// Flush: cheeks, nose, ears
    pub skin_warm: Rgb,
    /// Half-tone, the plane turning away
    pub skin_dk: Rgb,
    /// Core shadow
    pub skin_dk2: Rgb,
    /// Occlusion: nostrils, inner canthus, under the chin
    pub skin_shadow: Rgb,
    /// The cooler lower face of a shaven man, and the under-eye
    pub skin_cool: Rgb,
    pub lip: Rgb,
    pub lip_dk: Rgb,
    pub lip_hi: Rgb,
    pub mouth_line: Rgb,
    pub hair: Rgb,
    pub hair_hi: Rgb,
    pub hair_dk: Rgb,
    pub grey_hair: Rgb,
    pub iris: Rgb,
    pub iris_hi: Rgb,
    pub iris_dk: Rgb,
    pub iris_rim: Rgb,
    pub sclera: Rgb,
    pub sclera_dk: Rgb,
    pub canthus: Rgb,
    /// The skin under a beard
    pub beard_shadow: Rgb,
    pub luma: f32,
}

impl Tones {
    pub fn derive(skin_hex: &str, hair_hex: &str, eye_hex: &str, redness: f32) -> Tones {
        let skin = Rgb::hex(skin_hex);
        let hair = Rgb::hex(hair_hex);
        let iris = Rgb::hex(eye_hex);
        let luma = skin.luma();

        // Lighter skin flushes visibly; on dark skin the same blood reads as
        // a warmer, not a pinker, tone
        let flush = 0.16 + 0.22 * luma + redness * 0.12;
        let skin_warm = skin.toward("#D24E38", flush);
        let skin_hi = skin.lift(0.20).toward("#FFF1E0", 0.10);
        let skin_spec = skin.lift(0.62).toward("#FFF8F0", 0.30);
        let skin_dk = skin.shade(0.80).toward("#B5603A", 0.10);
        let skin_dk2 = skin.shade(0.60).toward("#7A3520", 0.14);
        let skin_shadow = skin.shade(0.40).toward("#3A1A1A", 0.24);
        let skin_cool = skin.shade(0.90).toward("#586878", 0.15);

        let pink = 0.22 + 0.46 * luma;
        let lip = skin.toward("#A94F58", pink).desaturate(0.08);
        let lip_dk = lip.shade(0.72);
        let lip_hi = lip.lift(0.22);
        let mouth_line = lip.shade(0.40).toward("#2A1015", 0.35);

        let hair_hi = hair.lift(0.22).toward("#C9A574", 0.08);
        let hair_dk = hair.shade(0.55);
        let grey_hair = Rgb::hex("#B9B5AE");

        let iris_hi = iris.lift(0.45).saturate(0.30);
        let iris_dk = iris.shade(0.60);
        let iris_rim = iris.shade(0.28);
        let sclera = Rgb::hex("#F1ECE5").mix(skin, 0.08 + 0.18 * luma);
        let sclera_dk = sclera.shade(0.70).toward("#8C7E78", 0.30);
        let canthus = Rgb::hex("#C8736C").mix(skin, 0.25);

        // Stubble is skin seen through cropped hair, so it has to darken the
        // complexion whatever the hair colour: a blond shade laid straight
        // onto pale skin reads as a light smear, not growth
        let beard_shadow = skin.shade(0.80).mix(hair, 0.45);

        Tones {
            skin,
            skin_hi,
            skin_spec,
            skin_warm,
            skin_dk,
            skin_dk2,
            skin_shadow,
            skin_cool,
            lip,
            lip_dk,
            lip_hi,
            mouth_line,
            hair,
            hair_hi,
            hair_dk,
            grey_hair,
            iris,
            iris_hi,
            iris_dk,
            iris_rim,
            sclera,
            sclera_dk,
            canthus,
            beard_shadow,
            luma,
        }
    }

    /// Hair with the grey mixed in — a man going grey is not one shade
    /// lighter, he is his own colour with white threaded through.
    pub fn hair_greyed(&self, grey: f32) -> Rgb {
        self.hair.mix(self.grey_hair, grey * 0.55)
    }
}
