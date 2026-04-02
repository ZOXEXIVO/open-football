/// Country-specific skill biases based on real-world football culture.
///
/// Applied as additive modifiers to the 1–20 skill scale after base generation.
/// Typical range: ±0.5 to ±2.0 per skill. These create recognizable national
/// playing styles without making all players from one country identical
/// (noise and role archetypes still dominate individual variation).

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

/// Returns per-skill additive bias for a player's country.
/// Country-specific overrides take precedence over continent defaults.
pub fn country_skill_bias(continent_id: u32, country_code: &str) -> [f32; SKILL_COUNT] {
    let mut b = [0.0f32; SKILL_COUNT];

    // Try country-specific bias first; fall back to continent/region
    if !apply_country_bias(&mut b, country_code) {
        apply_region_bias(&mut b, continent_id, country_code);
    }

    b
}

/// Country-specific biases for major football nations.
/// Returns true if a specific bias was applied.
fn apply_country_bias(b: &mut [f32; SKILL_COUNT], code: &str) -> bool {
    match code {
        // ── South America ──────────────────────────────────────────────
        "br" => {
            // Brazil: jogo bonito — flair, technique, creativity
            b[SK_DRIBBLING] = 2.0;  b[SK_TECHNIQUE] = 1.5;  b[SK_FIRST_TOUCH] = 1.0;
            b[SK_FINISHING] = 0.5;  b[SK_FREE_KICKS] = 0.5;
            b[SK_FLAIR] = 2.0;     b[SK_OFF_THE_BALL] = 0.5;
            b[SK_COMPOSURE] = 0.5;
            b[SK_AGILITY] = 0.5;   b[SK_BALANCE] = 0.5;    b[SK_ACCELERATION] = 0.5;
            // Tradeoffs
            b[SK_POSITIONING] = -0.5; b[SK_TEAMWORK] = -0.5;
            b[SK_CONCENTRATION] = -0.5;
        }
        "ar" => {
            // Argentina: technique + mental toughness + garra
            b[SK_TECHNIQUE] = 1.5;  b[SK_DRIBBLING] = 1.0;
            b[SK_PASSING] = 0.5;    b[SK_FINISHING] = 0.5;
            b[SK_COMPOSURE] = 1.0;  b[SK_DETERMINATION] = 0.5;
            b[SK_AGGRESSION] = 0.5; b[SK_FLAIR] = 0.5;
            b[SK_BALANCE] = 0.5;
        }
        "uy" => {
            // Uruguay: garra charrúa — fierce determination, defensive solidity
            b[SK_DETERMINATION] = 2.0; b[SK_AGGRESSION] = 1.0;
            b[SK_BRAVERY] = 0.5;      b[SK_COMPOSURE] = 0.5;
            b[SK_TACKLING] = 0.5;
            b[SK_STRENGTH] = 0.5;
        }
        "co" => {
            // Colombia: flair + technique, creative attackers
            b[SK_TECHNIQUE] = 1.0;  b[SK_DRIBBLING] = 0.5;  b[SK_PASSING] = 0.5;
            b[SK_FLAIR] = 1.0;      b[SK_COMPOSURE] = 0.5;
            b[SK_PACE] = 0.5;       b[SK_AGILITY] = 0.5;
        }
        "cl" => {
            // Chile: high-press intensity, work rate, aggression
            b[SK_WORK_RATE] = 1.5;  b[SK_AGGRESSION] = 1.0;
            b[SK_DETERMINATION] = 0.5;
            b[SK_STAMINA] = 0.5;    b[SK_PACE] = 0.5;
            b[SK_TECHNIQUE] = 0.5;
        }

        // ── Europe: major nations ──────────────────────────────────────
        "es" => {
            // Spain: positional play, passing mastery
            b[SK_PASSING] = 2.0;    b[SK_FIRST_TOUCH] = 1.5;  b[SK_TECHNIQUE] = 1.0;
            b[SK_VISION] = 1.0;     b[SK_COMPOSURE] = 1.0;
            b[SK_DECISIONS] = 0.5;  b[SK_TEAMWORK] = 0.5;
            // Tradeoffs
            b[SK_STRENGTH] = -0.5;  b[SK_PACE] = -0.5;
        }
        "de" => {
            // Germany: organised, disciplined, physical + tactical
            b[SK_TEAMWORK] = 1.5;   b[SK_POSITIONING] = 1.0;
            b[SK_WORK_RATE] = 1.0;  b[SK_CONCENTRATION] = 0.5;
            b[SK_DETERMINATION] = 0.5;
            b[SK_STAMINA] = 1.0;    b[SK_STRENGTH] = 0.5;
            b[SK_PASSING] = 0.5;
        }
        "it" => {
            // Italy: tactical intelligence, defensive mastery, composure
            b[SK_POSITIONING] = 1.5; b[SK_CONCENTRATION] = 1.0;
            b[SK_COMPOSURE] = 1.0;   b[SK_ANTICIPATION] = 0.5;
            b[SK_DECISIONS] = 0.5;
            b[SK_MARKING] = 0.5;     b[SK_TACKLING] = 0.5;
            b[SK_TECHNIQUE] = 0.5;
            // Tradeoff
            b[SK_PACE] = -0.5;
        }
        "fr" => {
            // France: athletic, versatile, physically dominant
            b[SK_PACE] = 1.5;       b[SK_STRENGTH] = 1.0;
            b[SK_ACCELERATION] = 1.0; b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_TECHNIQUE] = 0.5;   b[SK_DRIBBLING] = 0.5;
            b[SK_COMPOSURE] = 0.5;
        }
        "nl" => {
            // Netherlands: total football — technical, intelligent
            b[SK_PASSING] = 1.5;    b[SK_FIRST_TOUCH] = 1.0;
            b[SK_TECHNIQUE] = 1.0;
            b[SK_VISION] = 1.0;     b[SK_COMPOSURE] = 0.5;
            b[SK_DECISIONS] = 0.5;
            b[SK_STAMINA] = 0.5;
            // Tradeoff
            b[SK_STRENGTH] = -0.5;
        }
        "pt" => {
            // Portugal: technical flair, dribbling culture
            b[SK_DRIBBLING] = 1.5;  b[SK_TECHNIQUE] = 1.0;
            b[SK_FIRST_TOUCH] = 0.5; b[SK_CROSSING] = 0.5;
            b[SK_FLAIR] = 1.0;       b[SK_COMPOSURE] = 0.5;
            b[SK_AGILITY] = 0.5;     b[SK_BALANCE] = 0.5;
        }
        "gb" => {
            // England: direct, physical, determined
            b[SK_HEADING] = 0.5;    b[SK_CROSSING] = 0.5;
            b[SK_LONG_SHOTS] = 0.5;
            b[SK_DETERMINATION] = 1.0; b[SK_WORK_RATE] = 1.0;
            b[SK_BRAVERY] = 0.5;      b[SK_AGGRESSION] = 0.5;
            b[SK_PACE] = 0.5;         b[SK_STAMINA] = 0.5;
            b[SK_STRENGTH] = 0.5;
            // Tradeoffs
            b[SK_TECHNIQUE] = -0.5;    b[SK_FLAIR] = -0.5;
        }
        "sc" => {
            // Scotland: physical, direct, brave
            b[SK_HEADING] = 0.5;
            b[SK_DETERMINATION] = 1.5; b[SK_WORK_RATE] = 1.0;
            b[SK_BRAVERY] = 1.0;      b[SK_AGGRESSION] = 0.5;
            b[SK_STAMINA] = 0.5;      b[SK_STRENGTH] = 0.5;
            // Tradeoffs
            b[SK_TECHNIQUE] = -0.5;    b[SK_FLAIR] = -1.0;
        }
        "ie" => {
            // Northern Ireland / Ireland: determined, physical
            b[SK_DETERMINATION] = 1.0; b[SK_WORK_RATE] = 1.0;
            b[SK_BRAVERY] = 0.5;
            b[SK_STAMINA] = 0.5;      b[SK_STRENGTH] = 0.5;
            b[SK_FLAIR] = -0.5;
        }
        "be" => {
            // Belgium: modern blend — technical + physical
            b[SK_TECHNIQUE] = 0.5;  b[SK_PASSING] = 0.5;
            b[SK_FIRST_TOUCH] = 0.5;
            b[SK_PACE] = 0.5;      b[SK_STRENGTH] = 0.5;
            b[SK_COMPOSURE] = 0.5;
        }
        "hr" => {
            // Croatia: technical midfield tradition, tactical
            b[SK_PASSING] = 1.0;    b[SK_TECHNIQUE] = 1.0;
            b[SK_FIRST_TOUCH] = 0.5;
            b[SK_VISION] = 0.5;     b[SK_COMPOSURE] = 0.5;
            b[SK_DETERMINATION] = 0.5;
            b[SK_STAMINA] = 0.5;
        }
        "rs" => {
            // Serbia: technically solid, physical, determined
            b[SK_TECHNIQUE] = 0.5;  b[SK_PASSING] = 0.5;
            b[SK_HEADING] = 0.5;
            b[SK_DETERMINATION] = 1.0; b[SK_AGGRESSION] = 0.5;
            b[SK_STRENGTH] = 0.5;
        }
        "tr" => {
            // Turkey: passionate, physical, aggressive
            b[SK_AGGRESSION] = 1.0; b[SK_DETERMINATION] = 0.5;
            b[SK_BRAVERY] = 0.5;
            b[SK_STRENGTH] = 0.5;   b[SK_STAMINA] = 0.5;
            b[SK_TECHNIQUE] = 0.5;
        }

        // ── Africa: specific nations ───────────────────────────────────
        "ng" => {
            // Nigeria: explosive athletes, raw talent
            b[SK_PACE] = 2.0;      b[SK_ACCELERATION] = 1.5;
            b[SK_STRENGTH] = 1.5;  b[SK_NATURAL_FITNESS] = 1.0;
            b[SK_JUMPING] = 0.5;
            b[SK_DRIBBLING] = 0.5;
            // Tradeoffs
            b[SK_POSITIONING] = -0.5; b[SK_CONCENTRATION] = -0.5;
        }
        "gh" => {
            // Ghana: physical + technical blend, flair
            b[SK_PACE] = 1.5;       b[SK_ACCELERATION] = 1.0;
            b[SK_STRENGTH] = 1.0;   b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_DRIBBLING] = 1.0;  b[SK_FLAIR] = 0.5;
            b[SK_CONCENTRATION] = -0.5;
        }
        "cm" => {
            // Cameroon: physically imposing, determined
            b[SK_PACE] = 1.5;      b[SK_STRENGTH] = 1.5;
            b[SK_ACCELERATION] = 1.0; b[SK_JUMPING] = 1.0;
            b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_DETERMINATION] = 0.5; b[SK_AGGRESSION] = 0.5;
            b[SK_CONCENTRATION] = -0.5;
        }
        "ci" => {
            // Côte d'Ivoire: powerful, explosive
            b[SK_PACE] = 1.5;       b[SK_STRENGTH] = 1.5;
            b[SK_ACCELERATION] = 1.0; b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_DRIBBLING] = 0.5;    b[SK_TECHNIQUE] = 0.5;
            b[SK_POSITIONING] = -0.5;
        }
        "sn" => {
            // Senegal: athletic, technically improving
            b[SK_PACE] = 1.5;       b[SK_STRENGTH] = 1.0;
            b[SK_ACCELERATION] = 1.0; b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_DRIBBLING] = 0.5;    b[SK_DETERMINATION] = 0.5;
        }
        "ma" => {
            // Morocco: technical, tactical awareness (North African style)
            b[SK_TECHNIQUE] = 1.0;  b[SK_DRIBBLING] = 1.0;
            b[SK_PASSING] = 0.5;    b[SK_FIRST_TOUCH] = 0.5;
            b[SK_COMPOSURE] = 0.5;  b[SK_VISION] = 0.5;
            b[SK_AGILITY] = 0.5;
        }
        "eg" => {
            // Egypt: technical, creative
            b[SK_TECHNIQUE] = 1.0;  b[SK_DRIBBLING] = 0.5;
            b[SK_PASSING] = 0.5;    b[SK_FIRST_TOUCH] = 0.5;
            b[SK_COMPOSURE] = 0.5;  b[SK_FLAIR] = 0.5;
            b[SK_AGILITY] = 0.5;
        }
        "dz" | "tn" => {
            // Algeria, Tunisia: North African — technical, agile
            b[SK_TECHNIQUE] = 0.5;  b[SK_DRIBBLING] = 0.5;
            b[SK_PASSING] = 0.5;
            b[SK_COMPOSURE] = 0.5;
            b[SK_AGILITY] = 0.5;    b[SK_PACE] = 0.5;
        }

        // ── Asia ───────────────────────────────────────────────────────
        "jp" => {
            // Japan: technical excellence, discipline, agility
            b[SK_FIRST_TOUCH] = 1.5; b[SK_TECHNIQUE] = 1.0;  b[SK_PASSING] = 1.0;
            b[SK_TEAMWORK] = 1.5;    b[SK_WORK_RATE] = 1.0;
            b[SK_CONCENTRATION] = 0.5; b[SK_DECISIONS] = 0.5;
            b[SK_AGILITY] = 1.5;     b[SK_BALANCE] = 1.0;    b[SK_STAMINA] = 0.5;
            // Tradeoffs
            b[SK_STRENGTH] = -2.0;   b[SK_JUMPING] = -1.0;
            b[SK_HEADING] = -0.5;    b[SK_AGGRESSION] = -0.5;
        }
        "kr" => {
            // South Korea: disciplined, high-stamina, technical-physical mix
            b[SK_STAMINA] = 1.5;    b[SK_WORK_RATE] = 1.5;
            b[SK_DETERMINATION] = 1.0; b[SK_TEAMWORK] = 1.0;
            b[SK_CONCENTRATION] = 0.5;
            b[SK_PACE] = 0.5;       b[SK_AGILITY] = 0.5;
            b[SK_TECHNIQUE] = 0.5;
            // Tradeoff
            b[SK_STRENGTH] = -1.0;
        }
        "ir" => {
            // Iran: technically competent, physical
            b[SK_TECHNIQUE] = 0.5;  b[SK_PASSING] = 0.5;
            b[SK_DETERMINATION] = 0.5;
            b[SK_STRENGTH] = 0.5;   b[SK_HEADING] = 0.5;
        }
        "au" => {
            // Australia: physical, British-influenced style
            b[SK_STRENGTH] = 0.5;   b[SK_STAMINA] = 0.5;  b[SK_PACE] = 0.5;
            b[SK_WORK_RATE] = 0.5;  b[SK_DETERMINATION] = 0.5;
            b[SK_HEADING] = 0.5;
        }

        // ── North/Central America ──────────────────────────────────────
        "mx" => {
            // Mexico: technical, work rate, agile
            b[SK_TECHNIQUE] = 0.5;  b[SK_DRIBBLING] = 0.5;
            b[SK_PASSING] = 0.5;
            b[SK_WORK_RATE] = 1.0;  b[SK_TEAMWORK] = 0.5;
            b[SK_STAMINA] = 0.5;    b[SK_AGILITY] = 0.5;
        }
        "us" => {
            // USA: athletic, physical, improving technically
            b[SK_PACE] = 1.0;       b[SK_STAMINA] = 1.0;
            b[SK_STRENGTH] = 0.5;   b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_WORK_RATE] = 0.5;
            // Tradeoff
            b[SK_TECHNIQUE] = -0.5;
        }
        "jm" => {
            // Jamaica: explosive speed
            b[SK_PACE] = 2.0;       b[SK_ACCELERATION] = 1.5;
            b[SK_STRENGTH] = 0.5;   b[SK_AGILITY] = 0.5;
            b[SK_FLAIR] = 0.5;      b[SK_DRIBBLING] = 0.5;
        }

        _ => return false,
    }
    true
}

