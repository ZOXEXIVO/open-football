use chrono::{Datelike, NaiveDate, Utc};
use core::shared::FullName;
use core::utils::{FloatUtils, IntegerUtils};
use core::{
    Mental, PeopleNameGeneratorData, PersonAttributes, Physical, Player,
    PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, Technical,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

static PLAYER_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(1));

// ── Skill index constants (flat array order) ────────────────────────────
// Technical (0..14)
const SK_CORNERS: usize = 0;
const SK_CROSSING: usize = 1;
const SK_DRIBBLING: usize = 2;
const SK_FINISHING: usize = 3;
const SK_FIRST_TOUCH: usize = 4;
const SK_FREE_KICKS: usize = 5;
const SK_HEADING: usize = 6;
const SK_LONG_SHOTS: usize = 7;
const SK_LONG_THROWS: usize = 8;
const SK_MARKING: usize = 9;
const SK_PASSING: usize = 10;
const SK_PENALTY_TAKING: usize = 11;
const SK_TACKLING: usize = 12;
const SK_TECHNIQUE: usize = 13;
// Mental (14..28)
const SK_AGGRESSION: usize = 14;
const SK_ANTICIPATION: usize = 15;
const SK_BRAVERY: usize = 16;
const SK_COMPOSURE: usize = 17;
const SK_CONCENTRATION: usize = 18;
const SK_DECISIONS: usize = 19;
const SK_DETERMINATION: usize = 20;
const SK_FLAIR: usize = 21;
const SK_LEADERSHIP: usize = 22;
const SK_OFF_THE_BALL: usize = 23;
const SK_POSITIONING: usize = 24;
const SK_TEAMWORK: usize = 25;
const SK_VISION: usize = 26;
const SK_WORK_RATE: usize = 27;
// Physical (28..37)
const SK_ACCELERATION: usize = 28;
const SK_AGILITY: usize = 29;
const SK_BALANCE: usize = 30;
const SK_JUMPING: usize = 31;
const SK_NATURAL_FITNESS: usize = 32;
const SK_PACE: usize = 33;
const SK_STAMINA: usize = 34;
const SK_STRENGTH: usize = 35;
const SK_MATCH_READINESS: usize = 36;

const SKILL_COUNT: usize = 37;

// ── Helper functions ────────────────────────────────────────────────────

