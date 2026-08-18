//! What a footballer looks like, decided once from where he is from.
//!
//! A player is drawn twice by two renderers that share no code otherwise: as
//! an SVG portrait on his profile page (`web::face`) and as a figure on the
//! pitch in the replay viewer (`match_viewer`). Both need the same answer to
//! the same question — what colour is this man's skin, hair and eyes — and
//! for a long time only the portrait asked it properly. The viewer picked a
//! tone off a hash of the player id, so a squad of Nigerians took the field
//! as a scatter of five complexions and the same player was a different race
//! in his photograph and in the match.
//!
//! The decision here is nationality-driven and deterministic:
//!
//! 1. The country's database record carries the shares of its population in
//!    three ancestry buckets ([`SkinDist`]) — that is the mixing.
//! 2. The country's [`Region`] says what each of those buckets actually looks
//!    like *there*: a "metis" Briton and a "metis" Kazakh are not the same
//!    person.
//! 3. The pair resolves to a [`Phenotype`], a coherent appearance package.
//!    Every weight table hangs off the class, so features can never
//!    contradict each other — no blond monolid with an afro.
//! 4. Tone, hair and eye colour are drawn from that class's bands by
//!    [`Appearance`], seeded from the player id alone. Same player, same face,
//!    every render, on both sides of the WebAssembly boundary.

// ── Region ────────────────────────────────────────────────────

/// Geographic region of a country — combined with the DB skin buckets it
/// selects one of the phenotype classes below. The DB percentages do the
/// ancestry mixing; the region says what each bucket looks like there.
#[derive(Clone, Copy, PartialEq)]
pub enum Region {
    NorthEurope,
    BritIsles,
    WestEurope,
    EastEurope,
    SouthEurope,
    Mena,
    SubSaharan,
    HornAfrica,
    SouthAsia,
    EastAsia,
    SoutheastAsia,
    CentralAsia,
    LatinAmerica,
    Andes,
    Caribbean,
    NorthAmerica,
    Pacific,
}

impl Region {
    /// All 218 database country codes are mapped. `code` is the two-letter
    /// code as the country record carries it, in either case.
    pub fn for_code(code: &str) -> Region {
        use Region::*;
        match code.to_ascii_lowercase().as_str() {
            "se" | "no" | "dk" | "fi" | "is" | "fo" | "ee" | "lv" | "lt" => NorthEurope,
            "gb" | "ie" => BritIsles,
            "fr" | "be" | "nl" | "lu" | "de" | "at" | "ch" | "li" | "ad" | "mc" | "gi" => {
                WestEurope
            }
            "ru" | "ua" | "by" | "pl" | "cz" | "sk" | "hu" | "md" | "ro" | "bg" => EastEurope,
            "es" | "pt" | "it" | "gr" | "mt" | "cy" | "sm" | "hr" | "si" | "ba" | "rs" | "me"
            | "mk" | "al" => SouthEurope,
            "ma" | "dz" | "tn" | "ly" | "eg" | "tr" | "ir" | "iq" | "sy" | "jo" | "lb" | "il"
            | "ps" | "sa" | "kw" | "bh" | "qa" | "ae" | "om" | "ye" | "ge" | "am" | "az" | "af"
            | "mr" => Mena,
            "et" | "er" | "dj" | "so" | "sd" => HornAfrica,
            "in" | "pk" | "bd" | "lk" | "np" | "bt" | "mv" => SouthAsia,
            "cn" | "jp" | "kp" | "kr" | "tw" | "hk" | "mo" | "mn" => EastAsia,
            "th" | "vn" | "la" | "kh" | "mm" | "my" | "sg" | "id" | "ph" | "bn" | "tl" => {
                SoutheastAsia
            }
            "kz" | "kg" | "uz" | "tm" | "tj" => CentralAsia,
            "mx" | "hn" | "sv" | "ni" | "cr" | "pa" | "co" | "ve" | "br" | "ar" | "uy" | "cl" => {
                LatinAmerica
            }
            "ec" | "pe" | "bo" | "py" | "gt" => Andes,
            "jm" | "tt" | "bb" | "ht" | "cu" | "do" | "pr" | "bs" | "ag" | "ai" | "aw" | "bm"
            | "vg" | "ky" | "dm" | "gd" | "kn" | "lc" | "vc" | "ms" | "tc" | "vi" | "mf" | "gp"
            | "mq" | "gf" | "sr" | "gy" | "bz" => Caribbean,
            "au" | "nz" | "fj" | "pg" | "sb" | "vu" | "nc" | "ws" | "to" | "ck" | "as" | "gu"
            | "mp" | "fm" | "ki" | "tv" | "wf" => Pacific,
            // Sub-Saharan Africa and everything unlisted with an African
            // majority resolves through the DB buckets anyway
            "ng" | "gh" | "sn" | "ci" | "cm" | "cg" | "cd" | "ao" | "mz" | "zm" | "zw" | "mw"
            | "tz" | "ug" | "rw" | "bi" | "ke" | "bw" | "na" | "sz" | "ls" | "za" | "mg" | "ml"
            | "bf" | "ne" | "td" | "cf" | "ga" | "gq" | "gw" | "gm" | "sl" | "lr" | "tg" | "bj"
            | "st" | "cv" | "km" | "re" | "yt" | "sc" | "mu" => SubSaharan,
            // Unknown codes: the mixed-society mapping (white→European,
            // metis→Mestizo, black→West African) is the safest universal read
            _ => NorthAmerica,
        }
    }
}

