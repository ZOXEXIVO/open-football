//! Country-specific skill biases based on real-world football culture.
//!
//! Applied as additive modifiers to the 1–20 skill scale after base generation.
//! Typical range: ±0.5 to ±2.0 per skill. These create recognizable national
//! playing styles without making all players from one country identical
//! (noise and role archetypes still dominate individual variation).

// Skill index constants (must match player.rs)
#[allow(dead_code)]
const SK_CORNERS: usize = 0;
const SK_CROSSING: usize = 1;
const SK_DRIBBLING: usize = 2;
const SK_FINISHING: usize = 3;
const SK_FIRST_TOUCH: usize = 4;
const SK_FREE_KICKS: usize = 5;
const SK_HEADING: usize = 6;
const SK_LONG_SHOTS: usize = 7;
#[allow(dead_code)]
const SK_LONG_THROWS: usize = 8;
const SK_MARKING: usize = 9;
const SK_PASSING: usize = 10;
#[allow(dead_code)]
const SK_PENALTY_TAKING: usize = 11;
const SK_TACKLING: usize = 12;
const SK_TECHNIQUE: usize = 13;

const SK_AGGRESSION: usize = 14;
const SK_ANTICIPATION: usize = 15;
const SK_BRAVERY: usize = 16;
const SK_COMPOSURE: usize = 17;
const SK_CONCENTRATION: usize = 18;
const SK_DECISIONS: usize = 19;
const SK_DETERMINATION: usize = 20;
const SK_FLAIR: usize = 21;
#[allow(dead_code)]
const SK_LEADERSHIP: usize = 22;
const SK_OFF_THE_BALL: usize = 23;
const SK_POSITIONING: usize = 24;
const SK_TEAMWORK: usize = 25;
const SK_VISION: usize = 26;
const SK_WORK_RATE: usize = 27;

const SK_ACCELERATION: usize = 28;
const SK_AGILITY: usize = 29;
const SK_BALANCE: usize = 30;
const SK_JUMPING: usize = 31;
const SK_NATURAL_FITNESS: usize = 32;
const SK_PACE: usize = 33;
const SK_STAMINA: usize = 34;
const SK_STRENGTH: usize = 35;

const SKILL_COUNT: usize = 37;

/// One country's additive skill deltas. Multiple codes share a row where the
/// data has always treated them as one football culture.
struct CountrySkillBiasRow {
    codes: &'static [&'static str],
    deltas: &'static [(usize, f32)],
}