/// Box-Muller normal distribution (mean=0, std=1), no extra dependencies.
fn random_normal() -> f32 {
    let u1 = rand::random::<f32>().max(1e-10);
    let u2 = rand::random::<f32>();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ── Age curve per skill (peak tier system) ──────────────────────────────

/// Subtle per-skill peak timing modifier within a group.
/// The main age development is already handled by per-group age ratios in generate_skills.
/// This only shifts individual skills slightly based on early/mid/late peak timing.
/// Range: 0.92 to 1.05 (not a major multiplier, just fine-tuning).
fn age_curve(skill_idx: usize, age: u32) -> f32 {
    let (peak_start, peak_end) = match skill_idx {
        // Early peak
        SK_ACCELERATION | SK_PACE | SK_AGILITY | SK_JUMPING | SK_BALANCE | SK_NATURAL_FITNESS => {
            (18u32, 24u32)
        }
        // Late peak
        SK_DECISIONS | SK_POSITIONING | SK_VISION | SK_LEADERSHIP | SK_COMPOSURE | SK_PASSING => {
            (26, 34)
        }
        // Mid peak (everything else)
        _ => (22, 28),
    };

    let age_f = age as f32;
    if age < peak_start {
        // Slight ramp before peak: 0.92 → 1.0
        let ramp_start = peak_start.saturating_sub(6) as f32;
        let t = ((age_f - ramp_start) / (peak_start as f32 - ramp_start)).clamp(0.0, 1.0);
        lerp(0.92, 1.0, t)
    } else if age <= peak_end {
        // At peak: slight bonus
        1.03
    } else {
        // Post-peak: 1.03 → 0.92
        let t = ((age_f - peak_end as f32) / (40.0 - peak_end as f32)).clamp(0.0, 1.0);
        lerp(1.03, 0.92, t)
    }
}

// ── Position weight tables ──────────────────────────────────────────────

/// Position weights with wide range (0.1-1.8) for clear skill differentiation.
/// Used additively via spread formula: skill = group_mean + (weight - 1.0) * spread
fn position_weights(position: &PositionType) -> [f32; SKILL_COUNT] {
    let mut w = [0.8f32; SKILL_COUNT];
    match position {
        PositionType::Goalkeeper => {
            // GK-critical
            w[SK_POSITIONING] = 1.8; w[SK_CONCENTRATION] = 1.6; w[SK_AGILITY] = 1.7;
            w[SK_ANTICIPATION] = 1.5; w[SK_COMPOSURE] = 1.5; w[SK_JUMPING] = 1.5;
            w[SK_BRAVERY] = 1.4; w[SK_DECISIONS] = 1.3; w[SK_STRENGTH] = 1.1;
            // Modern GK
            w[SK_FIRST_TOUCH] = 1.0; w[SK_PASSING] = 1.0; w[SK_TECHNIQUE] = 0.9;
            w[SK_NATURAL_FITNESS] = 1.0; w[SK_PACE] = 0.8; w[SK_STAMINA] = 0.8;
            w[SK_LEADERSHIP] = 1.0; w[SK_BALANCE] = 1.0;
            w[SK_DETERMINATION] = 1.0; w[SK_TEAMWORK] = 1.0;
            w[SK_PENALTY_TAKING] = 0.4;
            // Irrelevant outfield
            w[SK_FINISHING] = 0.1; w[SK_LONG_SHOTS] = 0.1; w[SK_CROSSING] = 0.1;
            w[SK_CORNERS] = 0.1; w[SK_FREE_KICKS] = 0.2; w[SK_HEADING] = 0.2;
            w[SK_OFF_THE_BALL] = 0.2; w[SK_DRIBBLING] = 0.3; w[SK_LONG_THROWS] = 0.4;
            w[SK_TACKLING] = 0.2; w[SK_MARKING] = 0.2; w[SK_WORK_RATE] = 0.4;
            w[SK_FLAIR] = 0.3; w[SK_ACCELERATION] = 0.6;
        }
        PositionType::Defender => {
            w[SK_TACKLING] = 1.6; w[SK_MARKING] = 1.6; w[SK_POSITIONING] = 1.5;
            w[SK_HEADING] = 1.4; w[SK_STRENGTH] = 1.4; w[SK_CONCENTRATION] = 1.4;
            w[SK_ANTICIPATION] = 1.3; w[SK_BRAVERY] = 1.3;
            w[SK_PACE] = 1.1; w[SK_JUMPING] = 1.1; w[SK_PASSING] = 1.0;
            w[SK_TEAMWORK] = 1.1; w[SK_DECISIONS] = 1.1; w[SK_COMPOSURE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0; w[SK_STAMINA] = 1.0;
            w[SK_FINISHING] = 0.3; w[SK_DRIBBLING] = 0.4; w[SK_FLAIR] = 0.3;
            w[SK_LONG_SHOTS] = 0.3; w[SK_OFF_THE_BALL] = 0.4; w[SK_VISION] = 0.5;
            w[SK_CROSSING] = 0.5; w[SK_CORNERS] = 0.3; w[SK_FREE_KICKS] = 0.4;
        }
        PositionType::Midfielder => {
            w[SK_PASSING] = 1.5; w[SK_VISION] = 1.4; w[SK_STAMINA] = 1.3;
            w[SK_TECHNIQUE] = 1.3; w[SK_FIRST_TOUCH] = 1.3; w[SK_DECISIONS] = 1.3;
            w[SK_TEAMWORK] = 1.3; w[SK_WORK_RATE] = 1.2;
            w[SK_DRIBBLING] = 1.1; w[SK_TACKLING] = 1.0; w[SK_POSITIONING] = 1.0;
            w[SK_COMPOSURE] = 1.1; w[SK_ANTICIPATION] = 1.1; w[SK_CONCENTRATION] = 1.0;
            w[SK_PACE] = 1.0; w[SK_ACCELERATION] = 1.0; w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_BALANCE] = 1.0;
            w[SK_HEADING] = 0.5; w[SK_LONG_THROWS] = 0.4; w[SK_FINISHING] = 0.5;
            w[SK_MARKING] = 0.6; w[SK_STRENGTH] = 0.7;
        }
        PositionType::Striker => {
            w[SK_FINISHING] = 1.7; w[SK_OFF_THE_BALL] = 1.5; w[SK_COMPOSURE] = 1.4;
            w[SK_DRIBBLING] = 1.3; w[SK_PACE] = 1.3; w[SK_FIRST_TOUCH] = 1.3;
            w[SK_ANTICIPATION] = 1.3; w[SK_ACCELERATION] = 1.3;
            w[SK_HEADING] = 1.1; w[SK_TECHNIQUE] = 1.1; w[SK_STRENGTH] = 1.0;
            w[SK_AGILITY] = 1.0; w[SK_BALANCE] = 1.0; w[SK_DECISIONS] = 1.0;
            w[SK_DETERMINATION] = 1.0; w[SK_BRAVERY] = 1.0; w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_TACKLING] = 0.2; w[SK_MARKING] = 0.2; w[SK_POSITIONING] = 0.4;
            w[SK_CONCENTRATION] = 0.6; w[SK_LONG_THROWS] = 0.3;
            w[SK_CORNERS] = 0.3; w[SK_FREE_KICKS] = 0.4;
        }
    }
    w
}