// ── Phenotype ─────────────────────────────────────────────────

/// Coherent appearance package: every weight table below belongs to one
/// class so features never contradict (no blond monolid with an afro).
#[derive(Clone, Copy, PartialEq)]
pub enum Phenotype {
    NorthEuropean,
    WestEuropean,
    Slavic,
    Mediterranean,
    Mena,
    SouthAsian,
    EastAsian,
    SoutheastAsian,
    WestAfrican,
    EastAfrican,
    Mestizo,
    Andean,
    Oceanian,
}

/// One of the three ancestry shares a country record carries.
#[derive(Clone, Copy, PartialEq)]
pub enum SkinBucket {
    White,
    Black,
    Metis,
}

impl Phenotype {
    pub fn classify(region: Region, bucket: SkinBucket) -> Phenotype {
        use Phenotype::*;
        use Region::*;
        use SkinBucket as B;
        match (region, bucket) {
            (HornAfrica, _) => EastAfrican,
            (Pacific, B::White) => WestEuropean,
            (Pacific, _) => Oceanian,
            (_, B::Black) => WestAfrican,
            (NorthEurope, B::White) => NorthEuropean,
            (NorthEurope, B::Metis) => Phenotype::Mena,
            (BritIsles, B::White) => WestEuropean,
            (BritIsles, B::Metis) => SouthAsian,
            (WestEurope, B::White) => WestEuropean,
            (WestEurope, B::Metis) => Phenotype::Mena,
            (EastEurope, B::White) => Slavic,
            (EastEurope, B::Metis) => Phenotype::Mena,
            (SouthEurope, B::White) => Mediterranean,
            (SouthEurope, B::Metis) => Phenotype::Mena,
            (Region::Mena, B::White) => Mediterranean,
            (Region::Mena, B::Metis) => Phenotype::Mena,
            (SubSaharan, B::White) => WestEuropean,
            (SubSaharan, B::Metis) => Mestizo,
            (SouthAsia, _) => SouthAsian,
            (EastAsia, _) => EastAsian,
            (SoutheastAsia, _) => SoutheastAsian,
            (CentralAsia, B::White) => Slavic,
            (CentralAsia, B::Metis) => EastAsian,
            (LatinAmerica | Caribbean | Andes, B::White) => Mediterranean,
            (LatinAmerica | Caribbean, B::Metis) => Mestizo,
            (Andes, B::Metis) => Andean,
            (NorthAmerica, B::White) => WestEuropean,
            (NorthAmerica, B::Metis) => Mestizo,
        }
    }

    /// Inclusive band of [`Palette::SKIN`] indices
    pub fn skin_band(self) -> (usize, usize) {
        use Phenotype::*;
        match self {
            NorthEuropean => (0, 1),
            WestEuropean | Slavic => (0, 2),
            Mediterranean => (1, 4),
            Mena => (2, 5),
            SouthAsian => (4, 7),
            EastAsian => (1, 3),
            SoutheastAsian => (3, 5),
            Mestizo => (3, 6),
            Andean => (4, 6),
            WestAfrican => (8, 11),
            EastAfrican => (7, 10),
            Oceanian => (6, 9),
        }
    }

