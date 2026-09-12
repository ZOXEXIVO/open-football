//! Everything about a player that is decided by dice — drawn once, in a
//! fixed order, off the id-seeded stream.
//!
//! [`Appearance::draw`] MUST be the first call on the rng: the match viewer
//! makes the same call on a fresh stream for the same player and the two
//! have to agree on his complexion. Everything after it is the portrait's
//! own business and may be reordered freely.

use shared::{Appearance, AppearanceRng, Phenotype, SkinDist};

/// Scalp hair, as the styles a barber would name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HairStyle {
    Crop,
    SidePart,
    Medium,
    Buzz,
    SweptBack,
    Afro,
    Bald,
    Curly,
    Long,
    Fade,
    FauxHawk,
    Cornrows,
}

impl HairStyle {
    /// The old numeric code, kept for the debug comment in the SVG so a
    /// contact sheet can still be read off.
    pub fn code(self) -> u8 {
        match self {
            HairStyle::Crop => 0,
            HairStyle::SidePart => 1,
            HairStyle::Medium => 2,
            HairStyle::Buzz => 3,
            HairStyle::SweptBack => 4,
            HairStyle::Afro => 5,
            HairStyle::Bald => 6,
            HairStyle::Curly => 7,
            HairStyle::Long => 8,
            HairStyle::Fade => 9,
            HairStyle::FauxHawk => 10,
            HairStyle::Cornrows => 11,
        }
    }
}

/// The front edge of the hair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hairline {
    Straight,
    Rounded,
    WidowsPeak,
    /// Temples cut back — the M of a man in his thirties
    Receding,
}

/// Lower-face hair, when grown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Beard {
    Stubble,
    Boxed,
    Full,
    Goatee,
    Chinstrap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Moustache {
    Thin,
    Chevron,
    Handlebar,
    Walrus,
}

/// Continuous departures from the archetype, all roughly in −1.5..1.5.
#[derive(Clone, Copy, Debug, Default)]
pub struct Morph {
    /// Skull breadth
    pub width: f32,
    /// Jaw breadth relative to the skull
    pub jaw: f32,
    /// Chin breadth
    pub chin_w: f32,
    /// Face length
    pub length: f32,
    /// Cheekbone prominence
    pub cheek: f32,
    /// Chin rounding
    pub round: f32,
    /// Forehead height (hairline up or down)
    pub forehead: f32,
    /// Nose length
    pub nose_len: f32,
    /// Nose breadth
    pub nose_w: f32,
    /// Mouth breadth
    pub mouth_w: f32,
    /// Lip fullness
    pub lip: f32,
    /// Ear size
    pub ear: f32,
    /// Brow thickness
    pub brow_thick: f32,
    /// Inter-ocular distance
    pub eye_spacing: f32,
    /// Outer canthus lift
    pub eye_tilt: f32,
    /// Hooded lids + deep sockets, 0..1
    pub lid_heavy: f32,
    /// Overall eye size, 0.9..1.1
    pub eye_scale: f32,
    /// Brow-to-eye distance
    pub brow_gap: f32,
    /// Skin flush (cheeks, nose), 0..1
    pub redness: f32,
    /// Freckle density, 0..1 (mostly 0)
    pub freckles: f32,
    /// Jaw angle sharpness
    pub jaw_angle: f32,
    /// Brow ridge prominence
    pub brow_ridge: f32,
}

pub struct Identity {
    pub look: Appearance,
    pub phenotype: Phenotype,
    /// 0 oval, 1 square, 2 round, 3 heart, 4 oblong, 5 diamond
    pub face_var: usize,
    pub hair: HairStyle,
    pub hairline: Hairline,
    /// −1 parts on the left, +1 on the right
    pub part_side: f32,
    pub brow_st: usize,
    pub eye_st: usize,
    pub nose_st: usize,
    pub mouth_st: usize,
    /// Seed for the strand/texture jitter that never touches the rng
    pub seed: usize,
    pub marks: usize,
    pub beard: Option<Beard>,
    pub moustache: Option<Moustache>,
    /// Asymmetry offsets (x, y)
    pub asym: (f32, f32),
    /// Age-driven face width
    pub fw: f32,
    pub morph: Morph,
    /// Photographic head tilt in degrees
    pub tilt: f32,
    /// Slight head turn, −1..1: the far side of the face narrows
    pub turn: f32,
    /// Grey hair share 0..1
    pub grey: f32,
}