/// Apply a random role archetype to create variety within position groups.
fn apply_role_archetype(weights: &mut [f32; SKILL_COUNT], position: &PositionType) {
    let roll = rand::random::<f32>();
    match position {
        PositionType::Goalkeeper => {
            if roll < 0.35 {
                // Shot Stopper
                weights[SK_AGILITY] += 0.3; weights[SK_ANTICIPATION] += 0.2;
                weights[SK_CONCENTRATION] += 0.2;
            } else if roll < 0.60 {
                // Sweeper Keeper
                weights[SK_PACE] += 0.4; weights[SK_PASSING] += 0.4;
                weights[SK_FIRST_TOUCH] += 0.3; weights[SK_BRAVERY] += 0.2;
            } else if roll < 0.85 {
                // Commanding
                weights[SK_JUMPING] += 0.3; weights[SK_STRENGTH] += 0.3;
                weights[SK_LEADERSHIP] += 0.3; weights[SK_BRAVERY] += 0.2;
            } else {
                weights[SK_POSITIONING] += 0.1; weights[SK_CONCENTRATION] += 0.1;
            }
        }
        PositionType::Defender => {
            if roll < 0.25 {
                // Ball-Playing
                weights[SK_PASSING] += 0.4; weights[SK_FIRST_TOUCH] += 0.3;
                weights[SK_COMPOSURE] += 0.3; weights[SK_TECHNIQUE] += 0.2;
                weights[SK_AGGRESSION] -= 0.2; weights[SK_HEADING] -= 0.1;
            } else if roll < 0.50 {
                // Stopper
                weights[SK_AGGRESSION] += 0.3; weights[SK_HEADING] += 0.3;
                weights[SK_STRENGTH] += 0.3; weights[SK_BRAVERY] += 0.2;
                weights[SK_PASSING] -= 0.2; weights[SK_TECHNIQUE] -= 0.2;
            } else if roll < 0.75 {
                // Athletic
                weights[SK_PACE] += 0.4; weights[SK_ACCELERATION] += 0.3;
                weights[SK_AGILITY] += 0.2; weights[SK_STAMINA] += 0.2;
                weights[SK_STRENGTH] -= 0.2; weights[SK_HEADING] -= 0.1;
            } else {
                // No-Nonsense
                weights[SK_MARKING] += 0.3; weights[SK_TACKLING] += 0.2;
                weights[SK_POSITIONING] += 0.2;
                weights[SK_DRIBBLING] -= 0.3; weights[SK_FLAIR] -= 0.3;
            }
        }
        PositionType::Midfielder => {
            if roll < 0.20 {
                // Playmaker
                weights[SK_VISION] += 0.4; weights[SK_PASSING] += 0.3;
                weights[SK_TECHNIQUE] += 0.3; weights[SK_COMPOSURE] += 0.3;
                weights[SK_TACKLING] -= 0.3; weights[SK_STRENGTH] -= 0.2;
            } else if roll < 0.40 {
                // Box-to-Box
                weights[SK_STAMINA] += 0.4; weights[SK_WORK_RATE] += 0.3;
                weights[SK_TACKLING] += 0.3; weights[SK_STRENGTH] += 0.2;
                weights[SK_FLAIR] -= 0.2;
            } else if roll < 0.60 {
                // Ball Winner
                weights[SK_TACKLING] += 0.5; weights[SK_MARKING] += 0.4;
                weights[SK_AGGRESSION] += 0.3; weights[SK_STRENGTH] += 0.3;
                weights[SK_TECHNIQUE] -= 0.3; weights[SK_VISION] -= 0.3;
            } else if roll < 0.80 {
                // Winger
                weights[SK_PACE] += 0.5; weights[SK_CROSSING] += 0.5;
                weights[SK_DRIBBLING] += 0.4; weights[SK_ACCELERATION] += 0.3;
                weights[SK_TACKLING] -= 0.3; weights[SK_HEADING] -= 0.3;
            } else {
                // Mezzala
                weights[SK_DRIBBLING] += 0.4; weights[SK_OFF_THE_BALL] += 0.3;
                weights[SK_TECHNIQUE] += 0.3; weights[SK_ACCELERATION] += 0.2;
                weights[SK_MARKING] -= 0.2;
            }
        }
        PositionType::Striker => {
            if roll < 0.25 {
                // Poacher
                weights[SK_FINISHING] += 0.4; weights[SK_OFF_THE_BALL] += 0.4;
                weights[SK_ANTICIPATION] += 0.3; weights[SK_COMPOSURE] += 0.3;
                weights[SK_DRIBBLING] -= 0.3; weights[SK_PASSING] -= 0.3;
            } else if roll < 0.45 {
                // Target Man
                weights[SK_HEADING] += 0.5; weights[SK_STRENGTH] += 0.5;
                weights[SK_FIRST_TOUCH] += 0.3; weights[SK_BRAVERY] += 0.3;
                weights[SK_PACE] -= 0.4; weights[SK_ACCELERATION] -= 0.3;
            } else if roll < 0.65 {
                // Speed Merchant
                weights[SK_PACE] += 0.5; weights[SK_ACCELERATION] += 0.4;
                weights[SK_DRIBBLING] += 0.3; weights[SK_AGILITY] += 0.2;
                weights[SK_HEADING] -= 0.3; weights[SK_STRENGTH] -= 0.3;
            } else if roll < 0.85 {
                // Complete Forward
                weights[SK_TECHNIQUE] += 0.2; weights[SK_PASSING] += 0.2;
                weights[SK_VISION] += 0.2; weights[SK_DECISIONS] += 0.2;
            } else {
                // Deep-Lying Forward
                weights[SK_FIRST_TOUCH] += 0.4; weights[SK_PASSING] += 0.4;
                weights[SK_VISION] += 0.4; weights[SK_TECHNIQUE] += 0.3;
                weights[SK_HEADING] -= 0.3; weights[SK_OFF_THE_BALL] -= 0.2;
            }
        }
    }
}