    /// [`Palette::HAIR`] index weights (repetition = weight)
    pub fn hair_tbl(self) -> &'static [usize] {
        use Phenotype::*;
        match self {
            NorthEuropean => &[3, 4, 5, 6, 7, 7, 8, 8, 8, 2],
            WestEuropean => &[0, 1, 2, 3, 3, 4, 5, 6, 7, 9],
            Slavic => &[1, 2, 3, 3, 4, 5, 6, 7, 8, 2],
            Mediterranean => &[0, 0, 1, 1, 2, 2, 3, 3, 4, 2],
            Mena | SouthAsian => &[0, 0, 0, 1, 1, 1, 2, 2, 0, 1],
            EastAsian | SoutheastAsian | Andean => &[0, 0, 0, 0, 1, 1, 0, 0, 1, 0],
            WestAfrican | EastAfrican => &[0, 0, 0, 1, 1, 2, 0, 0, 1, 0],
            Oceanian => &[0, 0, 1, 1, 2, 0, 0, 1, 0, 0],
            Mestizo => &[0, 0, 1, 1, 2, 2, 3, 0, 1, 0],
        }
    }

    /// [`Palette::EYES`] index weights
    pub fn eye_tbl(self) -> &'static [usize] {
        use Phenotype::*;
        match self {
            NorthEuropean => &[0, 3, 3, 6, 6, 7, 7, 5],
            WestEuropean => &[0, 1, 3, 4, 5, 6, 7, 2],
            Slavic => &[0, 1, 3, 3, 6, 6, 7, 5],
            Mediterranean => &[0, 0, 1, 1, 2, 2, 4, 3],
            Mestizo => &[0, 0, 1, 1, 2, 2, 4, 5],
            EastAsian => &[0, 0, 0, 0, 1, 1, 0, 1],
            _ => &[0, 0, 0, 1, 1, 1, 2, 2],
        }
    }

    /// nose_st weights
    pub fn nose_tbl(self) -> &'static [usize] {
        use Phenotype::*;
        match self {
            WestAfrican | Oceanian => &[1, 1, 4, 4, 2, 5],
            EastAfrican => &[0, 3, 3, 5, 2, 0],
            Mena => &[1, 4, 4, 2, 5, 3],
            SouthAsian => &[2, 4, 5, 1, 3, 0],
            EastAsian | SoutheastAsian | Andean => &[0, 2, 2, 5, 5, 3],
            Mestizo => &[2, 4, 5, 0, 1, 3],
            _ => &[0, 1, 2, 3, 4, 5],
        }
    }

    /// mouth_st weights — fuller lips for African/Oceanian ancestry
    pub fn mouth_tbl(self) -> &'static [usize] {
        use Phenotype::*;
        match self {
            WestAfrican | Oceanian => &[3, 3, 1, 0, 3],
            EastAfrican => &[3, 0, 1, 2, 3],
            EastAsian | SoutheastAsian => &[0, 2, 4, 2, 0],
            _ => &[0, 1, 2, 3, 4],
        }
    }

    /// brow_st weights — MENA/South Asia carry the densest brows
    pub fn brow_tbl(self) -> &'static [usize] {
        use Phenotype::*;
        match self {
            Mena | SouthAsian => &[4, 4, 2, 0, 3, 1],
            _ => &[0, 1, 2, 3, 4, 5],
        }
    }

    /// Beard chance multiplier (numerator, denominator)
    pub fn beard_mul(self) -> (u16, u16) {
        use Phenotype::*;
        match self {
            Mena => (3, 2),
            SouthAsian => (7, 5),
            EastAsian | SoutheastAsian => (1, 3),
            Andean => (1, 2),
            _ => (1, 1),
        }
    }

    /// Epicanthic eye family (monolid/thin dominate)
    pub fn epicanthic(self) -> bool {
        matches!(self, Phenotype::EastAsian | Phenotype::SoutheastAsian)
    }

    /// Afro-textured hair: afro/cornrows plausible, straight long hair rare
    pub fn afro_hair(self) -> bool {
        matches!(
            self,
            Phenotype::WestAfrican | Phenotype::EastAfrican | Phenotype::Oceanian
        )
    }
}

// ── Skin distribution ─────────────────────────────────────────

/// A country's ancestry shares, as percentages of its population, plus the
/// region that says what each share looks like there.
#[derive(Clone, Copy)]
pub struct SkinDist {
    pub white: u8,
    pub black: u8,
    /// Everything the first two do not claim. Never read — the bucket roll
    /// falls through to it — but carried so the record reads as the database
    /// wrote it.
    pub metis: u8,
    pub region: Region,
}