const COUNTRY_SKILL_BIASES: &[CountrySkillBiasRow] = &[
    // ── South America ──────────────────────────────────────────────
    CountrySkillBiasRow {
        // Brazil: jogo bonito — flair, technique, creativity
        codes: &["br"],
        deltas: &[
            (SK_DRIBBLING, 2.0),
            (SK_TECHNIQUE, 1.5),
            (SK_FIRST_TOUCH, 1.0),
            (SK_FINISHING, 0.5),
            (SK_FREE_KICKS, 0.5),
            (SK_FLAIR, 2.0),
            (SK_OFF_THE_BALL, 0.5),
            (SK_COMPOSURE, 0.5),
            (SK_AGILITY, 0.5),
            (SK_BALANCE, 0.5),
            (SK_ACCELERATION, 0.5),
            // Tradeoffs
            (SK_POSITIONING, -0.5),
            (SK_TEAMWORK, -0.5),
            (SK_CONCENTRATION, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Argentina: technique + mental toughness + garra
        codes: &["ar"],
        deltas: &[
            (SK_TECHNIQUE, 1.5),
            (SK_DRIBBLING, 1.0),
            (SK_PASSING, 0.5),
            (SK_FINISHING, 0.5),
            (SK_COMPOSURE, 1.0),
            (SK_DETERMINATION, 0.5),
            (SK_AGGRESSION, 0.5),
            (SK_FLAIR, 0.5),
            (SK_BALANCE, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Uruguay: garra charrúa — fierce determination, defensive solidity
        codes: &["uy"],
        deltas: &[
            (SK_DETERMINATION, 2.0),
            (SK_AGGRESSION, 1.0),
            (SK_BRAVERY, 0.5),
            (SK_COMPOSURE, 0.5),
            (SK_TACKLING, 0.5),
            (SK_STRENGTH, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Colombia: flair + technique, creative attackers
        codes: &["co"],
        deltas: &[
            (SK_TECHNIQUE, 1.0),
            (SK_DRIBBLING, 0.5),
            (SK_PASSING, 0.5),
            (SK_FLAIR, 1.0),
            (SK_COMPOSURE, 0.5),
            (SK_PACE, 0.5),
            (SK_AGILITY, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Chile: high-press intensity, work rate, aggression
        codes: &["cl"],
        deltas: &[
            (SK_WORK_RATE, 1.5),
            (SK_AGGRESSION, 1.0),
            (SK_DETERMINATION, 0.5),
            (SK_STAMINA, 0.5),
            (SK_PACE, 0.5),
            (SK_TECHNIQUE, 0.5),
        ],
    },
    // ── Europe: major nations ──────────────────────────────────────
    CountrySkillBiasRow {
        // Spain: positional play, passing mastery
        codes: &["es"],
        deltas: &[
            (SK_PASSING, 2.0),
            (SK_FIRST_TOUCH, 1.5),
            (SK_TECHNIQUE, 1.0),
            (SK_VISION, 1.0),
            (SK_COMPOSURE, 1.0),
            (SK_DECISIONS, 0.5),
            (SK_TEAMWORK, 0.5),
            // Tradeoffs
            (SK_STRENGTH, -0.5),
            (SK_PACE, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Germany: organised, disciplined, physical + tactical
        codes: &["de"],
        deltas: &[
            (SK_TEAMWORK, 1.5),
            (SK_POSITIONING, 1.0),
            (SK_WORK_RATE, 1.0),
            (SK_CONCENTRATION, 0.5),
            (SK_DETERMINATION, 0.5),
            (SK_STAMINA, 1.0),
            (SK_STRENGTH, 0.5),
            (SK_PASSING, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Italy: tactical intelligence, defensive mastery, composure
        codes: &["it"],
        deltas: &[
            (SK_POSITIONING, 1.5),
            (SK_CONCENTRATION, 1.0),
            (SK_COMPOSURE, 1.0),
            (SK_ANTICIPATION, 0.5),
            (SK_DECISIONS, 0.5),
            (SK_MARKING, 0.5),
            (SK_TACKLING, 0.5),
            (SK_TECHNIQUE, 0.5),
            // Tradeoff
            (SK_PACE, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // France: athletic, versatile, physically dominant
        codes: &["fr"],
        deltas: &[
            (SK_PACE, 1.5),
            (SK_STRENGTH, 1.0),
            (SK_ACCELERATION, 1.0),
            (SK_NATURAL_FITNESS, 0.5),
            (SK_TECHNIQUE, 0.5),
            (SK_DRIBBLING, 0.5),
            (SK_COMPOSURE, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Netherlands: total football — technical, intelligent
        codes: &["nl"],
        deltas: &[
            (SK_PASSING, 1.5),
            (SK_FIRST_TOUCH, 1.0),
            (SK_TECHNIQUE, 1.0),
            (SK_VISION, 1.0),
            (SK_COMPOSURE, 0.5),
            (SK_DECISIONS, 0.5),
            (SK_STAMINA, 0.5),
            // Tradeoff
            (SK_STRENGTH, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Portugal: technical flair, dribbling culture
        codes: &["pt"],
        deltas: &[
            (SK_DRIBBLING, 1.5),
            (SK_TECHNIQUE, 1.0),
            (SK_FIRST_TOUCH, 0.5),
            (SK_CROSSING, 0.5),
            (SK_FLAIR, 1.0),
            (SK_COMPOSURE, 0.5),
            (SK_AGILITY, 0.5),
            (SK_BALANCE, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // England: direct, physical, determined
        codes: &["gb"],
        deltas: &[
            (SK_HEADING, 0.5),
            (SK_CROSSING, 0.5),
            (SK_LONG_SHOTS, 0.5),
            (SK_DETERMINATION, 1.0),
            (SK_WORK_RATE, 1.0),
            (SK_BRAVERY, 0.5),
            (SK_AGGRESSION, 0.5),
            (SK_PACE, 0.5),
            (SK_STAMINA, 0.5),
            (SK_STRENGTH, 0.5),
            // Tradeoffs
            (SK_TECHNIQUE, -0.5),
            (SK_FLAIR, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Scotland: physical, direct, brave
        codes: &["sc"],
        deltas: &[
            (SK_HEADING, 0.5),
            (SK_DETERMINATION, 1.5),
            (SK_WORK_RATE, 1.0),
            (SK_BRAVERY, 1.0),
            (SK_AGGRESSION, 0.5),
            (SK_STAMINA, 0.5),
            (SK_STRENGTH, 0.5),
            // Tradeoffs
            (SK_TECHNIQUE, -0.5),
            (SK_FLAIR, -1.0),
        ],
    },
    CountrySkillBiasRow {
        // Northern Ireland / Republic of Ireland: determined, physical
        codes: &["ie", "ei"],
        deltas: &[
            (SK_DETERMINATION, 1.0),
            (SK_WORK_RATE, 1.0),
            (SK_BRAVERY, 0.5),
            (SK_STAMINA, 0.5),
            (SK_STRENGTH, 0.5),
            (SK_FLAIR, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Belgium: modern blend — technical + physical
        codes: &["be"],
        deltas: &[
            (SK_TECHNIQUE, 0.5),
            (SK_PASSING, 0.5),
            (SK_FIRST_TOUCH, 0.5),
            (SK_PACE, 0.5),
            (SK_STRENGTH, 0.5),
            (SK_COMPOSURE, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Croatia: technical midfield tradition, tactical
        codes: &["hr"],
        deltas: &[
            (SK_PASSING, 1.0),
            (SK_TECHNIQUE, 1.0),
            (SK_FIRST_TOUCH, 0.5),
            (SK_VISION, 0.5),
            (SK_COMPOSURE, 0.5),
            (SK_DETERMINATION, 0.5),
            (SK_STAMINA, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Serbia: technically solid, physical, determined
        codes: &["rs"],
        deltas: &[
            (SK_TECHNIQUE, 0.5),
            (SK_PASSING, 0.5),
            (SK_HEADING, 0.5),
            (SK_DETERMINATION, 1.0),
            (SK_AGGRESSION, 0.5),
            (SK_STRENGTH, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Turkey: passionate, physical, aggressive
        codes: &["tr"],
        deltas: &[
            (SK_AGGRESSION, 1.0),
            (SK_DETERMINATION, 0.5),
            (SK_BRAVERY, 0.5),
            (SK_STRENGTH, 0.5),
            (SK_STAMINA, 0.5),
            (SK_TECHNIQUE, 0.5),
        ],
    },
    // ── Africa: specific nations ───────────────────────────────────
    CountrySkillBiasRow {
        // Nigeria: explosive athletes, raw talent
        codes: &["ng"],
        deltas: &[
            (SK_PACE, 2.0),
            (SK_ACCELERATION, 1.5),
            (SK_STRENGTH, 1.5),
            (SK_NATURAL_FITNESS, 1.0),
            (SK_JUMPING, 0.5),
            (SK_DRIBBLING, 0.5),
            // Tradeoffs
            (SK_POSITIONING, -0.5),
            (SK_CONCENTRATION, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Ghana: physical + technical blend, flair
        codes: &["gh"],
        deltas: &[
            (SK_PACE, 1.5),
            (SK_ACCELERATION, 1.0),
            (SK_STRENGTH, 1.0),
            (SK_NATURAL_FITNESS, 0.5),
            (SK_DRIBBLING, 1.0),
            (SK_FLAIR, 0.5),
            (SK_CONCENTRATION, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Cameroon: physically imposing, determined
        codes: &["cm"],
        deltas: &[
            (SK_PACE, 1.5),
            (SK_STRENGTH, 1.5),
            (SK_ACCELERATION, 1.0),
            (SK_JUMPING, 1.0),
            (SK_NATURAL_FITNESS, 0.5),
            (SK_DETERMINATION, 0.5),
            (SK_AGGRESSION, 0.5),
            (SK_CONCENTRATION, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Côte d'Ivoire: powerful, explosive
        codes: &["ci"],
        deltas: &[
            (SK_PACE, 1.5),
            (SK_STRENGTH, 1.5),
            (SK_ACCELERATION, 1.0),
            (SK_NATURAL_FITNESS, 0.5),
            (SK_DRIBBLING, 0.5),
            (SK_TECHNIQUE, 0.5),
            (SK_POSITIONING, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Senegal: athletic, technically improving
        codes: &["sn"],
        deltas: &[
            (SK_PACE, 1.5),
            (SK_STRENGTH, 1.0),
            (SK_ACCELERATION, 1.0),
            (SK_NATURAL_FITNESS, 0.5),
            (SK_DRIBBLING, 0.5),
            (SK_DETERMINATION, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Morocco: technical, tactical awareness (North African style)
        codes: &["ma"],
        deltas: &[
            (SK_TECHNIQUE, 1.0),
            (SK_DRIBBLING, 1.0),
            (SK_PASSING, 0.5),
            (SK_FIRST_TOUCH, 0.5),
            (SK_COMPOSURE, 0.5),
            (SK_VISION, 0.5),
            (SK_AGILITY, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Egypt: technical, creative
        codes: &["eg"],
        deltas: &[
            (SK_TECHNIQUE, 1.0),
            (SK_DRIBBLING, 0.5),
            (SK_PASSING, 0.5),
            (SK_FIRST_TOUCH, 0.5),
            (SK_COMPOSURE, 0.5),
            (SK_FLAIR, 0.5),
            (SK_AGILITY, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Algeria, Tunisia: North African — technical, agile
        codes: &["dz", "tn"],
        deltas: &[
            (SK_TECHNIQUE, 0.5),
            (SK_DRIBBLING, 0.5),
            (SK_PASSING, 0.5),
            (SK_COMPOSURE, 0.5),
            (SK_AGILITY, 0.5),
            (SK_PACE, 0.5),
        ],
    },
    // ── Asia ───────────────────────────────────────────────────────
    CountrySkillBiasRow {
        // Japan: technical excellence, discipline, agility
        codes: &["jp"],
        deltas: &[
            (SK_FIRST_TOUCH, 1.5),
            (SK_TECHNIQUE, 1.0),
            (SK_PASSING, 1.0),
            (SK_TEAMWORK, 1.5),
            (SK_WORK_RATE, 1.0),
            (SK_CONCENTRATION, 0.5),
            (SK_DECISIONS, 0.5),
            (SK_AGILITY, 1.5),
            (SK_BALANCE, 1.0),
            (SK_STAMINA, 0.5),
            // Tradeoffs
            (SK_STRENGTH, -2.0),
            (SK_JUMPING, -1.0),
            (SK_HEADING, -0.5),
            (SK_AGGRESSION, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // South Korea: disciplined, high-stamina, technical-physical mix
        codes: &["kr"],
        deltas: &[
            (SK_STAMINA, 1.5),
            (SK_WORK_RATE, 1.5),
            (SK_DETERMINATION, 1.0),
            (SK_TEAMWORK, 1.0),
            (SK_CONCENTRATION, 0.5),
            (SK_PACE, 0.5),
            (SK_AGILITY, 0.5),
            (SK_TECHNIQUE, 0.5),
            // Tradeoff
            (SK_STRENGTH, -1.0),
        ],
    },
    CountrySkillBiasRow {
        // Iran: technically competent, physical
        codes: &["ir"],
        deltas: &[
            (SK_TECHNIQUE, 0.5),
            (SK_PASSING, 0.5),
            (SK_DETERMINATION, 0.5),
            (SK_STRENGTH, 0.5),
            (SK_HEADING, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // Australia: physical, British-influenced style
        codes: &["au"],
        deltas: &[
            (SK_STRENGTH, 0.5),
            (SK_STAMINA, 0.5),
            (SK_PACE, 0.5),
            (SK_WORK_RATE, 0.5),
            (SK_DETERMINATION, 0.5),
            (SK_HEADING, 0.5),
        ],
    },
    // ── North/Central America ──────────────────────────────────────
    CountrySkillBiasRow {
        // Mexico: technical, work rate, agile
        codes: &["mx"],
        deltas: &[
            (SK_TECHNIQUE, 0.5),
            (SK_DRIBBLING, 0.5),
            (SK_PASSING, 0.5),
            (SK_WORK_RATE, 1.0),
            (SK_TEAMWORK, 0.5),
            (SK_STAMINA, 0.5),
            (SK_AGILITY, 0.5),
        ],
    },
    CountrySkillBiasRow {
        // USA: athletic, physical, improving technically
        codes: &["us"],
        deltas: &[
            (SK_PACE, 1.0),
            (SK_STAMINA, 1.0),
            (SK_STRENGTH, 0.5),
            (SK_NATURAL_FITNESS, 0.5),
            (SK_WORK_RATE, 0.5),
            // Tradeoff
            (SK_TECHNIQUE, -0.5),
        ],
    },
    CountrySkillBiasRow {
        // Jamaica: explosive speed
        codes: &["jm"],
        deltas: &[
            (SK_PACE, 2.0),
            (SK_ACCELERATION, 1.5),
            (SK_STRENGTH, 0.5),
            (SK_AGILITY, 0.5),
            (SK_FLAIR, 0.5),
            (SK_DRIBBLING, 0.5),
        ],
    },
];

/// Per-skill additive bias for a player's country, with country-specific
/// overrides taking precedence over continent defaults.
pub struct CountrySkillBias;

impl CountrySkillBias {
    pub fn for_country(continent_id: u32, country_code: &str) -> [f32; SKILL_COUNT] {
        let mut b = [0.0f32; SKILL_COUNT];

        // Try country-specific bias first; fall back to continent/region
        if !Self::apply_country(&mut b, country_code) {
            Self::apply_region(&mut b, continent_id, country_code);
        }

        b
    }

    /// Country-specific biases for major football nations.
    /// Returns true if a specific bias was applied.
    fn apply_country(b: &mut [f32; SKILL_COUNT], code: &str) -> bool {
        let Some(bias) = COUNTRY_SKILL_BIASES
            .iter()
            .find(|bias| bias.codes.contains(&code))
        else {
            return false;
        };
        for &(skill, delta) in bias.deltas {
            b[skill] = delta;
        }
        true
    }

    /// Region/continent-level defaults for countries without specific biases.
    fn apply_region(b: &mut [f32; SKILL_COUNT], continent_id: u32, code: &str) {
        match continent_id {
            0 => Self::apply_africa(b, code),
            1 => Self::apply_europe(b, code),
            2 => Self::apply_north_central_america(b, code),
            3 => Self::apply_south_america(b),
            4 => Self::apply_asia(b, code),
            5 => Self::apply_oceania(b, code),
            _ => {}
        }
    }

    fn apply_africa(b: &mut [f32; SKILL_COUNT], code: &str) {
        // Sub-region: North Africa
        if matches!(code, "ly" | "mr") {
            b[SK_TECHNIQUE] = 0.5;
            b[SK_DRIBBLING] = 0.5;
            b[SK_AGILITY] = 0.5;
            b[SK_COMPOSURE] = 0.5;
            return;
        }
        // Sub-region: East Africa
        if matches!(
            code,
            "ke" | "et" | "tz" | "ug" | "rw" | "bi" | "er" | "dj" | "so"
        ) {
            b[SK_STAMINA] = 2.0;
            b[SK_NATURAL_FITNESS] = 1.0;
            b[SK_PACE] = 1.0;
            b[SK_STRENGTH] = -1.0;
            return;
        }
        // Default Africa: West/Central/Southern — explosive physicality
        b[SK_PACE] = 1.5;
        b[SK_ACCELERATION] = 1.5;
        b[SK_STRENGTH] = 1.0;
        b[SK_NATURAL_FITNESS] = 0.5;
        b[SK_STAMINA] = 0.5;
        b[SK_JUMPING] = 0.5;
        b[SK_DRIBBLING] = 0.5;
        // Tradeoffs
        b[SK_POSITIONING] = -0.5;
        b[SK_CONCENTRATION] = -0.5;
    }

    fn apply_europe(b: &mut [f32; SKILL_COUNT], code: &str) {
        // Sub-region: Scandinavia
        if matches!(code, "se" | "no" | "dk" | "fi" | "is" | "fo") {
            b[SK_STRENGTH] = 1.5;
            b[SK_JUMPING] = 1.0;
            b[SK_HEADING] = 0.5;
            b[SK_STAMINA] = 0.5;
            b[SK_TEAMWORK] = 1.0;
            b[SK_WORK_RATE] = 0.5;
            b[SK_POSITIONING] = 0.5;
            // Tradeoffs
            b[SK_AGILITY] = -0.5;
            b[SK_FLAIR] = -0.5;
            return;
        }
        // Sub-region: Balkans
        if matches!(code, "ba" | "me" | "mk" | "si" | "al") {
            b[SK_TECHNIQUE] = 0.5;
            b[SK_PASSING] = 0.5;
            b[SK_DETERMINATION] = 1.0;
            b[SK_AGGRESSION] = 0.5;
            b[SK_STRENGTH] = 0.5;
            b[SK_STAMINA] = 0.5;
            return;
        }
        // Sub-region: Eastern Europe
        if matches!(
            code,
            "ru" | "ua"
                | "by"
                | "pl"
                | "cz"
                | "sk"
                | "hu"
                | "ro"
                | "bg"
                | "md"
                | "lt"
                | "lv"
                | "ee"
        ) {
            b[SK_STRENGTH] = 1.0;
            b[SK_STAMINA] = 1.0;
            b[SK_DETERMINATION] = 0.5;
            b[SK_WORK_RATE] = 0.5;
            // Tradeoff
            b[SK_AGILITY] = -0.5;
            return;
        }
        // Sub-region: Greece / Cyprus
        if matches!(code, "gr" | "cy") {
            b[SK_TEAMWORK] = 0.5;
            b[SK_POSITIONING] = 0.5;
            b[SK_CONCENTRATION] = 0.5;
            b[SK_STAMINA] = 0.5;
            b[SK_FLAIR] = -0.5;
            return;
        }
        // Default Europe: Swiss, Austrian, etc. — balanced, organised
        b[SK_TEAMWORK] = 0.5;
        b[SK_POSITIONING] = 0.5;
        b[SK_STAMINA] = 0.5;
    }

    fn apply_north_central_america(b: &mut [f32; SKILL_COUNT], code: &str) {
        // Sub-region: Caribbean
        if matches!(
            code,
            "tt" | "bb"
                | "gd"
                | "dm"
                | "lc"
                | "kn"
                | "vc"
                | "ag"
                | "ht"
                | "bs"
                | "ky"
                | "bm"
                | "vg"
                | "vi"
                | "tc"
                | "ms"
                | "ai"
                | "gp"
                | "mq"
                | "mf"
                | "pr"
        ) {
            b[SK_PACE] = 1.5;
            b[SK_ACCELERATION] = 1.0;
            b[SK_STRENGTH] = 0.5;
            b[SK_AGILITY] = 0.5;
            b[SK_FLAIR] = 0.5;
            return;
        }
        // Sub-region: Central America
        if matches!(code, "cr" | "hn" | "sv" | "gt" | "ni" | "pa" | "bz") {
            b[SK_TECHNIQUE] = 0.5;
            b[SK_AGILITY] = 0.5;
            b[SK_BALANCE] = 0.5;
            b[SK_WORK_RATE] = 0.5;
            return;
        }
        // Default: Canada, Cuba, Dominican Republic — athletic
        b[SK_PACE] = 0.5;
        b[SK_STAMINA] = 0.5;
    }

    fn apply_south_america(b: &mut [f32; SKILL_COUNT]) {
        // Default South America: technical, flair (Paraguay, Ecuador, Peru, etc.)
        b[SK_TECHNIQUE] = 0.5;
        b[SK_DRIBBLING] = 0.5;
        b[SK_FLAIR] = 0.5;
        b[SK_BALANCE] = 0.5;
    }

    fn apply_asia(b: &mut [f32; SKILL_COUNT], code: &str) {
        // Sub-region: Gulf states
        if matches!(code, "sa" | "ae" | "qa" | "kw" | "bh" | "om") {
            b[SK_PACE] = 0.5;
            b[SK_STRENGTH] = 0.5;
            b[SK_DETERMINATION] = 0.5;
            return;
        }
        // Sub-region: Central Asia
        if matches!(code, "uz" | "kg" | "tj" | "tm" | "kz") {
            b[SK_STRENGTH] = 0.5;
            b[SK_STAMINA] = 0.5;
            b[SK_DETERMINATION] = 0.5;
            return;
        }
        // Sub-region: Southeast Asia
        if matches!(
            code,
            "th" | "id" | "my" | "ph" | "sg" | "kh" | "mm" | "la" | "bn"
        ) {
            b[SK_AGILITY] = 1.0;
            b[SK_BALANCE] = 0.5;
            b[SK_PACE] = 0.5;
            b[SK_STRENGTH] = -1.5;
            b[SK_JUMPING] = -1.0;
            b[SK_HEADING] = -0.5;
            return;
        }
        // Sub-region: South Asia
        if matches!(code, "in" | "pk" | "bd" | "lk" | "np") {
            b[SK_STAMINA] = 0.5;
            b[SK_TECHNIQUE] = -0.5;
            return;
        }
        // Default Asia: China, Mongolia, etc. — developing
        b[SK_STAMINA] = 0.5;
        b[SK_WORK_RATE] = 0.5;
    }

    fn apply_oceania(b: &mut [f32; SKILL_COUNT], code: &str) {
        // Sub-region: Pacific Islands
        if matches!(
            code,
            "fj" | "pg"
                | "sb"
                | "vu"
                | "ws"
                | "to"
                | "ki"
                | "tv"
                | "ck"
                | "wf"
                | "fm"
                | "mp"
                | "th"
        ) {
            b[SK_STRENGTH] = 1.0;
            b[SK_PACE] = 0.5;
            b[SK_AGILITY] = -0.5;
            return;
        }
        // Default Oceania (NZ, Guam, etc.)
        b[SK_STRENGTH] = 0.5;
        b[SK_STAMINA] = 0.5;
        b[SK_WORK_RATE] = 0.5;
    }
}