/// Which broad group a skill index belongs to: 0=technical, 1=mental, 2=physical
fn skill_group(idx: usize) -> usize {
    if idx < 14 {
        0
    } else if idx < 28 {
        1
    } else {
        2
    }
}

// ── Skill affinity correlations ─────────────────────────────────────────

fn apply_affinities(skills: &mut [f32; SKILL_COUNT]) {
    // High passing → boost vision, first_touch
    if skills[SK_PASSING] > 14.0 {
        let bonus = (skills[SK_PASSING] - 14.0) * 0.15;
        skills[SK_VISION] += bonus;
        skills[SK_FIRST_TOUCH] += bonus;
    }
    // High aggression → boost bravery, reduce composure
    if skills[SK_AGGRESSION] > 14.0 {
        let bonus = (skills[SK_AGGRESSION] - 14.0) * 0.12;
        skills[SK_BRAVERY] += bonus;
        skills[SK_COMPOSURE] -= bonus * 0.5;
    }
    // High pace → boost acceleration
    if skills[SK_PACE] > 14.0 {
        let bonus = (skills[SK_PACE] - 14.0) * 0.15;
        skills[SK_ACCELERATION] += bonus;
    }
    // High finishing → boost composure, anticipation
    if skills[SK_FINISHING] > 14.0 {
        let bonus = (skills[SK_FINISHING] - 14.0) * 0.1;
        skills[SK_COMPOSURE] += bonus;
        skills[SK_ANTICIPATION] += bonus;
    }
    // High dribbling → boost flair, agility
    if skills[SK_DRIBBLING] > 14.0 {
        let bonus = (skills[SK_DRIBBLING] - 14.0) * 0.12;
        skills[SK_FLAIR] += bonus;
        skills[SK_AGILITY] += bonus;
    }
    // High leadership → boost determination, teamwork
    if skills[SK_LEADERSHIP] > 14.0 {
        let bonus = (skills[SK_LEADERSHIP] - 14.0) * 0.1;
        skills[SK_DETERMINATION] += bonus;
        skills[SK_TEAMWORK] += bonus;
    }
}

/// Convert a flat [f32; 37] array back into PlayerSkills.
fn skills_from_array(arr: &[f32; SKILL_COUNT]) -> PlayerSkills {
    PlayerSkills {
        technical: Technical {
            corners: arr[SK_CORNERS],
            crossing: arr[SK_CROSSING],
            dribbling: arr[SK_DRIBBLING],
            finishing: arr[SK_FINISHING],
            first_touch: arr[SK_FIRST_TOUCH],
            free_kicks: arr[SK_FREE_KICKS],
            heading: arr[SK_HEADING],
            long_shots: arr[SK_LONG_SHOTS],
            long_throws: arr[SK_LONG_THROWS],
            marking: arr[SK_MARKING],
            passing: arr[SK_PASSING],
            penalty_taking: arr[SK_PENALTY_TAKING],
            tackling: arr[SK_TACKLING],
            technique: arr[SK_TECHNIQUE],
        },
        mental: Mental {
            aggression: arr[SK_AGGRESSION],
            anticipation: arr[SK_ANTICIPATION],
            bravery: arr[SK_BRAVERY],
            composure: arr[SK_COMPOSURE],
            concentration: arr[SK_CONCENTRATION],
            decisions: arr[SK_DECISIONS],
            determination: arr[SK_DETERMINATION],
            flair: arr[SK_FLAIR],
            leadership: arr[SK_LEADERSHIP],
            off_the_ball: arr[SK_OFF_THE_BALL],
            positioning: arr[SK_POSITIONING],
            teamwork: arr[SK_TEAMWORK],
            vision: arr[SK_VISION],
            work_rate: arr[SK_WORK_RATE],
        },
        physical: Physical {
            acceleration: arr[SK_ACCELERATION],
            agility: arr[SK_AGILITY],
            balance: arr[SK_BALANCE],
            jumping: arr[SK_JUMPING],
            natural_fitness: arr[SK_NATURAL_FITNESS],
            pace: arr[SK_PACE],
            stamina: arr[SK_STAMINA],
            strength: arr[SK_STRENGTH],
            match_readiness: arr[SK_MATCH_READINESS],
        },
    }
}

// ── Types ───────────────────────────────────────────────────────────────

pub struct PlayerGenerator {
    people_names_data: PeopleNameGeneratorData,
}

