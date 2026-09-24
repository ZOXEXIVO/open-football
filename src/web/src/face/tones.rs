//! The pigments one player is made of, derived from his three palette
//! colours.
//!
//! These are albedos — what the surface does to light — not the colours it
//! ends up on screen: the studio decides those. Every variation here is a
//! departure from the base complexion in the direction real skin departs,
//! so a change of complexion carries the whole set with it: blood shows
//! redder where the skin is thin, the shaven lower face is bluer, lips are
//! the skin with more blood and less melanin at the surface.

use super::color::{Linear, Rgb};

pub struct Tones {
    pub skin: Linear,
    /// Fully flushed skin — the cheeks, nose and ears lean toward it
    pub flush: Linear,
    /// The forehead and bridge: thicker skin, a touch lighter and yellower
    pub pale: Linear,
    /// Skin over the roots of a shaven beard
    pub shaved: Linear,
    /// The thin skin round the eye, darker and cooler
    pub orbit: Linear,
    pub lip: Linear,
    pub hair: Linear,
    pub grey_hair: Linear,
    pub iris: Linear,
    pub sclera: Linear,
    /// The pink caruncle in the inner corner
    pub caruncle: Linear,
    pub lash: Linear,
    /// Freckles and moles: concentrated melanin
    pub mark: Linear,
    /// How light the complexion is, 0..1 on the encoded scale — the
    /// quantity every "shows more on fair skin" rule is written against
    pub fairness: f32,
}

impl Tones {
    pub fn derive(skin_hex: &str, hair_hex: &str, eye_hex: &str, redness: f32) -> Tones {
        let encoded = Rgb::hex(skin_hex);
        let fairness = (0.2126 * encoded.r + 0.7152 * encoded.g + 0.0722 * encoded.b) / 255.0;
        // The palette is how a complexion LOOKS in a lit photograph; the
        // pigment under it is darker, so the lit plane lands on the palette
        // tone and the planes turning away fall below it
        let skin = encoded.linear() * 0.97;
        let hair = Linear::hex(hair_hex);

        let blood = 0.65 + 0.45 * fairness + redness * 0.25;
        let flush = skin.mix(skin * Linear::new(1.32, 0.70, 0.68), blood);
        let pale = skin * Linear::new(1.06, 1.04, 0.92);
        // Follicles under the skin read as a blue-grey veil — strongest on
        // fair skin under dark hair, all but gone on dark skin or a blond
        let darkness = 1.0 - (hair.luma() / 0.3).min(1.0);
        let veil = (0.2 + 0.8 * fairness) * (0.35 + 0.65 * darkness);
        let shaved = skin * Linear::gray(1.0).mix(Linear::new(0.74, 0.76, 0.81), veil);
        let orbit = skin * Linear::new(0.82, 0.78, 0.84);

        // Lips on a dark complexion are close to the skin and browner; on a
        // fair one they are much darker and redder than it
        let depth = 0.70 + 0.22 * (1.0 - fairness);
        let lip = skin * Linear::new(1.06, 0.68, 0.70) * depth;

        let sclera = Linear::new(0.58, 0.54, 0.52).mix(skin, 0.10 + 0.08 * fairness);
        let caruncle = Linear::new(0.55, 0.22, 0.21).mix(skin, 0.35);
        let lash = hair.mix(Linear::new(0.012, 0.009, 0.008), 0.72);
        let mark = (skin * Linear::new(0.75, 0.55, 0.42)).deepen(1.18);

        Tones {
            skin,
            flush,
            pale,
            shaved,
            orbit,
            lip,
            hair,
            grey_hair: Linear::hex("#B9B5AE"),
            iris: Linear::hex(eye_hex),
            sclera,
            caruncle,
            lash,
            mark,
            fairness,
        }
    }

    /// Hair with the grey mixed in — a man going grey is not one shade
    /// lighter, he is his own colour with white threaded through.
    pub fn hair_greyed(&self, grey: f32) -> Linear {
        self.hair.mix(self.grey_hair, grey * 0.55)
    }
}