/// Region/continent-level defaults for countries without specific biases.
fn apply_region_bias(b: &mut [f32; SKILL_COUNT], continent_id: u32, code: &str) {
    match continent_id {
        // ── Africa (0) ─────────────────────────────────────────────────
        0 => {
            // Sub-region: North Africa
            if matches!(code, "ly" | "mr") {
                b[SK_TECHNIQUE] = 0.5; b[SK_DRIBBLING] = 0.5;
                b[SK_AGILITY] = 0.5;   b[SK_COMPOSURE] = 0.5;
                return;
            }
            // Sub-region: East Africa
            if matches!(code, "ke" | "et" | "tz" | "ug" | "rw" | "bi" | "er" | "dj" | "so") {
                b[SK_STAMINA] = 2.0;   b[SK_NATURAL_FITNESS] = 1.0;
                b[SK_PACE] = 1.0;
                b[SK_STRENGTH] = -1.0;
                return;
            }
            // Default Africa: West/Central/Southern — explosive physicality
            b[SK_PACE] = 1.5;       b[SK_ACCELERATION] = 1.5;
            b[SK_STRENGTH] = 1.0;   b[SK_NATURAL_FITNESS] = 0.5;
            b[SK_STAMINA] = 0.5;    b[SK_JUMPING] = 0.5;
            b[SK_DRIBBLING] = 0.5;
            // Tradeoffs
            b[SK_POSITIONING] = -0.5; b[SK_CONCENTRATION] = -0.5;
        }

        // ── Europe (1) ─────────────────────────────────────────────────
        1 => {
            // Sub-region: Scandinavia
            if matches!(code, "se" | "no" | "dk" | "fi" | "is" | "fo") {
                b[SK_STRENGTH] = 1.5;  b[SK_JUMPING] = 1.0;
                b[SK_HEADING] = 0.5;   b[SK_STAMINA] = 0.5;
                b[SK_TEAMWORK] = 1.0;  b[SK_WORK_RATE] = 0.5;
                b[SK_POSITIONING] = 0.5;
                // Tradeoffs
                b[SK_AGILITY] = -0.5;  b[SK_FLAIR] = -0.5;
                return;
            }
            // Sub-region: Balkans
            if matches!(code, "ba" | "me" | "mk" | "si" | "al") {
                b[SK_TECHNIQUE] = 0.5; b[SK_PASSING] = 0.5;
                b[SK_DETERMINATION] = 1.0; b[SK_AGGRESSION] = 0.5;
                b[SK_STRENGTH] = 0.5;  b[SK_STAMINA] = 0.5;
                return;
            }
            // Sub-region: Eastern Europe
            if matches!(code, "ru" | "ua" | "by" | "pl" | "cz" | "sk" | "hu" | "ro" | "bg" | "md" | "lt" | "lv" | "ee") {
                b[SK_STRENGTH] = 1.0;  b[SK_STAMINA] = 1.0;
                b[SK_DETERMINATION] = 0.5; b[SK_WORK_RATE] = 0.5;
                // Tradeoff
                b[SK_AGILITY] = -0.5;
                return;
            }
            // Sub-region: Greece / Cyprus
            if matches!(code, "gr" | "cy") {
                b[SK_TEAMWORK] = 0.5;  b[SK_POSITIONING] = 0.5;
                b[SK_CONCENTRATION] = 0.5;
                b[SK_STAMINA] = 0.5;
                b[SK_FLAIR] = -0.5;
                return;
            }
            // Default Europe: Swiss, Austrian, etc. — balanced, organised
            b[SK_TEAMWORK] = 0.5;  b[SK_POSITIONING] = 0.5;
            b[SK_STAMINA] = 0.5;
        }

        // ── North / Central America & Caribbean (2) ────────────────────
        2 => {
            // Sub-region: Caribbean
            if matches!(code, "tt" | "bb" | "gd" | "dm" | "lc" | "kn" | "vc"
                              | "ag" | "ht" | "bs" | "ky" | "bm" | "vg" | "vi"
                              | "tc" | "ms" | "ai" | "gp" | "mq" | "mf" | "pr") {
                b[SK_PACE] = 1.5;      b[SK_ACCELERATION] = 1.0;
                b[SK_STRENGTH] = 0.5;  b[SK_AGILITY] = 0.5;
                b[SK_FLAIR] = 0.5;
                return;
            }
            // Sub-region: Central America
            if matches!(code, "cr" | "hn" | "sv" | "gt" | "ni" | "pa" | "bz") {
                b[SK_TECHNIQUE] = 0.5; b[SK_AGILITY] = 0.5;
                b[SK_BALANCE] = 0.5;   b[SK_WORK_RATE] = 0.5;
                return;
            }
            // Default: Canada, Cuba, Dominican Republic — athletic
            b[SK_PACE] = 0.5;  b[SK_STAMINA] = 0.5;
        }

        // ── South America (3) ──────────────────────────────────────────
        3 => {
            // Default South America: technical, flair (Paraguay, Ecuador, Peru, etc.)
            b[SK_TECHNIQUE] = 0.5;  b[SK_DRIBBLING] = 0.5;
            b[SK_FLAIR] = 0.5;
            b[SK_BALANCE] = 0.5;
        }

        // ── Asia (4) ──────────────────────────────────────────────────
        4 => {
            // Sub-region: Gulf states
            if matches!(code, "sa" | "ae" | "qa" | "kw" | "bh" | "om") {
                b[SK_PACE] = 0.5;     b[SK_STRENGTH] = 0.5;
                b[SK_DETERMINATION] = 0.5;
                return;
            }
            // Sub-region: Central Asia
            if matches!(code, "uz" | "kg" | "tj" | "tm" | "kz") {
                b[SK_STRENGTH] = 0.5; b[SK_STAMINA] = 0.5;
                b[SK_DETERMINATION] = 0.5;
                return;
            }
            // Sub-region: Southeast Asia
            if matches!(code, "th" | "id" | "my" | "ph" | "sg" | "kh" | "mm"
                              | "la" | "bn") {
                b[SK_AGILITY] = 1.0;  b[SK_BALANCE] = 0.5; b[SK_PACE] = 0.5;
                b[SK_STRENGTH] = -1.5; b[SK_JUMPING] = -1.0;
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
            b[SK_STAMINA] = 0.5;  b[SK_WORK_RATE] = 0.5;
        }

        // ── Oceania (5) ────────────────────────────────────────────────
        5 => {
            // Sub-region: Pacific Islands
            if matches!(code, "fj" | "pg" | "sb" | "vu" | "ws" | "to" | "ki"
                              | "tv" | "ck" | "wf" | "fm" | "mp" | "th") {
                b[SK_STRENGTH] = 1.0; b[SK_PACE] = 0.5;
                b[SK_AGILITY] = -0.5;
                return;
            }
            // Default Oceania (NZ, Guam, etc.)
            b[SK_STRENGTH] = 0.5;  b[SK_STAMINA] = 0.5;
            b[SK_WORK_RATE] = 0.5;
        }

        _ => {}
    }
}