impl PlayerGenerator {
    pub fn with_people_names(people_names: &PeopleNameGeneratorData) -> Self {
        PlayerGenerator {
            people_names_data: PeopleNameGeneratorData {
                first_names: people_names.first_names.clone(),
                last_names: people_names.last_names.clone(),
                nicknames: people_names.nicknames.clone(),
            },
        }
    }
}

#[derive(Copy, Clone)]
pub enum PositionType {
    Goalkeeper,
    Defender,
    Midfielder,
    Striker,
}

impl PlayerGenerator {
    pub fn generate(
        &mut self,
        country_id: u32,
        position: PositionType,
        team_reputation: u16,
        country_reputation: u16,
        min_age: i32,
        max_age: i32,
        is_youth: bool,
    ) -> Player {
        let now = Utc::now();

        // Blend team rep (70%) with country rep (30%) so players from stronger
        // football nations are naturally better, even at weaker clubs.
        let blended_rep = team_reputation as f32 * 0.7 + country_reputation as f32 * 0.3;
        let rep_factor = (blended_rep / 10000.0).clamp(0.0, 1.0);

        let year = IntegerUtils::random(now.year() - max_age, now.year() - min_age) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 29) as u32;
        let age = (now.year() as u32).saturating_sub(year);

        let first_name = self.generate_first_name();
        let last_name = self.generate_last_name();
        let full_name = match self.generate_nickname() {
            Some(nickname) => FullName::with_nickname(first_name, last_name, nickname),
            None => FullName::new(first_name, last_name),
        };

        // Generate PA first so skills can target the right ability level
        let positions = Self::generate_positions(position);
        let potential_ability = Self::generate_potential_ability(rep_factor, age);

        // Skills target a CA appropriate for this PA and age, not just team rep
        let skills = Self::generate_skills(&position, age, rep_factor, potential_ability);
        let player_attributes =
            Self::generate_player_attributes(rep_factor, age, potential_ability, &skills, &positions);

        // FM-style salary: exponential curve based on reputation and ability.
        // Salaries in USD/year (annual). Massive gaps between tiers:
        //   rep_factor ~0.05 (amateur)     →    1K -    3K
        //   rep_factor ~0.15 (Chad/Malta)  →    3K -   12K
        //   rep_factor ~0.30 (Ghana/Nigeria)→   10K -   50K
        //   rep_factor ~0.50 (mid European)→   40K -  200K
        //   rep_factor ~0.65 (Eredivisie)  →  100K -  600K
        //   rep_factor ~0.80 (Serie A/BuLi)→  300K - 2.5M
        //   rep_factor ~0.90 (PL/La Liga)  →  600K - 6M
        //   rep_factor ~1.00 (elite)       →  1.2M - 12M
        let curve = rep_factor * rep_factor * rep_factor; // cubic — steep growth at top
        let salary_min = (1_000.0 + curve * 1_200_000.0) as i32;
        let salary_max = (3_000.0 + curve * 12_000_000.0) as i32;

        // Ability factor: salary scales with current ability (quadratic)
        // CA 200 → 1.0, CA 100 → 0.25, CA 50 → 0.0625
        // Keeps low-ability players from earning elite wages at big clubs
        let ca_normalized = player_attributes.current_ability as f64 / 200.0;
        let ability_salary_factor = (ca_normalized * ca_normalized).clamp(0.05, 1.0);

        // Age factor: peak earners 25-30, young players earn less
        let age_salary_factor = match age {
            0..=17 => 0.08,
            18 => 0.12,
            19 => 0.18,
            20 => 0.30,
            21 => 0.45,
            22 => 0.60,
            23 => 0.75,
            24 => 0.88,
            25..=30 => 1.0,
            31 => 0.85,
            32 => 0.70,
            33 => 0.55,
            34 => 0.40,
            _ => 0.30,
        };

        let base_salary = (IntegerUtils::random(salary_min, salary_max) as f64 * age_salary_factor * ability_salary_factor) as u32;
        let salary = if is_youth {
            (base_salary / 10).max(200)
        } else {
            base_salary.max(500)
        };
        let expiration =
            NaiveDate::from_ymd_opt(now.year() + IntegerUtils::random(1, 5), 3, 14).unwrap();

        let contract = if is_youth {
            PlayerClubContract::new_youth(salary, expiration)
        } else {
            PlayerClubContract::new(salary, expiration)
        };

        // Native languages based on player's nationality
        let native_languages: Vec<core::PlayerLanguage> = core::Language::from_country_code(
            &crate::loaders::CountryLoader::code_for_id(country_id)
        )
            .into_iter()
            .map(|lang| core::PlayerLanguage::native(lang))
            .collect();

