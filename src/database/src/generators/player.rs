use crate::loaders::{OdbPlayer, OdbPosition};
use chrono::{Datelike, NaiveDate, Utc};
use core::shared::FullName;
use core::utils::{FloatUtils, IntegerUtils};
use core::{
    ContractType, Mental, PeopleNameGeneratorData, PersonAttributes, Physical, Player,
    PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerPreferredFoot, PlayerSkills, Technical,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

static PLAYER_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(1));

/// Bump the procedural id sequence so the next generated player gets an id
/// strictly greater than `min_exclusive`. No-op if the counter is already
/// past it. Called before generation so generated players cannot collide
/// with ids supplied by `players.odb`.
pub fn seed_player_id_sequence(min_exclusive: u32) {
    let target = min_exclusive.saturating_add(1);
    let mut current = PLAYER_ID_SEQUENCE.load(Ordering::SeqCst);
    while current < target {
        match PLAYER_ID_SEQUENCE.compare_exchange(
            current,
            target,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

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

// ── Age-based skill cap ────────────────────────────────────────────────

/// Young players cannot reach elite skill levels regardless of talent.
/// Consistent with the core generator's age_skill_cap.
fn age_skill_cap(age: u32) -> f32 {
    match age {
        0..=14 => 12.0,
        15 => 13.0,
        16 => 14.0,
        17 => 15.0,
        18 => 15.5,
        19 => 16.0,
        20 => 17.0,
        21 => 17.5,
        22 => 18.0,
        23 => 18.5,
        24 => 19.0,
        25..=30 => 20.0,
        31..=33 => 19.0,
        34..=35 => 18.0,
        _ => 17.0,
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
            // Modern GK — ball-playing ability
            w[SK_FIRST_TOUCH] = 1.1; w[SK_PASSING] = 1.1; w[SK_TECHNIQUE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0; w[SK_PACE] = 0.8; w[SK_STAMINA] = 0.8;
            w[SK_LEADERSHIP] = 1.0; w[SK_BALANCE] = 1.0;
            w[SK_DETERMINATION] = 1.0; w[SK_TEAMWORK] = 1.0;
            w[SK_PENALTY_TAKING] = 0.4;
            // Secondary outfield — professional level for all skills
            w[SK_FINISHING] = 0.5; w[SK_LONG_SHOTS] = 0.5; w[SK_CROSSING] = 0.5;
            w[SK_CORNERS] = 0.5; w[SK_FREE_KICKS] = 0.55; w[SK_HEADING] = 0.55;
            w[SK_OFF_THE_BALL] = 0.5; w[SK_DRIBBLING] = 0.55; w[SK_LONG_THROWS] = 0.6;
            w[SK_TACKLING] = 0.55; w[SK_MARKING] = 0.55; w[SK_WORK_RATE] = 0.6;
            w[SK_FLAIR] = 0.5; w[SK_ACCELERATION] = 0.7;
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
        goalkeeping: Default::default(),
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
        continent_id: u32,
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
        let potential_ability = Self::generate_potential_ability(rep_factor, age);
        let positions = Self::generate_positions(position, potential_ability);

        // Skills target a CA appropriate for this PA and age, not just team rep
        let country_code = crate::loaders::CountryLoader::code_for_id(country_id);
        let skills = Self::generate_skills(&position, age, rep_factor, potential_ability, continent_id, &country_code);
        let player_attributes =
            Self::generate_player_attributes(rep_factor, age, potential_ability, &skills, &positions);

        // Salary: exponential curve based on reputation and ability.
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
            Self::youth_salary(player_attributes.current_ability)
        } else {
            base_salary.max(Self::reserve_salary(player_attributes.current_ability))
        };
        let contract_years = Self::generate_contract_years(age, player_attributes.current_ability, player_attributes.current_reputation);
        let expiration =
            NaiveDate::from_ymd_opt(now.year() + contract_years, 3, 14).unwrap();

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
            .generated(true)
            .build()
            .expect("Failed to build Player")
    }

    // ── Skill generation pipeline ───────────────────────────────────────

    fn generate_skills(position: &PositionType, age: u32, rep_factor: f32, potential_ability: u8, continent_id: u32, country_code: &str) -> PlayerSkills {
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

        // 7. PA-based floors and age cap
        let key_floor = (pa_final * 0.40).clamp(1.0, 9.0);
        // Universal minimum: no professional footballer should have any skill at 1-3.
        // PA 20 (pa_final ~2.8) → floor 4, PA 70 → floor 4, PA 150 → floor 5
        let universal_floor = (2.0 + pa_final * 0.2).clamp(4.0, 6.0);
        // Physical floor: footballers are professional athletes — even low-PA players
        // should have reasonable physical attributes, not 2-3 like untrained people.
        // PA 15 → 6, PA 50 → 6, PA 100 → 6.7, PA 150 → 8
        let physical_floor_base = (3.0 + pa_final * 0.35).clamp(6.0, 9.0);
        // Technical floor: all professional footballers train technical skills daily.
        // Position-trained skills (weight >= 0.8) get the full trained floor.
        // Other technical skills get a lower but still decent "footballer floor".
        // Mental skills use universal floor only — they develop with age/experience.
        let trained_floor = (pa_final * 0.35 + 3.0).clamp(6.0, 9.0);
        // PA 50 → floor ~5, PA 100 → floor ~6, PA 150 → floor ~7, PA 180 → floor ~8
        let footballer_tech_floor = (pa_final * 0.30 + 2.0).clamp(4.0, 9.0);
        let cap = age_skill_cap(age);
        for i in 0..SKILL_COUNT {
            if pos_w[i] >= 1.2 {
                skills[i] = skills[i].max(key_floor);
            }
            if skill_group(i) == 2 {
                // Physical: per-skill jitter so not every physical lands at the same value
                let jitter = (random_normal() * 2.0).clamp(-2.0, 2.0);
                let floor = (physical_floor_base + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else if skill_group(i) == 0 && pos_w[i] >= 0.8 {
                // Technical skills this position trains regularly
                let jitter = (random_normal() * 1.5).clamp(-2.0, 2.0);
                let floor = (trained_floor + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else if skill_group(i) == 0 {
                // All other technical skills — footballers can still pass, shoot, etc.
                let jitter = (random_normal() * 1.0).clamp(-1.0, 1.0);
                let floor = (footballer_tech_floor + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else {
                skills[i] = skills[i].clamp(universal_floor, cap);
            }
        }

        // 8. Apply affinities
        apply_affinities(&mut skills);

        // 9. Country-specific bias — national football culture
        let bias = super::country_bias::country_skill_bias(continent_id, country_code);
        for i in 0..SKILL_COUNT {
            skills[i] += bias[i];
        }

        // 10. Final clamp
        for v in skills.iter_mut() {
            *v = v.clamp(1.0, 20.0);
        }

        let mut result = skills_from_array(&skills);

        // 10. Generate goalkeeper-specific skills from the same PA/age budget
        if matches!(position, PositionType::Goalkeeper) {
            result.goalkeeping = Self::generate_gk_skills(pa_final, age, &pos_w);
        }

        result
    }

    /// Generate Goalkeeping-specific skills from the PA budget.
    /// Based on real FM attribute importance:
    ///   Core (shot-stopping): Handling, Reflexes, One-on-Ones — highest weight
    ///   Command: Command of Area, Aerial Reach, Communication, Punching
    ///   Distribution: Kicking, Throwing, First Touch, Passing
    ///   Specialist: Rushing Out, Eccentricity
    fn generate_gk_skills(pa_final: f32, age: u32, _pos_w: &[f32; SKILL_COUNT]) -> core::Goalkeeping {
        // GK skills develop like mental — peak in late 20s/early 30s (experience matters)
        let gk_age_ratio = match age {
            0..=17 =>  0.60,
            18..=19 => 0.70,
            20..=22 => 0.80,
            23..=26 => 0.90,
            27..=29 => 0.97,
            30..=34 => 1.0,
            _ =>       0.95,
        };

        let gk_mean = pa_final * gk_age_ratio;
        let spread = (pa_final * 0.45).max(2.0);
        let noise = 1.5;

        // GK role archetype — creates variety between keepers
        let roll = rand::random::<f32>();
        // Weights: 1.0 = average, >1.0 = boosted, <1.0 = reduced
        let (_archetype_name, w) = if roll < 0.35 {
            // Shot Stopper — elite reflexes, handling, positioning
            ("shot_stopper", [
                0.9,  // aerial_reach
                0.9,  // command_of_area
                0.8,  // communication
                0.4,  // eccentricity
                0.6,  // first_touch
                1.6,  // handling
                0.7,  // kicking
                1.3,  // one_on_ones
                0.6,  // passing
                1.1,  // punching
                1.7,  // reflexes
                0.8,  // rushing_out
                0.7,  // throwing
            ])
        } else if roll < 0.60 {
            // Sweeper Keeper — distribution, rushing out, brave
            ("sweeper_keeper", [
                0.8,  // aerial_reach
                1.0,  // command_of_area
                1.0,  // communication
                1.2,  // eccentricity
                1.5,  // first_touch
                1.1,  // handling
                1.3,  // kicking
                1.2,  // one_on_ones
                1.4,  // passing
                0.7,  // punching
                1.1,  // reflexes
                1.5,  // rushing_out
                1.2,  // throwing
            ])
        } else if roll < 0.82 {
            // Commanding — aerial dominance, communication, set-piece defense
            ("commanding", [
                1.6,  // aerial_reach
                1.5,  // command_of_area
                1.4,  // communication
                0.5,  // eccentricity
                0.7,  // first_touch
                1.2,  // handling
                0.9,  // kicking
                1.0,  // one_on_ones
                0.7,  // passing
                1.3,  // punching
                1.1,  // reflexes
                0.9,  // rushing_out
                0.8,  // throwing
            ])
        } else {
            // All-Rounder — balanced
            ("all_rounder", [
                1.0, 1.0, 1.0, 0.7,
                1.0, 1.2, 1.0, 1.1,
                0.9, 0.9, 1.2, 1.0, 0.9,
            ])
        };

        // Generate each GK skill
        let mut gk_skills = [0.0f32; 13];
        for i in 0..13 {
            let pos_mean = gk_mean + (w[i] - 1.0) * spread;
            let raw = pos_mean + random_normal() * noise;
            gk_skills[i] = raw.clamp(1.0, 20.0);
        }

        // GK skill floor: core skills should be at least proportional to PA
        let core_floor = (pa_final * 0.45).clamp(3.0, 10.0);
        let general_floor = (pa_final * 0.25).clamp(2.0, 7.0);

        // Core skills (indices: 5=handling, 10=reflexes, 7=one_on_ones)
        gk_skills[5] = gk_skills[5].max(core_floor);  // handling
        gk_skills[10] = gk_skills[10].max(core_floor); // reflexes
        gk_skills[7] = gk_skills[7].max(core_floor);   // one_on_ones

        // All other skills get general floor
        for i in 0..13 {
            gk_skills[i] = gk_skills[i].max(general_floor).clamp(1.0, 20.0);
        }

        core::Goalkeeping {
            aerial_reach:    gk_skills[0],
            command_of_area: gk_skills[1],
            communication:   gk_skills[2],
            eccentricity:    gk_skills[3],
            first_touch:     gk_skills[4],
            handling:        gk_skills[5],
            kicking:         gk_skills[6],
            one_on_ones:     gk_skills[7],
            passing:         gk_skills[8],
            punching:        gk_skills[9],
            reflexes:        gk_skills[10],
            rushing_out:     gk_skills[11],
            throwing:        gk_skills[12],
        }
    }

    // ── Position generation ─────────────────────────────────────────────

    /// Generate position profile. Primary position always 20.
    /// Higher PA → higher chance of having a secondary position.
    ///
    /// DCL/DCR and MCL/MCR are formation slots, not primary positions.
    /// DC players automatically get DCL/DCR, MC players get MCL/MCR.
    /// Wide players have a chance of cross-side versatility (e.g. ML + MR).
    fn generate_positions(position: PositionType, potential_ability: u8) -> PlayerPositions {
        let mut positions = Vec::with_capacity(6);

        let primary = match position {
            PositionType::Goalkeeper => PlayerPositionType::Goalkeeper,
            PositionType::Defender => match IntegerUtils::random(0, 8) {
                0 => PlayerPositionType::DefenderLeft,
                1 | 2 | 3 | 4 => PlayerPositionType::DefenderCenter,
                5 => PlayerPositionType::DefenderRight,
                6 => PlayerPositionType::WingbackLeft,
                _ => PlayerPositionType::WingbackRight,
            },
            PositionType::Midfielder => match IntegerUtils::random(0, 6) {
                0 => PlayerPositionType::MidfielderLeft,
                1 => PlayerPositionType::MidfielderRight,
                2 | 3 | 4 => PlayerPositionType::MidfielderCenter,
                _ => PlayerPositionType::DefensiveMidfielder,
            },
            PositionType::Striker => match IntegerUtils::random(0, 5) {
                0 => PlayerPositionType::Striker,
                1 => PlayerPositionType::ForwardLeft,
                2 => PlayerPositionType::ForwardCenter,
                3 => PlayerPositionType::ForwardRight,
                _ => PlayerPositionType::AttackingMidfielderCenter,
            },
        };

        positions.push(PlayerPosition { position: primary, level: 20 });

        // DC and MC players automatically get formation-slot variants
        match primary {
            PlayerPositionType::DefenderCenter => {
                positions.push(PlayerPosition {
                    position: PlayerPositionType::DefenderCenterLeft,
                    level: IntegerUtils::random(17, 20) as u8,
                });
                positions.push(PlayerPosition {
                    position: PlayerPositionType::DefenderCenterRight,
                    level: IntegerUtils::random(17, 20) as u8,
                });
            }
            PlayerPositionType::MidfielderCenter => {
                positions.push(PlayerPosition {
                    position: PlayerPositionType::MidfielderCenterLeft,
                    level: IntegerUtils::random(17, 20) as u8,
                });
                positions.push(PlayerPosition {
                    position: PlayerPositionType::MidfielderCenterRight,
                    level: IntegerUtils::random(17, 20) as u8,
                });
            }
            _ => {}
        }

        // Each natural adjacent position has an independent 40% chance of being added
        let adjacent = natural_adjacent_positions(primary);
        for adj in &adjacent {
            if IntegerUtils::random(0, 99) < 40 {
                let level = IntegerUtils::random(14, 18) as u8;
                positions.push(PlayerPosition { position: *adj, level });
            }
        }

        // Cross-side versatility: ~15% chance for wide players to play opposite flank.
        // These players (e.g. M L/R, D L/R) are more versatile and valuable.
        if let Some(opposite) = cross_side_position(primary) {
            if IntegerUtils::random(0, 99) < 15 {
                if !positions.iter().any(|p| p.position == opposite) {
                    let level = IntegerUtils::random(12, 16) as u8;
                    positions.push(PlayerPosition { position: opposite, level });
                }
            }
        }

        // Higher PA → additional chance of a versatile position beyond adjacent
        let pa = potential_ability as i32;
        let versatility_pct = (pa * pa / 800).min(35); // PA 120→18%, PA 160→32%, PA 200→35%
        if IntegerUtils::random(0, 99) < versatility_pct {
            if let Some(extra) = pick_extra_position(primary) {
                if !positions.iter().any(|p| p.position == extra) {
                    let min_level = 10 + (potential_ability as i32 / 30).min(6);
                    let max_level = 14 + (potential_ability as i32 / 50).min(4);
                    positions.push(PlayerPosition {
                        position: extra,
                        level: IntegerUtils::random(min_level, max_level.max(min_level + 1)) as u8,
                    });
                }
            }
        }

        PlayerPositions { positions }
    }

    // ── Person attributes ───────────────────────────────────────────────

    fn youth_salary(current_ability: u8) -> u32 {
        match current_ability {
            0..=60 => 2_000,
            61..=80 => 5_000,
            81..=100 => 10_000,
            101..=120 => 20_000,
            121..=150 => 40_000,
            _ => 60_000,
        }
    }

    fn reserve_salary(current_ability: u8) -> u32 {
        match current_ability {
            0..=60 => 5_000,
            61..=80 => 12_000,
            81..=100 => 25_000,
            101..=120 => 50_000,
            121..=150 => 100_000,
            _ => 150_000,
        }
    }

    /// Smart initial contract duration based on age, ability, and reputation.
    /// Mirrors real football: young prospects get longer deals, aging players get shorter ones.
    fn generate_contract_years(age: u32, ability: u8, reputation: i16) -> i32 {
        let mut years: f32 = 3.0;

        // Age factor: young players get longer contracts, old players shorter
        match age {
            0..=19 => { years += 1.5; }   // youth: clubs lock in prospects
            20..=23 => { years += 1.0; }   // emerging: still investing
            24..=29 => { }                  // peak: standard deals
            30..=31 => { years -= 0.5; }   // early decline risk
            32..=33 => { years -= 1.0; }   // short deals
            _ => { years -= 1.5; }          // 34+: 1-2 year deals
        }

        // High-ability players get longer deals (club wants to lock them in)
        if ability > 150 {
            years += 1.0;
        } else if ability > 120 {
            years += 0.5;
        } else if ability < 60 {
            years -= 0.5; // low ability: club hedges with shorter deal
        }

        // High-reputation players get longer deals
        if reputation > 7000 {
            years += 1.0;
        } else if reputation > 4000 {
            years += 0.5;
        }

        // Add slight randomness (±0.5 year) to avoid all contracts expiring at once
        years += IntegerUtils::random(-1, 1) as f32 * 0.5;

        (years.round() as i32).clamp(1, 5)
    }

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
            consistency: FloatUtils::random(4.0f32, 18.0f32),
            important_matches: FloatUtils::random(4.0f32, 18.0f32),
            dirtiness: FloatUtils::random(0.0f32, 20.0f32),
        }
    }

    // ── Potential ability (generated before skills) ─────────────────────

    fn generate_potential_ability(rep_factor: f32, age: u32) -> u8 {
        // Three-tier PA distribution:
        //   Normal:   majority of squad — ability matches club level
        //   Standout: ~6-8% — notably better (every club has 1-3)
        //   Gem:      ~1-2% — exceptional talent well above club level
        //
        // Target per ~28-player squad:
        //   PL top (rep ~0.95): ~0.5 gems (5★), ~2-3 standouts (4★), normals 3-4★
        //   Juventus (rep ~0.89): ~0.4 gems, ~2 standouts (4★), normals mostly 4★
        //   Mid European (0.50): rare gem, ~1 standout, normals 2-3★
        //   Lower league (0.20): almost no gems, ~0.5 standout, normals 1-2★

        let roll = rand::random::<f32>();

        // Gem: very rare (1-2% at top clubs, <1% elsewhere)
        let gem_chance = (0.005 + rep_factor * rep_factor * 0.015).min(0.02);
        // Standout: every club has 1-3 above-average players
        let standout_chance = gem_chance + 0.05 + rep_factor * 0.04;

        if roll < gem_chance {
            // Gem: PA well above club range (5★ potential)
            let gem_min = (110.0 + rep_factor * 65.0) as i32;
            let gem_max = (140.0 + rep_factor * 55.0).min(195.0) as i32;
            IntegerUtils::random(gem_min, gem_max).min(200) as u8
        } else if roll < standout_chance {
            // Standout: clearly best players at the club (4-5★)
            let standout_base = 60.0 + rep_factor * 55.0 + rep_factor * rep_factor * 65.0;
            let noise = random_normal() * 10.0;
            let pa = standout_base + noise;
            pa.clamp(30.0, 190.0) as u8
        } else {
            // Normal: bulk of squad — top clubs should have mostly 3-4★ players
            let base = 25.0 + rep_factor * 60.0 + rep_factor * rep_factor * 80.0;
            let youth_bonus = if age <= 21 { 5.0 } else if age <= 25 { 2.0 } else { 0.0 };
            let noise = random_normal() * (5.0 + rep_factor * 10.0);
            let pa = base + youth_bonus + noise;
            pa.clamp(20.0, 185.0) as u8
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

        // PA must never be lower than CA — position-weighted skill calculation
        // can produce CA above the raw PA when skills align well with the position
        let potential_ability = potential_ability.max(current_ability);

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
            condition: IntegerUtils::random(6000, 9500) as i16,
            fitness: IntegerUtils::random(5000, 9500) as i16,
            jadedness: IntegerUtils::random(0, 3000) as i16,
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

    // ── ODB hydration ───────────────────────────────────────────────────
    //
    // Build a `Player` from an `OdbPlayer` record loaded from `players.odb`.
    // CA/PA from the record are authoritative; per-skill values are generated
    // through the same PA-anchored pipeline and then uniformly rescaled so the
    // resulting position-weighted CA matches the target.
    pub fn generate_from_odb(record: &OdbPlayer, continent_id: u32, country_code: &str) -> Player {
        let now = Utc::now().date_naive();
        let age = age_in_years(record.birth_date, now);

        let positions = positions_from_odb(&record.positions);
        let primary = positions
            .positions
            .first()
            .map(|p| p.position)
            .unwrap_or(PlayerPositionType::MidfielderCenter);
        let pos_type = position_type_from_player_position(primary);

        // Drive the skill generator off the recorded CA so the spread is
        // appropriate (cheap reuse of the existing pipeline).
        let rep_factor = (record.current_ability as f32 / 200.0).clamp(0.05, 1.0);
        let mut skills = Self::generate_skills(
            &pos_type,
            age,
            rep_factor,
            record.potential_ability,
            continent_id,
            country_code,
        );
        rescale_skills_to_target_ca(&mut skills, primary, record.current_ability);

        let full_name = build_full_name(record);
        let preferred_foot = parse_preferred_foot(record.preferred_foot.as_deref());

        let contract = build_main_contract(record);
        let contract_loan = build_loan_contract(record);

        let player_attributes = build_player_attributes(record, age, primary, &skills);

        let native_languages: Vec<core::PlayerLanguage> =
            core::Language::from_country_code(country_code)
                .into_iter()
                .map(core::PlayerLanguage::native)
                .collect();

        Player::builder()
            .id(record.id)
            .full_name(full_name)
            .birth_date(record.birth_date)
            .country_id(record.country_id)
            .skills(skills)
            .attributes(Self::generate_person_attributes())
            .player_attributes(player_attributes)
            .contract(contract)
            .contract_loan(contract_loan)
            .preferred_foot(preferred_foot)
            .positions(positions)
            .languages(native_languages)
            .build()
            .expect("Failed to build Player from ODB record")
    }
}

fn age_in_years(dob: NaiveDate, now: NaiveDate) -> u32 {
    let mut years = now.year() - dob.year();
    if (now.month(), now.day()) < (dob.month(), dob.day()) {
        years -= 1;
    }
    years.max(0) as u32
}

fn position_type_from_player_position(p: PlayerPositionType) -> PositionType {
    match p {
        PlayerPositionType::Goalkeeper => PositionType::Goalkeeper,
        PlayerPositionType::Sweeper
        | PlayerPositionType::DefenderLeft
        | PlayerPositionType::DefenderCenterLeft
        | PlayerPositionType::DefenderCenter
        | PlayerPositionType::DefenderCenterRight
        | PlayerPositionType::DefenderRight => PositionType::Defender,
        PlayerPositionType::DefensiveMidfielder
        | PlayerPositionType::MidfielderLeft
        | PlayerPositionType::MidfielderCenterLeft
        | PlayerPositionType::MidfielderCenter
        | PlayerPositionType::MidfielderCenterRight
        | PlayerPositionType::MidfielderRight
        | PlayerPositionType::WingbackLeft
        | PlayerPositionType::WingbackRight => PositionType::Midfielder,
        PlayerPositionType::AttackingMidfielderLeft
        | PlayerPositionType::AttackingMidfielderCenter
        | PlayerPositionType::AttackingMidfielderRight
        | PlayerPositionType::ForwardLeft
        | PlayerPositionType::ForwardCenter
        | PlayerPositionType::ForwardRight
        | PlayerPositionType::Striker => PositionType::Striker,
    }
}

fn parse_position_code(code: &str) -> Option<PlayerPositionType> {
    Some(match code.to_ascii_uppercase().as_str() {
        "GK" => PlayerPositionType::Goalkeeper,
        "SW" => PlayerPositionType::Sweeper,
        "DL" => PlayerPositionType::DefenderLeft,
        "DCL" => PlayerPositionType::DefenderCenterLeft,
        "DC" => PlayerPositionType::DefenderCenter,
        "DCR" => PlayerPositionType::DefenderCenterRight,
        "DR" => PlayerPositionType::DefenderRight,
        "DM" => PlayerPositionType::DefensiveMidfielder,
        "ML" => PlayerPositionType::MidfielderLeft,
        "MCL" => PlayerPositionType::MidfielderCenterLeft,
        "MC" => PlayerPositionType::MidfielderCenter,
        "MCR" => PlayerPositionType::MidfielderCenterRight,
        "MR" => PlayerPositionType::MidfielderRight,
        "AML" => PlayerPositionType::AttackingMidfielderLeft,
        "AMC" => PlayerPositionType::AttackingMidfielderCenter,
        "AMR" => PlayerPositionType::AttackingMidfielderRight,
        "WBL" => PlayerPositionType::WingbackLeft,
        "WBR" => PlayerPositionType::WingbackRight,
        "ST" => PlayerPositionType::Striker,
        "FL" => PlayerPositionType::ForwardLeft,
        "FC" => PlayerPositionType::ForwardCenter,
        "FR" => PlayerPositionType::ForwardRight,
        _ => return None,
    })
}

fn positions_from_odb(odb_positions: &[OdbPosition]) -> PlayerPositions {
    let mut positions: Vec<PlayerPosition> = odb_positions
        .iter()
        .filter_map(|p| {
            parse_position_code(&p.code).map(|pt| PlayerPosition {
                position: pt,
                level: p.level.clamp(1, 20),
            })
        })
        .collect();
    if positions.is_empty() {
        positions.push(PlayerPosition {
            position: PlayerPositionType::MidfielderCenter,
            level: 20,
        });
    }
    PlayerPositions { positions }
}

fn parse_preferred_foot(s: Option<&str>) -> PlayerPreferredFoot {
    match s.map(|v| v.to_ascii_lowercase()).as_deref() {
        Some("left") => PlayerPreferredFoot::Left,
        Some("both") | Some("either") => PlayerPreferredFoot::Both,
        _ => PlayerPreferredFoot::Right,
    }
}

fn parse_contract_type(s: Option<&str>) -> ContractType {
    match s.map(|v| v.to_ascii_lowercase()).as_deref() {
        Some("parttime") | Some("part-time") | Some("part_time") => ContractType::PartTime,
        Some("youth") => ContractType::Youth,
        Some("amateur") => ContractType::Amateur,
        Some("noncontract") | Some("non-contract") | Some("non_contract") => {
            ContractType::NonContract
        }
        Some("loan") => ContractType::Loan,
        _ => ContractType::FullTime,
    }
}

fn build_full_name(record: &OdbPlayer) -> FullName {
    let first = record.first_name.clone();
    let last = record.last_name.clone();
    match (record.middle_name.as_ref(), record.nickname.as_ref()) {
        (None, Some(nick)) => FullName::with_nickname(first, last, nick.clone()),
        _ => FullName::new(first, last),
    }
}

fn build_main_contract(record: &OdbPlayer) -> Option<PlayerClubContract> {
    let mut c = PlayerClubContract {
        shirt_number: record.contract.shirt_number,
        salary: record.contract.salary,
        contract_type: parse_contract_type(record.contract.contract_type.as_deref()),
        squad_status: core::PlayerSquadStatus::NotYetSet,
        is_transfer_listed: false,
        transfer_status: None,
        started: record.contract.started,
        expiration: record.contract.expiration,
        loan_from_club_id: None,
        loan_from_team_id: None,
        loan_to_club_id: None,
        loan_match_fee: None,
        loan_wage_contribution_pct: None,
        loan_future_fee: None,
        loan_future_fee_obligation: false,
        loan_recall_available_after: None,
        loan_min_appearances: None,
        bonuses: vec![],
        clauses: vec![],
    };
    // If currently loaned out, the main contract retains parent terms but
    // records the borrower via loan_to_club_id so the value/wage code knows.
    if let Some(ref loan) = record.loan {
        c.loan_to_club_id = Some(loan.to_club_id);
    }
    Some(c)
}

fn build_loan_contract(record: &OdbPlayer) -> Option<PlayerClubContract> {
    let loan = record.loan.as_ref()?;
    Some(PlayerClubContract {
        shirt_number: None,
        salary: loan.salary,
        contract_type: ContractType::Loan,
        squad_status: core::PlayerSquadStatus::NotYetSet,
        is_transfer_listed: false,
        transfer_status: None,
        started: None,
        expiration: loan.expiration,
        loan_from_club_id: Some(record.club_id),
        loan_from_team_id: None,
        loan_to_club_id: Some(loan.to_club_id),
        loan_match_fee: loan.match_fee,
        loan_wage_contribution_pct: loan.wage_contribution_pct,
        loan_future_fee: loan.future_fee,
        loan_future_fee_obligation: loan.future_fee_obligation,
        loan_recall_available_after: None,
        loan_min_appearances: loan.min_appearances,
        bonuses: vec![],
        clauses: vec![],
    })
}

/// Derive (current, home, world) reputation from current ability alone.
///
/// Ability is normalised to a 0..1 coefficient and shaped by a different
/// curve for each reputation dimension, mirroring how fame actually
/// distributes in football:
///
/// - **home** — mildly concave (`coef^0.9`). Known domestically scales
///   almost linearly with ability, but bends upward a touch at the top
///   so superstars cross the World Class label cleanly. The earlier
///   `coef^0.6` pushed a CA-100 second-division keeper over the
///   Continental threshold, which is not how anyone at that level would
///   describe himself.
/// - **current** — near-linear (`coef^1.0`). Match-to-match standing
///   tracks raw playing ability closely.
/// - **world** — steeply convex (`coef^2.5`). International recognition
///   is reserved for the very top of the ability distribution — a
///   lower-division pro (CA ~60) barely registers at all, while a
///   Ballon d'Or candidate dominates. A mid-tier CA-100 regular sits
///   around 1700, not the 2700 the softer curve produced.
///
/// The old flat `CA * 45 * fixed_mult` formula over-rewarded mid-tier
/// players on world fame and under-rewarded elite players — you ended up
/// with mathematically-similar numbers for a Ballon d'Or candidate and a
/// solid top-flight regular, which doesn't match how transfer markets
/// or contract talks should read those players.
fn derive_reputation_from_ability(ca: u8) -> (i16, i16, i16) {
    const REP_CEILING: f32 = 9500.0;
    let coef = (ca as f32 / 200.0).clamp(0.05, 1.0);
    let current = (REP_CEILING * coef.powf(1.0) * 0.95) as i16;
    let home = (REP_CEILING * coef.powf(0.9)) as i16;
    let world = (REP_CEILING * coef.powf(2.5)) as i16;
    (current, home, world)
}

fn build_player_attributes(
    record: &OdbPlayer,
    age: u32,
    primary: PlayerPositionType,
    skills: &PlayerSkills,
) -> PlayerAttributes {
    // Reputation: any record-supplied value wins for its own field;
    // every missing field falls back to an ability-curve derivation so
    // partial overrides (e.g. a scraper that only captured world fame)
    // still produce coherent home/current numbers.
    let (derived_current, derived_home, derived_world) =
        derive_reputation_from_ability(record.current_ability);
    let (current_rep, home_rep, world_rep) = match record.reputation.as_ref() {
        Some(r) => (
            r.current.unwrap_or(derived_current),
            r.home.unwrap_or(derived_home),
            r.world.unwrap_or(derived_world),
        ),
        None => (derived_current, derived_home, derived_world),
    };

    // Scaled CA can drift a couple of points off target after rescaling;
    // pull the recorded value back in so downstream code (squad status,
    // value calc) sees what the ODB intended.
    let derived_ca = skills.calculate_ability_for_position(primary);
    let current_ability = if (derived_ca as i32 - record.current_ability as i32).abs() <= 6 {
        record.current_ability
    } else {
        derived_ca
    };
    let potential_ability = record.potential_ability.max(current_ability);

    PlayerAttributes {
        is_banned: false,
        is_injured: false,
        condition: IntegerUtils::random(7000, 9500) as i16,
        fitness: IntegerUtils::random(6000, 9500) as i16,
        jadedness: 0,
        weight: record
            .weight
            .unwrap_or_else(|| IntegerUtils::random(65, 90) as u8),
        height: record
            .height
            .unwrap_or_else(|| default_height_for_position(primary)),
        value: record.value.unwrap_or(0),
        current_reputation: current_rep,
        home_reputation: home_rep,
        world_reputation: world_rep,
        current_ability,
        potential_ability,
        international_apps: 0,
        international_goals: 0,
        under_21_international_apps: 0,
        under_21_international_goals: if age <= 23 { 0 } else { 0 },
        injury_days_remaining: 0,
        injury_type: None,
        injury_proneness: (IntegerUtils::random(1, 10) + IntegerUtils::random(1, 10)) as u8,
        recovery_days_remaining: 0,
        last_injury_body_part: 0,
        injury_count: 0,
        days_since_last_match: 0,
    }
}

fn default_height_for_position(primary: PlayerPositionType) -> u8 {
    match primary {
        PlayerPositionType::Goalkeeper => 188,
        PlayerPositionType::DefenderCenter
        | PlayerPositionType::DefenderCenterLeft
        | PlayerPositionType::DefenderCenterRight => 186,
        PlayerPositionType::Striker | PlayerPositionType::ForwardCenter => 183,
        _ => 180,
    }
}

/// Uniformly scale all 1-20 skills toward a target CA. One pass is sufficient
/// because `calculate_ability_for_position` is roughly linear in the skill
/// average for non-extreme inputs.
fn rescale_skills_to_target_ca(
    skills: &mut PlayerSkills,
    primary: PlayerPositionType,
    target_ca: u8,
) {
    if target_ca == 0 {
        return;
    }
    let current = skills.calculate_ability_for_position(primary).max(1);
    let factor = (target_ca as f32 / current as f32).clamp(0.40, 2.50);
    if (factor - 1.0).abs() < 0.02 {
        return;
    }
    scale_in_place(skills, factor);
    // Second pass to tighten.
    let after = skills.calculate_ability_for_position(primary).max(1);
    let f2 = (target_ca as f32 / after as f32).clamp(0.85, 1.15);
    if (f2 - 1.0).abs() > 0.01 {
        scale_in_place(skills, f2);
    }
}

fn scale_in_place(skills: &mut PlayerSkills, factor: f32) {
    macro_rules! s {
        ($v:expr) => {
            $v = ($v * factor).clamp(1.0, 20.0)
        };
    }
    let t = &mut skills.technical;
    s!(t.corners); s!(t.crossing); s!(t.dribbling); s!(t.finishing);
    s!(t.first_touch); s!(t.free_kicks); s!(t.heading); s!(t.long_shots);
    s!(t.long_throws); s!(t.marking); s!(t.passing); s!(t.penalty_taking);
    s!(t.tackling); s!(t.technique);
    let m = &mut skills.mental;
    s!(m.aggression); s!(m.anticipation); s!(m.bravery); s!(m.composure);
    s!(m.concentration); s!(m.decisions); s!(m.determination); s!(m.flair);
    s!(m.leadership); s!(m.off_the_ball); s!(m.positioning); s!(m.teamwork);
    s!(m.vision); s!(m.work_rate);
    let p = &mut skills.physical;
    s!(p.acceleration); s!(p.agility); s!(p.balance); s!(p.jumping);
    s!(p.natural_fitness); s!(p.pace); s!(p.stamina); s!(p.strength);
    let g = &mut skills.goalkeeping;
    s!(g.aerial_reach); s!(g.command_of_area); s!(g.communication);
    s!(g.eccentricity); s!(g.first_touch); s!(g.handling); s!(g.kicking);
    s!(g.one_on_ones); s!(g.passing); s!(g.punching); s!(g.reflexes);
    s!(g.rushing_out); s!(g.throwing);
}

/// Natural adjacent positions that most players at a given position can also play.
/// E.g., a DC can play DCL/DCR, a DL can play WBL, an MC can play MCL/MCR.
fn natural_adjacent_positions(primary: PlayerPositionType) -> Vec<PlayerPositionType> {
    match primary {
        PlayerPositionType::Goalkeeper => vec![],
        // DC: DCL/DCR are auto-added as formation slots; adjacent is DM
        PlayerPositionType::DefenderCenter => vec![PlayerPositionType::DefensiveMidfielder],
        // DCL/DCR kept for compatibility (if they appear through other means)
        PlayerPositionType::DefenderCenterLeft => vec![PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderLeft],
        PlayerPositionType::DefenderCenterRight => vec![PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderRight],
        // Full-backs: DL ↔ WBL, DR ↔ WBR
        PlayerPositionType::DefenderLeft => vec![PlayerPositionType::WingbackLeft],
        PlayerPositionType::DefenderRight => vec![PlayerPositionType::WingbackRight],
        // MC: MCL/MCR are auto-added as formation slots; adjacent is DM or AMC
        PlayerPositionType::MidfielderCenter => {
            if IntegerUtils::random(0, 1) == 0 {
                vec![PlayerPositionType::DefensiveMidfielder]
            } else {
                vec![PlayerPositionType::AttackingMidfielderCenter]
            }
        }
        // MCL/MCR kept for compatibility
        PlayerPositionType::MidfielderCenterLeft => vec![PlayerPositionType::MidfielderCenter, PlayerPositionType::MidfielderLeft],
        PlayerPositionType::MidfielderCenterRight => vec![PlayerPositionType::MidfielderCenter, PlayerPositionType::MidfielderRight],
        // Wide midfielders: ML ↔ AML, MR ↔ AMR
        PlayerPositionType::MidfielderLeft => vec![PlayerPositionType::AttackingMidfielderLeft],
        PlayerPositionType::MidfielderRight => vec![PlayerPositionType::AttackingMidfielderRight],
        // Wingbacks: WBL ↔ DL, WBR ↔ DR
        PlayerPositionType::WingbackLeft => vec![PlayerPositionType::DefenderLeft],
        PlayerPositionType::WingbackRight => vec![PlayerPositionType::DefenderRight],
        // DM ↔ MC
        PlayerPositionType::DefensiveMidfielder => vec![PlayerPositionType::MidfielderCenter],
        // Attacking midfielders: AMC ↔ MC, AML ↔ ML, AMR ↔ MR
        PlayerPositionType::AttackingMidfielderCenter => vec![PlayerPositionType::MidfielderCenter],
        PlayerPositionType::AttackingMidfielderLeft => vec![PlayerPositionType::MidfielderLeft],
        PlayerPositionType::AttackingMidfielderRight => vec![PlayerPositionType::MidfielderRight],
        // Forwards: ST ↔ FC, FL ↔ AML, FR ↔ AMR
        PlayerPositionType::Striker => vec![PlayerPositionType::ForwardCenter],
        PlayerPositionType::ForwardCenter => vec![PlayerPositionType::Striker],
        PlayerPositionType::ForwardLeft => vec![PlayerPositionType::AttackingMidfielderLeft],
        PlayerPositionType::ForwardRight => vec![PlayerPositionType::AttackingMidfielderRight],
        _ => vec![],
    }
}

/// Extra position for versatile players (beyond natural adjacent).
fn pick_extra_position(primary: PlayerPositionType) -> Option<PlayerPositionType> {
    match primary {
        PlayerPositionType::Goalkeeper => None,
        PlayerPositionType::DefenderCenter => Some(PlayerPositionType::Sweeper),
        PlayerPositionType::DefenderCenterLeft => Some(PlayerPositionType::DefenderCenterRight),
        PlayerPositionType::DefenderCenterRight => Some(PlayerPositionType::DefenderCenterLeft),
        PlayerPositionType::DefenderLeft => Some(PlayerPositionType::MidfielderLeft),
        PlayerPositionType::DefenderRight => Some(PlayerPositionType::MidfielderRight),
        PlayerPositionType::DefensiveMidfielder => Some(PlayerPositionType::DefenderCenter),
        PlayerPositionType::MidfielderCenter => Some(if IntegerUtils::random(0, 1) == 0 {
            PlayerPositionType::DefensiveMidfielder
        } else {
            PlayerPositionType::AttackingMidfielderCenter
        }),
        PlayerPositionType::MidfielderLeft => Some(PlayerPositionType::WingbackLeft),
        PlayerPositionType::MidfielderRight => Some(PlayerPositionType::WingbackRight),
        PlayerPositionType::WingbackLeft => Some(PlayerPositionType::MidfielderLeft),
        PlayerPositionType::WingbackRight => Some(PlayerPositionType::MidfielderRight),
        PlayerPositionType::AttackingMidfielderLeft => Some(PlayerPositionType::ForwardLeft),
        PlayerPositionType::AttackingMidfielderCenter => Some(PlayerPositionType::Striker),
        PlayerPositionType::AttackingMidfielderRight => Some(PlayerPositionType::ForwardRight),
        PlayerPositionType::Striker => Some(PlayerPositionType::AttackingMidfielderCenter),
        PlayerPositionType::ForwardLeft => Some(PlayerPositionType::ForwardCenter),
        PlayerPositionType::ForwardCenter => Some(PlayerPositionType::AttackingMidfielderCenter),
        PlayerPositionType::ForwardRight => Some(PlayerPositionType::ForwardCenter),
        _ => None,
    }
}

/// Opposite-side position for cross-side versatility.
/// Players who can play both flanks (e.g. M L/R) are more valuable.
fn cross_side_position(primary: PlayerPositionType) -> Option<PlayerPositionType> {
    match primary {
        PlayerPositionType::DefenderLeft => Some(PlayerPositionType::DefenderRight),
        PlayerPositionType::DefenderRight => Some(PlayerPositionType::DefenderLeft),
        PlayerPositionType::MidfielderLeft => Some(PlayerPositionType::MidfielderRight),
        PlayerPositionType::MidfielderRight => Some(PlayerPositionType::MidfielderLeft),
        PlayerPositionType::WingbackLeft => Some(PlayerPositionType::WingbackRight),
        PlayerPositionType::WingbackRight => Some(PlayerPositionType::WingbackLeft),
        PlayerPositionType::AttackingMidfielderLeft => Some(PlayerPositionType::AttackingMidfielderRight),
        PlayerPositionType::AttackingMidfielderRight => Some(PlayerPositionType::AttackingMidfielderLeft),
        PlayerPositionType::ForwardLeft => Some(PlayerPositionType::ForwardRight),
        PlayerPositionType::ForwardRight => Some(PlayerPositionType::ForwardLeft),
        _ => None,
    }
}

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn test_versatility_by_pa() {
        let total = 500;
        for pa in [30u8, 80, 140] {
            let multi = (0..total)
                .filter(|_| PlayerGenerator::generate_positions(PositionType::Midfielder, pa).positions.len() > 1)
                .count();
            let pct = multi * 100 / total;
            eprintln!("PA={pa}: {multi}/{total} = {pct}%");
            // PA 30 → ~5%, PA 80 → ~15%, PA 140 → ~38%
            assert!(multi > 5, "PA={pa}: only {multi}/{total} multi-pos");
        }
    }
}