impl SkinDist {
    /// A population of one ancestry — for a fixture that wants a particular
    /// phenotype on the pitch rather than a real country's mix.
    pub const fn pure(bucket: SkinBucket, region: Region) -> SkinDist {
        match bucket {
            SkinBucket::White => SkinDist {
                white: 100,
                black: 0,
                metis: 0,
                region,
            },
            SkinBucket::Black => SkinDist {
                white: 0,
                black: 100,
                metis: 0,
                region,
            },
            SkinBucket::Metis => SkinDist {
                white: 0,
                black: 0,
                metis: 100,
                region,
            },
        }
    }
}

impl Default for SkinDist {
    fn default() -> Self {
        SkinDist {
            white: 50,
            black: 20,
            metis: 30,
            region: Region::NorthAmerica,
        }
    }
}

// ── Palettes ──────────────────────────────────────────────────

/// The colours both renderers draw from, as `#rrggbb`.
///
/// Written as hex rather than as triples because that is what an SVG
/// attribute takes, and the viewer parses each of these exactly once when it
/// builds its materials. The important thing is that there is ONE table:
/// indices are what crosses the wire between the server and the viewer, so a
/// palette that differed at either end would land every player on somebody
/// else's complexion.
pub struct Palette;

impl Palette {
    /// Lightest to darkest, and continuous — a phenotype's band is a run of
    /// neighbouring entries, so a class covers a range of real people rather
    /// than one swatch.
    pub const SKIN: [&'static str; 12] = [
        "#F5E0CB", "#EACFB0", "#DDBF98", "#CDA97A", "#C09368", "#A87D58", "#926845", "#7D5535",
        "#694530", "#503322", "#3D2518", "#2E1B11",
    ];

    /// Black through to blond, with ginger last — it is not on the ramp, so
    /// it has to sit outside it and be reached by weight alone.
    pub const HAIR: [&'static str; 10] = [
        "#0E0E0E", "#1C150C", "#2F1F11", "#4D3A2B", "#6A5038", "#7E644A", "#96795A", "#B0946C",
        "#C4A882", "#6B2010",
    ];

    pub const EYES: [&'static str; 8] = [
        "#33251A", "#4A3828", "#5C4E3A", "#384F62", "#3D5844", "#4E6356", "#686D72", "#3F5A72",
    ];
}

// ── RNG ───────────────────────────────────────────────────────

/// The identity stream: xorshift over a mixed player id.
///
/// Not seeded from a clock and never re-seeded — everything a player looks
/// like is a pure function of his id, so a face is the same in the portrait,
/// on the pitch, and after a reload.
pub struct AppearanceRng {
    state: u64,
}

impl AppearanceRng {
    pub fn new(player_id: u32) -> Self {
        let mut s = player_id as u64;
        s = s.wrapping_add(0x9E3779B97F4A7C15);
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
        s ^= s >> 31;
        if s == 0 {
            s = 1;
        }
        AppearanceRng { state: s }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    pub fn range(&mut self, max: usize) -> usize {
        (self.next() % max as u64) as usize
    }

    pub fn chance(&mut self, pct: u8) -> bool {
        (self.next() % 100) < pct as u64
    }

    pub fn frange(&mut self, min: f32, max: f32) -> f32 {
        let t = (self.next() % 10000) as f32 / 10000.0;
        min + t * (max - min)
    }
}

// ── The draw ──────────────────────────────────────────────────

/// One player's colouring: his class, and an index into each palette.
///
/// Indices rather than colours because they are what the match page sends to
/// the viewer — three bytes per player instead of three hex strings, and the
/// viewer keeps one shared material per entry rather than one per man.
pub struct Appearance {
    pub phenotype: Phenotype,
    /// Index into [`Palette::SKIN`].
    pub skin: usize,
    /// Index into [`Palette::HAIR`].
    pub hair: usize,
    /// Index into [`Palette::EYES`].
    pub eyes: usize,
}

impl Appearance {
    /// What this player looks like. `dist` is his country's record.
    pub fn of(player_id: u32, dist: SkinDist) -> Appearance {
        Appearance::draw(&mut AppearanceRng::new(player_id), dist)
    }