        Player::builder()
            .id(PLAYER_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst))
            .full_name(full_name)
            .birth_date(NaiveDate::from_ymd_opt(year as i32, month, day).unwrap())
            .country_id(country_id)
            .skills(skills)
            .attributes(Self::generate_person_attributes())
            .player_attributes(player_attributes)
            .contract(Some(contract))
            .positions(positions)
            .languages(native_languages)
            .build()
            .expect("Failed to build Player")
    }

    // ── Skill generation pipeline ───────────────────────────────────────

    fn generate_skills(position: &PositionType, age: u32, rep_factor: f32, potential_ability: u8) -> PlayerSkills {
        // ── PA is the anchor ───────────────────────────────────────────────
        // PA maps to a "fully developed" skill level: what this player's average
        // skill would be at peak. Position weights create FM-like differentiation
        // via ADDITIVE spread so even PA 15 players have clear strengths/weaknesses.

        let pa = potential_ability as f32;
        // Final skill level this PA implies (PA 1→1, PA 100→10.5, PA 200→20)
        let pa_final = (pa - 1.0) / 199.0 * 19.0 + 1.0;

        // Age-dependent development ratio per skill group
        // Technical develops early (ball work from childhood), peaks mid-20s
        let tech_age_ratio = match age {
            0..=17 =>  0.75,
            18..=19 => 0.82,
            20..=22 => 0.90,
            23..=26 => 0.95,
            27..=29 => 1.0,
            30..=32 => 0.97,
            _ =>       0.93,
        };
        // Mental develops late (experience, reading the game), peaks early 30s
        let mental_age_ratio = match age {
            0..=17 =>  0.55,
            18..=19 => 0.62,
            20..=22 => 0.72,
            23..=26 => 0.85,
            27..=29 => 0.95,
            30..=32 => 1.0,
            _ =>       1.0,
        };
        // Physical peaks mid-20s, declines after 30
        let physical_age_ratio = match age {
            0..=17 =>  0.70,
            18..=19 => 0.78,
            20..=22 => 0.88,
            23..=26 => 0.95,
            27..=29 => 1.0,
            30..=32 => 0.93,
            _ =>       0.82,
        };

        // Group means: pure PA-driven
        let tech_mean   = pa_final * tech_age_ratio;
        let mental_mean = pa_final * mental_age_ratio;
        let phys_mean   = pa_final * physical_age_ratio;

        // Position spread: how much key/weak skills deviate from group mean.
        // Ensures differentiation at ALL ability levels:
        //   PA 15 (pa_final ~2.3): spread ~2.5 → key ~4, weak ~1
        //   PA 50 (pa_final ~5.7): spread ~2.8 → key ~8, weak ~3
        //   PA 100 (pa_final ~10.5): spread ~5.2 → key ~14, weak ~5
        //   PA 180 (pa_final ~18.1): spread ~9.1 → key ~20, weak ~9
        let spread = (pa_final * 0.5).max(2.5);

        let mut pos_w = position_weights(position);
        apply_role_archetype(&mut pos_w, position);

        // Noise per group
        let base_noise = 1.5 + rep_factor * 1.0;
        let tech_noise = if age <= 18 { base_noise + 2.0 } else { base_noise + 0.5 };
        let mental_noise = base_noise * 0.5;
        let phys_noise = base_noise * 1.5;

        let mut skills = [0.0f32; SKILL_COUNT];

        for i in 0..SKILL_COUNT {
            // 1. Pick the correct group mean and noise
            let (group_mean, noise) = match skill_group(i) {
                0 => (tech_mean, tech_noise),
                1 => (mental_mean, mental_noise),
                _ => (phys_mean, phys_noise),
            };

            // 2. Additive position spread: key skills (w>1) get bonus, weak skills (w<1) get penalty
            //    w=1.7 → group_mean + 0.7 * spread (big boost)
            //    w=0.2 → group_mean - 0.8 * spread (big penalty)
            let pos_mean = group_mean + (pos_w[i] - 1.0) * spread;
            let base = pos_mean + random_normal() * noise;

            // 3. Apply per-skill age curve for individual peak timing
            let raw = base * age_curve(i, age);

            // 4. Clamp
            skills[i] = raw.clamp(1.0, 20.0);
        }

        // 5. Mental cohesion: pull toward group mean (mentality is unified)
        let m_start = 14;
        let m_end = 28;
        let m_count = (m_end - m_start) as f32;
        let m_avg: f32 = skills[m_start..m_end].iter().sum::<f32>() / m_count;
        for i in m_start..m_end {
            skills[i] = skills[i] * 0.70 + m_avg * 0.30;
        }

        // 6. Physical cohesion: light pull toward group mean (keep individuality)
        let p_start = 28;
        let p_end = SKILL_COUNT;
        let p_count = (p_end - p_start) as f32;
        let p_avg: f32 = skills[p_start..p_end].iter().sum::<f32>() / p_count;
        for i in p_start..p_end {
            skills[i] = skills[i] * 0.85 + p_avg * 0.15;
        }

        // 7. PA-based floors
        let key_floor = (pa_final * 0.40).clamp(1.0, 9.0);
        // Universal minimum: no skill should be 1 for any professional player.
        // Even a bad GK can pass at 3. Even a striker has basic tackling at 2-3.
        // PA 20 (pa_final ~2.8) → floor 2, PA 70 (pa_final ~7.6) → floor 3, PA 150 → floor 4
        let universal_floor = (1.0 + pa_final * 0.3).clamp(2.0, 5.0);
        // Physical floor: footballers are professional athletes — even low-PA players
        // should have reasonable physical attributes, not 2-3 like untrained people.
        // PA 15 → 5, PA 50 → 5, PA 100 → 6.7, PA 150 → 8
        let physical_floor_base = (3.0 + pa_final * 0.35).clamp(6.0, 9.0);
        // Technical floor: all professional footballers train technical skills daily.
        // Position-trained skills (weight >= 0.8) get the full trained floor.
        // Other technical skills get a lower but still decent "footballer floor".
        // Mental skills use universal floor only — they develop with age/experience.
        let trained_floor = (pa_final * 0.35 + 3.0).clamp(6.0, 9.0);
        let footballer_tech_floor = (pa_final * 0.25 + 2.0).clamp(4.0, 7.0);
        for i in 0..SKILL_COUNT {
            if pos_w[i] >= 1.2 {
                skills[i] = skills[i].max(key_floor);
            }
            if skill_group(i) == 2 {
                // Physical: per-skill jitter so not every physical lands at the same value
                let jitter = (random_normal() * 2.5).clamp(-3.0, 3.0);
                let floor = (physical_floor_base + jitter).max(4.0);
                skills[i] = skills[i].max(floor);
            } else if skill_group(i) == 0 && pos_w[i] >= 0.8 {
                // Technical skills this position trains regularly
                let jitter = (random_normal() * 1.5).clamp(-2.0, 2.0);
                let floor = (trained_floor + jitter).max(4.0);
                skills[i] = skills[i].max(floor);
            } else if skill_group(i) == 0 {
                // All other technical skills — footballers can still pass, shoot, etc.
                let jitter = (random_normal() * 1.5).clamp(-2.0, 2.0);
                let floor = (footballer_tech_floor + jitter).max(3.0);
                skills[i] = skills[i].max(floor);
            } else {
                skills[i] = skills[i].max(universal_floor);
            }
        }

        // 8. Apply affinities
        apply_affinities(&mut skills);

        // 9. Final clamp
        for v in skills.iter_mut() {
            *v = v.clamp(1.0, 20.0);
        }

        skills_from_array(&skills)
    }

    // ── Position generation ─────────────────────────────────────────────

    fn generate_positions(position: PositionType) -> PlayerPositions {
        let mut positions = Vec::with_capacity(5);

        match position {
            PositionType::Goalkeeper => positions.push(PlayerPosition {
                position: PlayerPositionType::Goalkeeper,
                level: 20,
            }),
            PositionType::Defender => match IntegerUtils::random(0, 5) {
                0 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::DefenderLeft,
                        level: 20,
                    });
                }
                1 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::DefenderCenterLeft,
                        level: 20,
                    });
                }
                2 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::DefenderCenter,
                        level: 20,
                    });
                }
                3 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::DefenderCenterRight,
                        level: 20,
                    });
                }
                4 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::DefenderRight,
                        level: 20,
                    });
                }
                _ => {}
            },
            PositionType::Midfielder => match IntegerUtils::random(0, 7) {
                0 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::MidfielderLeft,
                        level: 20,
                    });
                }
                1 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::MidfielderCenterLeft,
                        level: 20,
                    });
                }
                2 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    });
                }
                3 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::MidfielderCenterRight,
                        level: 20,
                    });
                }
                4 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::MidfielderRight,
                        level: 20,
                    });
                }
                5 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::WingbackLeft,
                        level: 20,
                    });
                }
                6 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::WingbackRight,
                        level: 20,
                    });
                }
                _ => {}
            },
            PositionType::Striker => match IntegerUtils::random(0, 4) {
                0 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::Striker,
                        level: 20,
                    });
                }
                1 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::ForwardLeft,
                        level: 20,
                    });
                }
                2 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::ForwardCenter,
                        level: 20,
                    });
                }
                3 => {
                    positions.push(PlayerPosition {
                        position: PlayerPositionType::ForwardRight,
                        level: 20,
                    });
                }
                _ => {}
            },
        }

        PlayerPositions { positions }
    }

    // ── Person attributes ───────────────────────────────────────────────

    fn generate_person_attributes() -> PersonAttributes {
        PersonAttributes {
            adaptability: FloatUtils::random(0.0f32, 20.0f32),
            ambition: FloatUtils::random(0.0f32, 20.0f32),
            controversy: FloatUtils::random(0.0f32, 20.0f32),
            loyalty: FloatUtils::random(0.0f32, 20.0f32),
            pressure: FloatUtils::random(0.0f32, 20.0f32),
            professionalism: FloatUtils::random(0.0f32, 20.0f32),
            sportsmanship: FloatUtils::random(0.0f32, 20.0f32),
            temperament: FloatUtils::random(0.0f32, 20.0f32),
        }
    }

    // ── Potential ability (generated before skills) ─────────────────────

    fn generate_potential_ability(rep_factor: f32, age: u32) -> u8 {
        // Three-tier PA distribution:
        //   Normal:   majority of squad — ability matches club level
        //   Standout: ~6-10% — notably better than club level (every club has 1-2)
        //   Gem:      ~1-3% — exceptional talent well above club level
        //
        // Floriana (rep 0.265): normal ~26, standout ~55-85, gem ~80-120
        // Premier League (rep 0.90): normal ~145, standout ~155-180, gem ~170-195

        let roll = rand::random::<f32>();

        // Gem: rare exceptional talent
        let gem_chance = (0.01 + rep_factor * rep_factor * 0.04).min(0.05);
        // Standout: every club has a few above-average players
        let standout_chance = gem_chance + 0.06 + rep_factor * 0.04;

        if roll < gem_chance {
            // Gem: PA well above club range
            let gem_min = (70.0 + rep_factor * 60.0) as i32;
            let gem_max = (100.0 + rep_factor * 95.0).min(195.0) as i32;
            IntegerUtils::random(gem_min, gem_max).min(200) as u8
        } else if roll < standout_chance {
            // Standout: clearly best player at the club, PA 1.5-2.5x normal range
            let standout_base = 45.0 + rep_factor * 90.0;
            let noise = random_normal() * 10.0;
            let pa = standout_base + noise;
            pa.clamp(30.0, 185.0) as u8
        } else {
            // Normal: bulk of squad
            // Floor of 25 ensures even the smallest clubs (Floriana, Chad league)
            // produce players with visible skill differentiation, not all-3 profiles
            let base = 25.0 + rep_factor * rep_factor * 150.0;
            let youth_bonus = if age <= 21 { 8.0 } else if age <= 25 { 3.0 } else { 0.0 };
            let noise = random_normal() * (8.0 + rep_factor * 10.0);
            let pa = base + youth_bonus + noise;
            pa.clamp(20.0, 190.0) as u8
        }
    }

    // ── Player attributes (CA from skills, PA already determined) ─────

    fn generate_player_attributes(
        rep_factor: f32,
        age: u32,
        potential_ability: u8,
        skills: &PlayerSkills,
        positions: &PlayerPositions,
    ) -> PlayerAttributes {
        // Current ability: derived from actual generated skills
        let primary_position = positions
            .positions
            .first()
            .map(|p| p.position)
            .unwrap_or(PlayerPositionType::MidfielderCenter);
        let current_ability = skills.calculate_ability_for_position(primary_position);

        let rep_base = (rep_factor * 3000.0) as i32;

        // U21 caps
        let u21_apps = if age < 17 {
            0
        } else {
            let u21_years = (age.min(23) - 17) as i32;
            let max_u21 = (u21_years as f32 * rep_factor * 8.0) as i32;
            IntegerUtils::random(0, max_u21.max(1)) as u16
        };
        let _u21_goals = if u21_apps > 0 {
            IntegerUtils::random(0, (u21_apps as f32 * 0.35) as i32) as u16
        } else {
            0
        };

        PlayerAttributes {
            is_banned: false,
            is_injured: false,
            condition: IntegerUtils::random(3000, 10000) as i16,
            fitness: IntegerUtils::random(3000, 10000) as i16,
            jadedness: IntegerUtils::random(0, 5000) as i16,
            weight: IntegerUtils::random(60, 100) as u8,
            height: IntegerUtils::random(150, 220) as u8,
            value: 0,
            current_reputation: IntegerUtils::random(
                (rep_base as f32 * 0.3) as i32,
                rep_base,
            ) as i16,
            home_reputation: IntegerUtils::random(
                (rep_base as f32 * 0.5) as i32,
                rep_base,
            ) as i16,
            world_reputation: IntegerUtils::random(
                (rep_base as f32 * 0.1) as i32,
                (rep_base as f32 * 0.4) as i32,
            ) as i16,
            current_ability,
            potential_ability,
            international_apps: 0,
            international_goals: 0,
            under_21_international_apps: 0,
            under_21_international_goals: 0,
            injury_days_remaining: 0,
            injury_type: None,
            injury_proneness: (IntegerUtils::random(1, 10) + IntegerUtils::random(1, 10)) as u8,
            recovery_days_remaining: 0,
            last_injury_body_part: 0,
            injury_count: 0,
            days_since_last_match: 0,
        }
    }

    // ── Name generation ─────────────────────────────────────────────────

    fn generate_nickname(&self) -> Option<String> {
        if self.people_names_data.nicknames.is_empty() {
            return None;
        }
        if IntegerUtils::random(0, 9) != 0 {
            return None;
        }
        let idx = IntegerUtils::random(0, self.people_names_data.nicknames.len() as i32) as usize;
        Some(self.people_names_data.nicknames[idx].to_owned())
    }

    fn generate_first_name(&self) -> String {
        let names = &self.people_names_data.first_names;
        if names.is_empty() { return String::new(); }
        let idx = IntegerUtils::random(0, names.len() as i32 - 1) as usize;
        names[idx].to_owned()
    }

    fn generate_last_name(&self) -> String {
        let names = &self.people_names_data.last_names;
        if names.is_empty() { return String::new(); }
        let idx = IntegerUtils::random(0, names.len() as i32 - 1) as usize;
        names[idx].to_owned()
    }
}