impl Identity {
    pub fn draw(rng: &mut AppearanceRng, dist: SkinDist, age: u8) -> Identity {
        let look = Appearance::draw(rng, dist);
        let ph = look.phenotype;

        let face_var = rng.range(6);

        // Weighted style roll — everyday cuts dominate; statement styles
        // (mohawk, afro, long hair) are rare accents like on a real pitch
        let hair = match rng.range(48) {
            0..=7 => HairStyle::Crop,
            8..=13 => HairStyle::SidePart,
            14..=19 => HairStyle::Medium,
            20..=25 => HairStyle::Buzz,
            26..=30 => HairStyle::SweptBack,
            31..=35 => HairStyle::Fade,
            36..=38 => HairStyle::Curly,
            39..=40 => HairStyle::Afro,
            41..=42 => HairStyle::Long,
            43..=44 => HairStyle::Cornrows,
            45..=46 => HairStyle::Bald,
            _ => HairStyle::FauxHawk,
        };
        // A bald 17-year-old is not a thing — young players keep hair
        let hair = if age <= 23 && hair == HairStyle::Bald {
            HairStyle::Crop
        } else {
            hair
        };
        // Hair texture follows the class: afro/cornrows need tight curls;
        // conversely straight-hair styles don't hold on afro-textured hair
        let hair = if ph.afro_hair() {
            match hair {
                HairStyle::SweptBack => HairStyle::Fade,
                HairStyle::Long => HairStyle::Curly,
                HairStyle::SidePart => HairStyle::Crop,
                _ => hair,
            }
        } else {
            match hair {
                HairStyle::Afro => HairStyle::Medium,
                HairStyle::Cornrows => HairStyle::Crop,
                _ => hair,
            }
        };

        let b_tbl = ph.brow_tbl();
        let brow_st = b_tbl[rng.range(6) % b_tbl.len()];
        // Eye-shape roll by class family. Epicanthic classes draw monolid/thin
        // forms; the Andean family gets a milder fold; elsewhere open/large
        // forms are the majority so narrow forms stay distinct accents
        let eye_st = if ph.epicanthic() {
            match rng.range(12) {
                0..=5 => 3,
                6..=8 => 5,
                9..=10 => 1,
                _ => 0,
            }
        } else if ph == Phenotype::Andean {
            match rng.range(12) {
                0..=2 => 3,
                3..=4 => 1,
                5..=6 => 5,
                7..=9 => 0,
                10 => 4,
                _ => 6,
            }
        } else {
            match rng.range(12) {
                0..=2 => 0,
                3..=4 => 2,
                5..=6 => 7,
                7 => 1,
                8 => 3,
                9 => 4,
                10 => 5,
                _ => 6,
            }
        };
        let n_tbl = ph.nose_tbl();
        let nose_st = n_tbl[rng.range(6) % n_tbl.len()];
        let m_tbl = ph.mouth_tbl();
        let mouth_st = m_tbl[rng.range(5) % m_tbl.len()];
        let seed = rng.range(9999);
        let marks = rng.range(5);

        // Facial hair by age, scaled by class density: MENA/South Asia carry
        // the heaviest growth, East Asia the sparsest
        let (bc, mc): (u8, u8) = match age {
            0..=19 => (0, 0),
            20..=24 => (18, 10),
            25..=29 => (40, 30),
            30..=34 => (55, 42),
            _ => (65, 50),
        };
        let (bn, bd) = ph.beard_mul();
        let bc = ((bc as u16 * bn / bd).min(85)) as u8;
        let mc = ((mc as u16 * bn / bd).min(70)) as u8;
        let grown = bc > 0 && rng.chance(bc);
        let lip_hair = mc > 0 && rng.chance(mc);
        let beard_v = rng.range(5);
        let mst_v = rng.range(4);
        let beard = grown.then_some(match beard_v {
            0 => Beard::Stubble,
            1 => Beard::Boxed,
            2 => Beard::Full,
            3 => Beard::Goatee,
            _ => Beard::Chinstrap,
        });
        let moustache = lip_hair.then_some(match mst_v {
            0 => Moustache::Thin,
            1 => Moustache::Chevron,
            2 => Moustache::Handlebar,
            _ => Moustache::Walrus,
        });

        let asym = (rng.frange(-0.8, 0.8), rng.frange(-0.5, 0.5));

        // Face width by age — soft tissue fills out through the twenties
        let fw: f32 = match age {
            0..=19 => rng.frange(-1.1, 0.1),
            20..=24 => rng.frange(-0.5, 0.8),
            25..=29 => rng.frange(0.0, 1.6),
            30..=34 => rng.frange(0.8, 2.4),
            _ => rng.frange(1.3, 3.0),
        };

        let morph = Morph {
            width: rng.frange(-1.6, 1.6),
            jaw: rng.frange(-1.8, 1.8),
            chin_w: rng.frange(-1.4, 1.6),
            length: rng.frange(-1.5, 1.8),
            cheek: rng.frange(-1.0, 1.2),
            round: rng.frange(-1.0, 1.4),
            forehead: rng.frange(-1.2, 1.2),
            nose_len: rng.frange(-1.2, 1.2),
            nose_w: rng.frange(-1.0, 1.0),
            mouth_w: rng.frange(-1.2, 1.2),
            lip: rng.frange(-1.0, 1.0),
            ear: rng.frange(-1.0, 1.0),
            brow_thick: rng.frange(-1.0, 1.2),
            eye_spacing: rng.frange(-1.3, 1.6),
            eye_tilt: rng.frange(-1.2, 1.2),
            lid_heavy: rng.frange(0.0, 1.0),
            eye_scale: rng.frange(0.90, 1.10),
            brow_gap: rng.frange(-0.6, 1.4),
            redness: rng.frange(0.0, 1.0),
            freckles: {
                let f = rng.frange(0.0, 1.0);
                if f > 0.8 { (f - 0.8) * 5.0 } else { 0.0 }
            },
            jaw_angle: rng.frange(-1.0, 1.0),
            brow_ridge: rng.frange(-1.0, 1.0),
        };

        let tilt = rng.frange(-2.2, 2.2);
        let turn = rng.frange(-1.0, 1.0);
        let hairline = match rng.range(8) {
            0..=2 => Hairline::Rounded,
            3..=4 => Hairline::Straight,
            5 => Hairline::WidowsPeak,
            _ => Hairline::Receding,
        };
        // Recession is a thing that happens to a man, not a thing he is born
        // with; under 26 the M reads as a rounded line
        let hairline = if age < 26 && hairline == Hairline::Receding {
            Hairline::Rounded
        } else {
            hairline
        };
        let part_side = if rng.chance(50) { -1.0 } else { 1.0 };
        let grey_roll = rng.frange(0.0, 1.0);
        let grey = match age {
            0..=29 => 0.0,
            30..=33 => grey_roll * 0.18,
            34..=36 => 0.08 + grey_roll * 0.30,
            _ => 0.18 + grey_roll * 0.45,
        };

        Identity {
            look,
            phenotype: ph,
            face_var,
            hair,
            hairline,
            part_side,
            brow_st,
            eye_st,
            nose_st,
            mouth_st,
            seed,
            marks,
            beard,
            moustache,
            asym,
            fw,
            morph,
            tilt,
            turn,
            grey,
        }
    }

    /// Deterministic per-strand jitter in 0..1 that never consumes the RNG
    /// stream, so adding a strand somewhere never moves a feature elsewhere.
    pub fn jitter(&self, i: usize, k: usize) -> f32 {
        let mut h = (self.seed + i * 7919 + k * 104_729) as u64;
        h ^= h >> 17;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 29;
        (h % 10_000) as f32 / 10_000.0
    }

    /// The same jitter centred on zero: −1..1.
    pub fn jitter_signed(&self, i: usize, k: usize) -> f32 {
        self.jitter(i, k) * 2.0 - 1.0
    }
}