    /// The same, off a stream the caller is already holding.
    ///
    /// The portrait generator draws a hundred more things from the same
    /// stream — head shape, hair style, asymmetry — and this has to be the
    /// FIRST thing off it, or the two entry points here diverge and a player
    /// changes colour between his photograph and the pitch.
    pub fn draw(rng: &mut AppearanceRng, dist: SkinDist) -> Appearance {
        let roll = rng.range(100) as u8;
        let bucket = if roll < dist.white {
            SkinBucket::White
        } else if roll < dist.white.saturating_add(dist.black) {
            SkinBucket::Black
        } else {
            SkinBucket::Metis
        };
        let phenotype = Phenotype::classify(dist.region, bucket);

        let (lo, hi) = phenotype.skin_band();
        let skin = lo + rng.range(hi - lo + 1);

        // Rolled across the palette and folded into the class's weight table,
        // rather than rolled over the table itself. Every class then takes
        // the same number of bits out of the stream whatever its table looks
        // like, which is what lets a table be re-weighted without moving
        // every other feature of every player who has that class.
        let hair_tbl = phenotype.hair_tbl();
        let hair = hair_tbl[rng.range(Palette::HAIR.len()) % hair_tbl.len()];
        let eye_tbl = phenotype.eye_tbl();
        let eyes = eye_tbl[rng.range(Palette::EYES.len()) % eye_tbl.len()];

        Appearance {
            phenotype,
            skin,
            hair,
            eyes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSES: [Phenotype; 13] = [
        Phenotype::NorthEuropean,
        Phenotype::WestEuropean,
        Phenotype::Slavic,
        Phenotype::Mediterranean,
        Phenotype::Mena,
        Phenotype::SouthAsian,
        Phenotype::EastAsian,
        Phenotype::SoutheastAsian,
        Phenotype::WestAfrican,
        Phenotype::EastAfrican,
        Phenotype::Mestizo,
        Phenotype::Andean,
        Phenotype::Oceanian,
    ];

    /// Every table here is a list of palette indices, and both renderers use
    /// them to subscript a fixed-size array. A typo in one of them is a panic
    /// in the middle of a match rather than a wrong colour.
    #[test]
    fn every_table_indexes_a_colour_that_exists() {
        for class in CLASSES {
            let (lo, hi) = class.skin_band();
            assert!(lo <= hi, "empty skin band");
            assert!(hi < Palette::SKIN.len(), "skin band runs off the ramp");
            assert!(class.hair_tbl().iter().all(|i| *i < Palette::HAIR.len()));
            assert!(class.eye_tbl().iter().all(|i| *i < Palette::EYES.len()));
            // These two index the portrait generator's own variant sets,
            // whose sizes it rolls against: 6 noses, 5 mouths, 6 brows.
            assert!(class.nose_tbl().iter().all(|i| *i < 6));
            assert!(class.mouth_tbl().iter().all(|i| *i < 5));
            assert!(class.brow_tbl().iter().all(|i| *i < 6));
        }
    }

    /// The whole point of the crate: the portrait and the pitch ask in two
    /// different ways and have to get the same answer.
    #[test]
    fn both_entry_points_agree() {
        let dist = SkinDist {
            white: 30,
            black: 40,
            metis: 30,
            region: Region::WestEurope,
        };
        for id in 1..500u32 {
            let held = Appearance::draw(&mut AppearanceRng::new(id), dist);
            let fresh = Appearance::of(id, dist);
            assert_eq!(held.skin, fresh.skin);
            assert_eq!(held.hair, fresh.hair);
            assert_eq!(held.eyes, fresh.eyes);
        }
    }

    /// A country's percentages decide the class, so a squad from one place
    /// lands inside that place's band instead of scattered across the ramp.
    #[test]
    fn a_nation_lands_in_its_own_band() {
        let nigeria = SkinDist {
            white: 0,
            black: 100,
            metis: 0,
            region: Region::for_code("ng"),
        };
        let (lo, hi) = Phenotype::WestAfrican.skin_band();
        for id in 1..500u32 {
            let look = Appearance::of(id, nigeria);
            assert!(
                (lo..=hi).contains(&look.skin),
                "id {id} left the West African band"
            );
        }

        // And a nation that really is mixed still is.
        let brazil = SkinDist {
            white: 43,
            black: 10,
            metis: 47,
            region: Region::for_code("br"),
        };
        let tones: std::collections::HashSet<usize> = (1..500u32)
            .map(|id| Appearance::of(id, brazil).skin)
            .collect();
        assert!(tones.len() > 4, "Brazil came out monochrome");
    }

    #[test]
    fn a_country_code_reads_in_either_case() {
        assert!(Region::for_code("BR") == Region::LatinAmerica);
        assert!(Region::for_code("br") == Region::LatinAmerica);
        // Nothing in the table: the mixed-society mapping is the safe read.
        assert!(Region::for_code("zz") == Region::NorthAmerica);
        assert!(Region::for_code("") == Region::NorthAmerica);
    }
}
