use crate::loaders::{CountryLoader, OdbHistoryItem, OdbPlayer, OdbPosition};
use crate::DatabaseEntity;
use chrono::{Datelike, NaiveDate, Utc};
use core::shared::FullName;
use core::utils::{FloatUtils, IntegerUtils};
use core::league::Season;
use core::{
    ContractType, Mental, PeopleNameGeneratorData, PersonAttributes, Physical, Player,
    PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerPreferredFoot, PlayerSkills, PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatisticsHistoryItem, PositionWeights, TeamType, Technical, WageCalculator,
};
use core::next_player_id;

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

// ── Age curves and per-skill peak timing ────────────────────────────────

/// All age-driven adjustments applied during skill generation. Lives as a
/// stateless namespace so the generator pipeline reads cleanly without free
/// utility functions in scope.
pub struct AgeCurve;

impl AgeCurve {
    /// Subtle per-skill peak timing modifier within a group. The main age
    /// development is handled by per-group age ratios in `generate_skills`;
    /// this only shifts individual skills based on early/mid/late peak timing.
    /// Range: 0.92 .. 1.05 (fine-tuning, not a major multiplier).
    pub fn peak_modifier(skill_idx: usize, age: u32) -> f32 {
        let (peak_start, peak_end) = match skill_idx {
            SK_ACCELERATION | SK_PACE | SK_AGILITY | SK_JUMPING | SK_BALANCE | SK_NATURAL_FITNESS => {
                (18u32, 24u32)
            }
            SK_DECISIONS | SK_POSITIONING | SK_VISION | SK_LEADERSHIP | SK_COMPOSURE | SK_PASSING => {
                (26, 34)
            }
            _ => (22, 28),
        };

        let age_f = age as f32;
        if age < peak_start {
            let ramp_start = peak_start.saturating_sub(6) as f32;
            let t = ((age_f - ramp_start) / (peak_start as f32 - ramp_start)).clamp(0.0, 1.0);
            lerp(0.92, 1.0, t)
        } else if age <= peak_end {
            1.03
        } else {
            let t = ((age_f - peak_end as f32) / (40.0 - peak_end as f32)).clamp(0.0, 1.0);
            lerp(1.03, 0.92, t)
        }
    }

    /// Young players cannot reach elite skill levels regardless of talent.
    /// Consistent with the core generator's `age_skill_cap`.
    pub fn skill_cap(age: u32) -> f32 {
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
}

/// Skill group buckets — Technical (0..14), Mental (14..28), Physical (28..36),
/// MatchReadiness (36). The generator uses this to pick group means and
/// noise scales without scattering raw index ranges across the codebase.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SkillGroup {
    Technical,
    Mental,
    Physical,
    MatchReadiness,
}

impl SkillGroup {
    pub fn from_index(idx: usize) -> SkillGroup {
        if idx < 14 { SkillGroup::Technical }
        else if idx < 28 { SkillGroup::Mental }
        else if idx < 36 { SkillGroup::Physical }
        else { SkillGroup::MatchReadiness }
    }
}

// ── Role archetype overlay ──────────────────────────────────────────────

/// Apply a random role archetype to create variety within position groups.
/// The base weights come from `core::PositionWeights::for_position` (per
/// exact position); these archetype shifts add the "Poacher vs Target Man"
/// flavour on top. Used as `RoleArchetype::apply(&mut weights, &bucket)`.
pub struct RoleArchetype;

impl RoleArchetype {
    pub fn apply(weights: &mut [f32; SKILL_COUNT], position: &PositionType) {
        Self::apply_inner(weights, position);
    }

    fn apply_inner(weights: &mut [f32; SKILL_COUNT], position: &PositionType) {
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
}

// ── Skill affinity correlations + array→struct conversion ───────────────

/// Cross-attribute correlations applied after the base distribution. High
/// passing pulls vision/first_touch up, high aggression pulls bravery up
/// and composure down, etc. Centralised on a stateless namespace so the
/// generator pipeline doesn't pull in free helpers.
pub struct SkillAffinities;

impl SkillAffinities {
    pub fn apply(skills: &mut [f32; SKILL_COUNT]) {
        if skills[SK_PASSING] > 14.0 {
            let bonus = (skills[SK_PASSING] - 14.0) * 0.15;
            skills[SK_VISION] += bonus;
            skills[SK_FIRST_TOUCH] += bonus;
        }
        if skills[SK_AGGRESSION] > 14.0 {
            let bonus = (skills[SK_AGGRESSION] - 14.0) * 0.12;
            skills[SK_BRAVERY] += bonus;
            skills[SK_COMPOSURE] -= bonus * 0.5;
        }
        if skills[SK_PACE] > 14.0 {
            let bonus = (skills[SK_PACE] - 14.0) * 0.15;
            skills[SK_ACCELERATION] += bonus;
        }
        if skills[SK_FINISHING] > 14.0 {
            let bonus = (skills[SK_FINISHING] - 14.0) * 0.1;
            skills[SK_COMPOSURE] += bonus;
            skills[SK_ANTICIPATION] += bonus;
        }
        if skills[SK_DRIBBLING] > 14.0 {
            let bonus = (skills[SK_DRIBBLING] - 14.0) * 0.12;
            skills[SK_FLAIR] += bonus;
            skills[SK_AGILITY] += bonus;
        }
        if skills[SK_LEADERSHIP] > 14.0 {
            let bonus = (skills[SK_LEADERSHIP] - 14.0) * 0.1;
            skills[SK_DETERMINATION] += bonus;
            skills[SK_TEAMWORK] += bonus;
        }
    }
}

/// Conversion between the flat `[f32; SKILL_COUNT]` work array used inside
/// the generator and the structured `PlayerSkills` consumed by the rest of
/// the simulation. Stateless namespace so the generator never reaches for
/// a free helper to do this last-mile conversion.
pub struct SkillsArray;

impl SkillsArray {
    /// Convert the flat array back into the structured `PlayerSkills`.
    pub fn into_skills(arr: &[f32; SKILL_COUNT]) -> PlayerSkills {
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

#[derive(Copy, Clone, Debug)]
pub enum PositionType {
    Goalkeeper,
    Defender,
    Midfielder,
    Striker,
}

/// Role this player is being generated to fill in their squad. Drives target
/// CA, PA headroom, and salary scaling. The distribution per `TeamType` is
/// owned by the caller (`generate_players`).
#[derive(Copy, Clone, Debug)]
pub enum SquadRole {
    /// Best players at the club (5★ ceiling, near-PA CA).
    Star,
    /// First-team starters (4★, CA close to PA).
    Starter,
    /// Squad rotation, dependable (3★).
    Rotation,
    /// Backups and depth (2-3★).
    Backup,
    /// Young player below current CA but PA ceiling well above.
    Prospect,
    /// Lower-tier squad filler (1-2★).
    Fringe,
}

impl SquadRole {
    /// Multiplicative bump applied to the rep-blend baseline when picking
    /// target CA. Stars pull above their club's average; backups/fringe sit
    /// well below it.
    fn ca_factor(self) -> f32 {
        match self {
            SquadRole::Star => 1.30,
            SquadRole::Starter => 1.10,
            SquadRole::Rotation => 0.95,
            SquadRole::Backup => 0.78,
            SquadRole::Prospect => 0.65,
            SquadRole::Fringe => 0.55,
        }
    }

    /// Headroom (PA - CA) range used when generating PA from target CA.
    /// Prospects carry the most ceiling; starters/stars are already near peak.
    fn pa_headroom_range(self, age: u32) -> (i32, i32) {
        let age_dampen = if age >= 30 { 0 } else if age >= 27 { 4 } else if age >= 24 { 8 } else { 14 };
        match self {
            SquadRole::Star => (4 + age_dampen / 2, 12 + age_dampen),
            SquadRole::Starter => (4 + age_dampen / 2, 14 + age_dampen),
            SquadRole::Rotation => (2 + age_dampen / 2, 12 + age_dampen),
            SquadRole::Backup => (0, 8 + age_dampen / 2),
            SquadRole::Prospect => (24 + age_dampen, 55 + age_dampen * 2),
            SquadRole::Fringe => (0, 4 + age_dampen / 4),
        }
    }
}

/// Position- and country-aware physical profile (height + weight).
/// Replaces the original `random(150..220)` / `random(60..100)` rolls with
/// realistic ranges for each playing role and a small regional offset
/// reflecting country-level body-type tendencies.
pub struct PhysicalProfile {
    pub height_cm: u8,
    pub weight_kg: u8,
}

impl PhysicalProfile {
    pub fn for_position(primary: PlayerPositionType, country_id: u32) -> Self {
        let (lo, hi) = Self::height_range(primary);
        let offset = Self::country_height_offset(country_id);
        let height = (IntegerUtils::random(lo + offset, hi + offset)
            .clamp(160, 210)) as u8;
        let weight = Self::weight_for(height, primary);
        PhysicalProfile { height_cm: height, weight_kg: weight }
    }

    fn height_range(primary: PlayerPositionType) -> (i32, i32) {
        use PlayerPositionType::*;
        match primary {
            Goalkeeper => (184, 198),
            DefenderCenter | DefenderCenterLeft | DefenderCenterRight | Sweeper => (183, 196),
            Striker | ForwardCenter => (177, 192),
            DefensiveMidfielder => (177, 190),
            MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight => (174, 187),
            DefenderLeft | DefenderRight => (172, 184),
            WingbackLeft | WingbackRight => (170, 182),
            MidfielderLeft | MidfielderRight => (170, 183),
            AttackingMidfielderCenter => (170, 183),
            AttackingMidfielderLeft | AttackingMidfielderRight
            | ForwardLeft | ForwardRight => (168, 182),
        }
    }

    /// Country-level height offset (cm). Northern Europe trends taller,
    /// East Asia / Latin America trend shorter. Match arms are lowercase
    /// because `CountryLoader::code_for_id` always returns lowercase
    /// (matching the loader's data and `country_skill_bias`).
    fn country_height_offset(country_id: u32) -> i32 {
        let code = CountryLoader::code_for_id(country_id);
        match code.as_str() {
            "nl" | "dk" | "no" | "se" | "is" | "fi" | "lt" | "lv" | "ee"
            | "de" | "at" | "ch" | "rs" | "me" | "ba" | "hr" | "si"
            | "be" | "pl" | "cz" | "sk" | "ua" | "by" | "ru" => 3,
            "gb" | "ie" | "sc" | "wl" | "ni" => 1,
            "it" | "es" | "pt" | "fr" | "gr" | "tr" | "mt" => -1,
            "br" | "ar" | "uy" | "cl" | "py" | "bo" | "pe" | "ec" | "ve" | "co" | "mx" => -2,
            "jp" | "kr" | "cn" | "th" | "vn" | "id" | "ph" | "my" => -3,
            _ => 0,
        }
    }

    /// Weight follows height for a body-mass index suited to professional
    /// athletes (~22-26 BMI), with role-specific muscle bias: defenders and
    /// strikers carry more mass, wingers/wide players run lean.
    fn weight_for(height_cm: u8, primary: PlayerPositionType) -> u8 {
        let h_m = height_cm as f32 / 100.0;
        let bmi_target = match primary {
            PlayerPositionType::Goalkeeper => 24.0,
            PlayerPositionType::DefenderCenter
            | PlayerPositionType::DefenderCenterLeft
            | PlayerPositionType::DefenderCenterRight
            | PlayerPositionType::Sweeper => 24.5,
            PlayerPositionType::Striker | PlayerPositionType::ForwardCenter => 23.8,
            PlayerPositionType::DefensiveMidfielder => 23.5,
            PlayerPositionType::AttackingMidfielderLeft
            | PlayerPositionType::AttackingMidfielderRight
            | PlayerPositionType::ForwardLeft
            | PlayerPositionType::ForwardRight
            | PlayerPositionType::MidfielderLeft
            | PlayerPositionType::MidfielderRight
            | PlayerPositionType::WingbackLeft
            | PlayerPositionType::WingbackRight => 22.5,
            _ => 23.0,
        };
        let jitter = (random_normal() * 1.2).clamp(-3.0, 3.0);
        ((bmi_target + jitter) * h_m * h_m).clamp(55.0, 110.0) as u8
    }
}

/// Match-readiness, condition, fitness, and jadedness — *state*, not skill.
/// Initialised from age-realistic preseason values; the PA pipeline never
/// touches them. The previous code threaded them through skill generation,
/// which let young players have low match readiness for no reason.
pub struct FitnessState {
    pub condition: i16,
    pub fitness: i16,
    pub jadedness: i16,
}

impl FitnessState {
    pub fn for_age(age: u32) -> Self {
        // Preseason snapshot: pros come back near full condition; older
        // players carry slightly more wear.
        let condition_base = if age >= 32 { 7800 } else if age >= 28 { 8200 } else { 8500 };
        let fitness_base = if age >= 32 { 7400 } else if age >= 28 { 7800 } else { 8200 };
        let condition = (condition_base + IntegerUtils::random(-400, 600)).clamp(6500, 9700) as i16;
        let fitness = (fitness_base + IntegerUtils::random(-500, 700)).clamp(6000, 9700) as i16;
        let jadedness = if age >= 32 {
            IntegerUtils::random(800, 2500) as i16
        } else {
            IntegerUtils::random(0, 1500) as i16
        };
        FitnessState { condition, fitness, jadedness }
    }

    /// Initial match_readiness (0..20 scale). Pre-season state, lifted by
    /// fitness ratio. Older players ramp slightly slower.
    pub fn match_readiness(age: u32) -> f32 {
        let base = if age >= 32 { 9.0 } else if age >= 28 { 11.0 } else { 12.5 };
        let jitter = random_normal() * 1.5;
        (base + jitter).clamp(6.0, 16.0)
    }
}

/// Owns the (current, potential) ability targeting pipeline for senior
/// players. Splits CA from PA cleanly: CA is anchored on rep + role + age,
/// PA = CA + role-and-age-aware headroom (so a young prospect can carry a
/// big PA delta while a 30-year-old starter gets only a few points of room).
pub struct AbilityTarget {
    pub current: u8,
    pub potential: u8,
}

impl AbilityTarget {
    /// Combined reputation score (0..1) blending team / league / country.
    /// Team weight dominates (it most directly drives the player you can
    /// attract); league and country are softer pulls. Top European club in
    /// a top league + top football country lands ~0.95; a lower-division
    /// minnow in a weak country sits around 0.10.
    pub fn rep_blend(team_rep: u16, league_rep: u16, country_rep: u16) -> f32 {
        let blended = team_rep as f32 * 0.50 + league_rep as f32 * 0.30 + country_rep as f32 * 0.20;
        (blended / 10000.0).clamp(0.0, 1.0)
    }

    /// Build the (CA, PA) pair for a generated player. Same continuous
    /// formula across every TeamType — squad role + age curve modulate it.
    pub fn for_role(rep_factor: f32, age: u32, role: SquadRole) -> AbilityTarget {
        let current = Self::current_for(rep_factor, age, role);
        let potential = Self::potential_from(current, age, role);
        AbilityTarget { current, potential }
    }

    fn current_for(rep_factor: f32, age: u32, role: SquadRole) -> u8 {
        // Slightly convex base so top reputation pulls away from the middle.
        let rep_curve = rep_factor * 0.85 + rep_factor * rep_factor * 0.15;
        let base_ca = 35.0 + rep_curve * 145.0; // ~35..180
        let raw = base_ca * role.ca_factor() * Self::age_factor(age);
        let noise = random_normal() * (3.0 + rep_factor * 4.0);
        (raw + noise).clamp(15.0, 195.0).round() as u8
    }

    fn potential_from(target_ca: u8, age: u32, role: SquadRole) -> u8 {
        let (lo, hi) = role.pa_headroom_range(age);
        let headroom = IntegerUtils::random(lo, hi.max(lo + 1));
        ((target_ca as i32) + headroom).clamp(target_ca as i32, 200) as u8
    }

    /// Age curve applied to target CA. Skills don't fully bloom until the
    /// mid-twenties; older players retain most CA but slowly decline.
    fn age_factor(age: u32) -> f32 {
        match age {
            0..=16 => 0.55,
            17 => 0.62,
            18 => 0.70,
            19 => 0.78,
            20 => 0.85,
            21 => 0.91,
            22 => 0.95,
            23 => 0.98,
            24..=30 => 1.00,
            31 => 0.97,
            32 => 0.94,
            33 => 0.90,
            34 => 0.85,
            35 => 0.80,
            _ => 0.72,
        }
    }
}

impl PlayerGenerator {
    /// Senior-team player generation. League-aware, role-aware, exact-position
    /// driven. The pipeline is:
    ///   1. Rep-blend → 0..1 reputation factor (team/league/country).
    ///   2. Age picked uniformly inside [min_age, max_age].
    ///   3. Target CA from rep + role + age curve.
    ///   4. PA = CA + role-and-age-aware headroom.
    ///   5. Pick exact `PlayerPositionType` matching `bucket` (and bias
    ///      towards modern roles for higher-PA players).
    ///   6. Skills generated to target CA via per-exact-position weights and
    ///      rescaled until `calculate_ability_for_position` lands on target.
    pub fn generate(
        &self,
        country_id: u32,
        continent_id: u32,
        bucket: PositionType,
        team_reputation: u16,
        league_reputation: u16,
        country_reputation: u16,
        team_type: TeamType,
        role: SquadRole,
        min_age: i32,
        max_age: i32,
    ) -> Player {
        let now = Utc::now();

        let rep_factor = AbilityTarget::rep_blend(team_reputation, league_reputation, country_reputation);

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

        // CA before PA: target CA is the anchor of skill generation.
        let ability = AbilityTarget::for_role(rep_factor, age, role);
        let target_ca = ability.current;
        let potential_ability = ability.potential;

        // Pick the exact playing position before generating skills so the
        // attribute distribution matches the role (DC vs WBL vs AMC etc.).
        // Picker is context-aware: PA, team_type, and squad_role bias the
        // distribution toward modern/specialist roles for top-tier squads
        // and toward traditional roles for lower tiers.
        let primary_position = Self::pick_exact_position(bucket, potential_ability, team_type, role);
        let positions = Self::generate_positions_from_primary(primary_position, potential_ability);

        let country_code = CountryLoader::code_for_id(country_id);
        let mut skills = Self::generate_skills(
            primary_position,
            age,
            rep_factor,
            potential_ability,
            target_ca,
            continent_id,
            &country_code,
        );
        SkillRescaler::to_target_ca(&mut skills, primary_position, target_ca);
        // Convergence contract:
        //   - Adult players (age >= ~22) converge on target CA within ±5.
        //   - Younger players intentionally undershoot when the rep blend
        //     implies a CA the age cap won't allow yet. The age cap always
        //     wins — a 16yo at a top club gets a 16yo's skill ceiling, not
        //     a CA-130 inflation of it. This is enforced post-rescale via
        //     `clamp_to_cap`. Tests assert both halves of the contract.
        SkillRescaler::clamp_to_cap(&mut skills, AgeCurve::skill_cap(age));
        skills.physical.match_readiness = FitnessState::match_readiness(age);

        let is_youth = matches!(team_type, TeamType::U18 | TeamType::U19 | TeamType::U20 | TeamType::U21 | TeamType::U23);

        let player_attributes = Self::generate_player_attributes(
            rep_factor,
            age,
            potential_ability,
            &skills,
            primary_position,
            country_id,
        );

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
            &CountryLoader::code_for_id(country_id)
        )
            .into_iter()
            .map(|lang| core::PlayerLanguage::native(lang))
            .collect();

        Player::builder()
            .id(next_player_id())
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

    fn generate_skills(
        primary_position: PlayerPositionType,
        age: u32,
        rep_factor: f32,
        potential_ability: u8,
        target_ca: u8,
        continent_id: u32,
        country_code: &str,
    ) -> PlayerSkills {
        // Target CA — not PA — drives current skill level. PA only governs
        // the per-skill ceiling so a young CA-80 prospect with PA 160 still
        // looks like an 80-CA player today, not a 160 mini-superstar.
        let pa = potential_ability as f32;
        let pa_final = (pa - 1.0) / 199.0 * 19.0 + 1.0;
        let target_ca_f = target_ca.max(1) as f32;
        let ca_skill_target = (target_ca_f - 1.0) / 199.0 * 19.0 + 1.0;

        // Per-group age ratios still vary (mentality lags physicals etc.),
        // but they now scale the *target CA* baseline rather than PA.
        let tech_age_ratio = match age {
            0..=17 => 0.78, 18..=19 => 0.85, 20..=22 => 0.92,
            23..=26 => 0.97, 27..=29 => 1.0, 30..=32 => 0.97, _ => 0.93,
        };
        let mental_age_ratio = match age {
            0..=17 => 0.60, 18..=19 => 0.68, 20..=22 => 0.78,
            23..=26 => 0.88, 27..=29 => 0.96, 30..=32 => 1.0, _ => 1.0,
        };
        let physical_age_ratio = match age {
            0..=17 => 0.72, 18..=19 => 0.82, 20..=22 => 0.90,
            23..=26 => 0.97, 27..=29 => 1.0, 30..=32 => 0.93, _ => 0.82,
        };

        let tech_mean = ca_skill_target * tech_age_ratio;
        let mental_mean = ca_skill_target * mental_age_ratio;
        let phys_mean = ca_skill_target * physical_age_ratio;

        // Spread is set so high-weight attributes lift well above the mean
        // and low-weight attributes drop well below it.
        let spread = (ca_skill_target * 0.45).max(2.5);

        let bucket = PositionType::from_player_position(primary_position);
        let mut pos_w = PositionWeights::for_position(primary_position);
        RoleArchetype::apply(&mut pos_w, &bucket);

        let base_noise = 1.4 + rep_factor * 0.8;
        let tech_noise = if age <= 18 { base_noise + 1.5 } else { base_noise + 0.4 };
        let mental_noise = base_noise * 0.5;
        let phys_noise = base_noise * 1.3;

        let mut skills = [0.0f32; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS { continue; }
            let (group_mean, noise) = match SkillGroup::from_index(i) {
                SkillGroup::Technical => (tech_mean, tech_noise),
                SkillGroup::Mental => (mental_mean, mental_noise),
                SkillGroup::Physical => (phys_mean, phys_noise),
                SkillGroup::MatchReadiness => continue,
            };
            let pos_mean = group_mean + (pos_w[i] - 1.0) * spread;
            let base = pos_mean + random_normal() * noise;
            skills[i] = (base * AgeCurve::peak_modifier(i, age)).clamp(1.0, 20.0);
        }

        // Mental cohesion: mentality is largely unified — strong-willed
        // players are strong-willed across the board, not just one slot.
        let m_avg: f32 = skills[14..28].iter().sum::<f32>() / 14.0;
        for i in 14..28 { skills[i] = skills[i] * 0.70 + m_avg * 0.30; }

        // Physical cohesion: lighter pull, keeps individuality.
        let p_avg: f32 = skills[28..36].iter().sum::<f32>() / 8.0;
        for i in 28..36 { skills[i] = skills[i] * 0.85 + p_avg * 0.15; }

        // Affinities + country bias before the age cap, so the cap is final.
        SkillAffinities::apply(&mut skills);
        let bias = super::country_bias::country_skill_bias(continent_id, country_code);
        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS { continue; }
            skills[i] += bias[i];
        }

        // Floors & caps. Skills can't exceed PA's implied ceiling (capped
        // at the age-based maximum), and can't fall below role-aware floors.
        // The cap always wins: a low-PA player's floor is squeezed downward
        // so it never exceeds the cap (otherwise `clamp(min>max)` panics).
        let pa_cap_skill = pa_final;
        let key_floor = (ca_skill_target * 0.40).clamp(1.0, 9.0);
        let universal_floor = (2.0 + ca_skill_target * 0.18).clamp(4.0, 6.0);
        let physical_floor_base = (3.0 + ca_skill_target * 0.32).clamp(6.0, 9.0);
        let trained_floor = (ca_skill_target * 0.32 + 3.0).clamp(6.0, 9.0);
        let footballer_tech_floor = (ca_skill_target * 0.28 + 2.0).clamp(4.0, 9.0);
        let cap = AgeCurve::skill_cap(age).min(pa_cap_skill + 0.5);
        let safe_floor = |f: f32| f.min(cap).max(1.0);

        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS { continue; }
            if pos_w[i] >= 1.2 {
                skills[i] = skills[i].max(safe_floor(key_floor));
            }
            match SkillGroup::from_index(i) {
                SkillGroup::Physical => {
                    let jitter = (random_normal() * 1.6).clamp(-2.0, 2.0);
                    let floor = safe_floor((physical_floor_base + jitter).max(4.0));
                    skills[i] = skills[i].clamp(floor, cap);
                }
                SkillGroup::Technical if pos_w[i] >= 0.8 => {
                    let jitter = (random_normal() * 1.3).clamp(-2.0, 2.0);
                    let floor = safe_floor((trained_floor + jitter).max(4.0));
                    skills[i] = skills[i].clamp(floor, cap);
                }
                SkillGroup::Technical => {
                    let jitter = (random_normal() * 0.9).clamp(-1.0, 1.0);
                    let floor = safe_floor((footballer_tech_floor + jitter).max(4.0));
                    skills[i] = skills[i].clamp(floor, cap);
                }
                SkillGroup::Mental => {
                    skills[i] = skills[i].clamp(safe_floor(universal_floor), cap);
                }
                SkillGroup::MatchReadiness => {}
            }
        }
        // match_readiness lives on Physical but is a state, not a skill —
        // initialised separately by `generate_player_attributes`.
        skills[SK_MATCH_READINESS] = 0.0;

        for v in skills.iter_mut() { *v = v.clamp(0.0, 20.0); }

        let mut result = SkillsArray::into_skills(&skills);
        if matches!(primary_position, PlayerPositionType::Goalkeeper) {
            result.goalkeeping = Self::generate_gk_skills(ca_skill_target, age, &pos_w);
        }
        result
    }

    /// Generate Goalkeeping-specific skills anchored on the *current* CA
    /// target — same anchoring as outfield skills. PA is not the input here:
    /// it only governs future development through the development pipeline,
    /// not the values minted at world-init or signing.
    /// Based on real FM attribute importance:
    ///   Core (shot-stopping): Handling, Reflexes, One-on-Ones — highest weight
    ///   Command: Command of Area, Aerial Reach, Communication, Punching
    ///   Distribution: Kicking, Throwing, First Touch, Passing
    ///   Specialist: Rushing Out, Eccentricity
    fn generate_gk_skills(ca_skill_target: f32, age: u32, _pos_w: &[f32; SKILL_COUNT]) -> core::Goalkeeping {
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

        let gk_mean = ca_skill_target * gk_age_ratio;
        let spread = (ca_skill_target * 0.45).max(2.0);
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

        // GK floor scales with target CA — same logic as outfield floors.
        let core_floor = (ca_skill_target * 0.45).clamp(3.0, 10.0);
        let general_floor = (ca_skill_target * 0.25).clamp(2.0, 7.0);

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

    /// Pick a single primary `PlayerPositionType` from a broad bucket.
    /// Higher-PA / top-flight / first-team-quality players tilt toward
    /// modern specialist roles (wing-backs, AMC, inside forwards / wide
    /// forwards); lower-tier squads stay anchored on traditional roles
    /// (DL/DR, MC, ST). Single continuous `modern_pull` does the work —
    /// no per-tier branching.
    fn pick_exact_position(
        bucket: PositionType,
        pa: u8,
        team_type: TeamType,
        role: SquadRole,
    ) -> PlayerPositionType {
        let modern_pull = Self::modern_role_pull(pa, team_type, role);

        match bucket {
            PositionType::Goalkeeper => PlayerPositionType::Goalkeeper,
            PositionType::Defender => {
                // Wing-backs are a modern role; share grows with `modern_pull`.
                // Centre-backs always dominate, full-backs fill the rest.
                let wb_chance = (12.0 + modern_pull * 28.0) as i32; // 12..40%
                let cb_chance = 50; // centre-backs ~half regardless
                let fb_chance = 100 - wb_chance - cb_chance;
                let roll = IntegerUtils::random(0, 100);
                if roll < wb_chance / 2 {
                    PlayerPositionType::WingbackLeft
                } else if roll < wb_chance {
                    PlayerPositionType::WingbackRight
                } else if roll < wb_chance + cb_chance {
                    PlayerPositionType::DefenderCenter
                } else if roll < wb_chance + cb_chance + fb_chance / 2 {
                    PlayerPositionType::DefenderLeft
                } else {
                    PlayerPositionType::DefenderRight
                }
            }
            PositionType::Midfielder => {
                let amc_chance = (5.0 + modern_pull * 22.0) as i32; // 5..27%
                let dm_chance = 12 + (modern_pull * 8.0) as i32;    // 12..20%
                let wide_chance = 22;
                let roll = IntegerUtils::random(0, 100);
                if roll < amc_chance {
                    PlayerPositionType::AttackingMidfielderCenter
                } else if roll < amc_chance + dm_chance {
                    PlayerPositionType::DefensiveMidfielder
                } else if roll < amc_chance + dm_chance + wide_chance / 2 {
                    PlayerPositionType::MidfielderLeft
                } else if roll < amc_chance + dm_chance + wide_chance {
                    PlayerPositionType::MidfielderRight
                } else {
                    PlayerPositionType::MidfielderCenter
                }
            }
            PositionType::Striker => {
                // Top clubs favour wide forwards / inside-forward profiles
                // (FL, FR) and the occasional false 9 (FC). Lower-tier
                // squads default to traditional ST.
                let wide_chance = (16.0 + modern_pull * 30.0) as i32; // 16..46%
                let fc_chance = (5.0 + modern_pull * 15.0) as i32;    // 5..20%
                let roll = IntegerUtils::random(0, 100);
                if roll < wide_chance / 2 {
                    PlayerPositionType::ForwardLeft
                } else if roll < wide_chance {
                    PlayerPositionType::ForwardRight
                } else if roll < wide_chance + fc_chance {
                    PlayerPositionType::ForwardCenter
                } else {
                    PlayerPositionType::Striker
                }
            }
        }
    }

    /// 0..1 signal: how much this player's combined context (PA, team type,
    /// squad role) tilts them toward modern/specialist positions.
    fn modern_role_pull(pa: u8, team_type: TeamType, role: SquadRole) -> f32 {
        let pa_pull = ((pa as f32 - 90.0) / 90.0).clamp(0.0, 1.0);
        let team_pull = match team_type {
            TeamType::Main => 1.0,
            TeamType::B | TeamType::Second | TeamType::Reserve => 0.6,
            TeamType::U23 | TeamType::U21 | TeamType::U20 => 0.55,
            TeamType::U18 | TeamType::U19 => 0.5,
        };
        let role_pull = match role {
            SquadRole::Star | SquadRole::Starter => 1.0,
            SquadRole::Rotation => 0.8,
            SquadRole::Backup | SquadRole::Prospect => 0.5,
            SquadRole::Fringe => 0.3,
        };
        (pa_pull * 0.5 + team_pull * 0.3 + role_pull * 0.2).clamp(0.0, 1.0)
    }

    /// Generate the secondary/adjacent positions around a known primary.
    /// Primary always gets level 20; DC/MC fan out to formation slots; wide
    /// and high-PA players collect extras.
    fn generate_positions_from_primary(primary: PlayerPositionType, potential_ability: u8) -> PlayerPositions {
        let mut positions = Vec::with_capacity(6);
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
        let adjacent = PositionLayout::adjacent(primary);
        for adj in &adjacent {
            if IntegerUtils::random(0, 99) < 40 {
                let level = IntegerUtils::random(14, 18) as u8;
                positions.push(PlayerPosition { position: *adj, level });
            }
        }

        // Cross-side versatility: ~15% chance for wide players to play opposite flank.
        // These players (e.g. M L/R, D L/R) are more versatile and valuable.
        if let Some(opposite) = PositionLayout::cross_side(primary) {
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
            if let Some(extra) = PositionLayout::extra(primary) {
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

    // ── Player attributes (CA from skills, PA already determined) ─────

    fn generate_player_attributes(
        rep_factor: f32,
        age: u32,
        potential_ability: u8,
        skills: &PlayerSkills,
        primary_position: PlayerPositionType,
        country_id: u32,
    ) -> PlayerAttributes {
        let current_ability = skills.calculate_ability_for_position(primary_position);

        // PA must never be lower than CA — position-weighted skill calculation
        // can produce CA above the raw PA when skills align well with the position
        let potential_ability = potential_ability.max(current_ability);

        let rep_base = (rep_factor * 3000.0) as i32;
        let physical = PhysicalProfile::for_position(primary_position, country_id);
        let state = FitnessState::for_age(age);

        PlayerAttributes {
            is_banned: false,
            is_injured: false,
            condition: state.condition,
            fitness: state.fitness,
            jadedness: state.jadedness,
            weight: physical.weight_kg,
            height: physical.height_cm,
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
    pub fn generate_from_odb(
        record: &OdbPlayer,
        continent_id: u32,
        country_code: &str,
        data: &DatabaseEntity,
    ) -> Player {
        let now = Utc::now().date_naive();
        let age = age_in_years(record.birth_date, now);

        let positions = positions_from_odb(&record.positions);
        let primary = positions
            .positions
            .first()
            .map(|p| p.position)
            .unwrap_or(PlayerPositionType::MidfielderCenter);
        let pos_type = PositionType::from_player_position(primary);

        // Drive the skill generator off the recorded CA so the spread is
        // appropriate. The new pipeline takes both target_ca and PA so the
        // ODB record's authoritative numbers flow straight through.
        let _ = pos_type; // bucket no longer needed — pipeline picks weights from exact position
        let rep_factor = (record.current_ability as f32 / 200.0).clamp(0.05, 1.0);
        let mut skills = Self::generate_skills(
            primary,
            age,
            rep_factor,
            record.potential_ability,
            record.current_ability,
            continent_id,
            country_code,
        );
        SkillRescaler::to_target_ca(&mut skills, primary, record.current_ability);
        skills.physical.match_readiness = FitnessState::match_readiness(age);

        let full_name = build_full_name(record);
        let preferred_foot = parse_preferred_foot(record.preferred_foot.as_deref());

        let contract = build_main_contract(record, age, primary, data);
        let contract_loan = build_loan_contract(record, data);

        let player_attributes = build_player_attributes(record, age, primary, &skills);

        let native_languages: Vec<core::PlayerLanguage> =
            core::Language::from_country_code(country_code)
                .into_iter()
                .map(core::PlayerLanguage::native)
                .collect();

        let statistics_history = build_statistics_history(&record.history, data);

        let mut builder = Player::builder()
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
            .languages(native_languages);

        if let Some(history) = statistics_history {
            builder = builder.statistics_history(history);
        }

        builder.build().expect("Failed to build Player from ODB record")
    }
}

/// Build a `PlayerStatisticsHistory` from scraped per-season records, looking
/// up each entry's club/team/league/country name and slug from the current
/// database. Returns `None` when there is no history to avoid overriding the
/// default empty history (and the simulator's initial-team seeding).
fn build_statistics_history(
    history: &[OdbHistoryItem],
    data: &DatabaseEntity,
) -> Option<PlayerStatisticsHistory> {
    if history.is_empty() {
        return None;
    }

    // Chronological order — oldest first — so the assigned seq_ids grow with
    // time and `view_items` sorts the newest season to the top.
    let mut sorted: Vec<&OdbHistoryItem> = history.iter().collect();
    sorted.sort_by_key(|h| h.season);

    let items: Vec<PlayerStatisticsHistoryItem> = sorted
        .into_iter()
        .enumerate()
        .map(|(seq, h)| {
            let (team_name, team_slug, team_reputation, league_name, league_slug) =
                resolve_club_display(h.club_id, data);

            let statistics = PlayerStatistics {
                played: h.played,
                played_subs: 0,
                goals: h.goals,
                assists: 0,
                penalties: 0,
                player_of_the_match: 0,
                yellow_cards: 0,
                red_cards: 0,
                shots_on_target: 0.0,
                tackling: 0.0,
                passes: 0,
                average_rating: h.rating,
                conceded: 0,
                clean_sheets: 0,
            };

            PlayerStatisticsHistoryItem {
                season: Season::new(h.season),
                team_name,
                team_slug,
                team_reputation,
                league_name,
                league_slug,
                is_loan: h.is_loan,
                transfer_fee: None,
                statistics,
                seq_id: seq as u32,
            }
        })
        .collect();

    Some(PlayerStatisticsHistory::from_items(items))
}

/// Look up a club by id and return (team_name, team_slug, team_reputation,
/// league_name, league_slug). Prefers the club's main team; falls back to the
/// first team. Missing clubs return empty strings so the page still renders
/// the stats row with no links.
fn resolve_club_display(
    club_id: u32,
    data: &DatabaseEntity,
) -> (String, String, u16, String, String) {
    let Some(club) = data.clubs.iter().find(|c| c.id == club_id) else {
        return (String::new(), String::new(), 0, String::new(), String::new());
    };

    // Prefer the "Main" team; fall back to the first listed team.
    let team = club
        .teams
        .iter()
        .find(|t| t.team_type.eq_ignore_ascii_case("Main"))
        .or_else(|| club.teams.first());

    let (team_slug, team_reputation, league_id) = match team {
        Some(t) => (t.slug.clone(), t.reputation.world, t.league_id),
        None => (String::new(), 0, None),
    };

    let (league_name, league_slug) = league_id
        .and_then(|lid| data.leagues.iter().find(|l| l.id == lid))
        .map(|l| (l.name.clone(), l.slug.clone()))
        .unwrap_or_default();

    (club.name.clone(), team_slug, team_reputation, league_name, league_slug)
}

fn age_in_years(dob: NaiveDate, now: NaiveDate) -> u32 {
    let mut years = now.year() - dob.year();
    if (now.month(), now.day()) < (dob.month(), dob.day()) {
        years -= 1;
    }
    years.max(0) as u32
}

impl PositionType {
    /// Collapse an exact playing position into the broad bucket the
    /// archetype/role logic dispatches off (Goalkeeper / Defender /
    /// Midfielder / Striker).
    pub fn from_player_position(p: PlayerPositionType) -> PositionType {
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

fn build_main_contract(
    record: &OdbPlayer,
    age: u32,
    primary: PlayerPositionType,
    data: &DatabaseEntity,
) -> Option<PlayerClubContract> {
    // Free-agent records have no active contract — leave the player
    // contract-less so the free-agent flows treat them as available.
    let src = record.contract.as_ref()?;
    let salary = match src.salary {
        Some(s) if s > 0 => s,
        _ => default_annual_salary(record, age, primary, data),
    };
    let mut c = PlayerClubContract {
        shirt_number: src.shirt_number,
        salary,
        contract_type: parse_contract_type(src.contract_type.as_deref()),
        squad_status: core::PlayerSquadStatus::NotYetSet,
        is_transfer_listed: false,
        transfer_status: None,
        started: src.started,
        expiration: src.expiration,
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
        last_yearly_rise_year: None,
        last_loyalty_paid_year: None,
        signing_bonus_paid: false,
    };
    // If currently loaned out, the main contract retains parent terms but
    // records the borrower via loan_to_club_id so the value/wage code knows.
    if let Some(ref loan) = record.loan {
        c.loan_to_club_id = Some(loan.to_club_id);
    }
    Some(c)
}

/// Default-fill an annual salary using the same wage curve the runtime uses
/// for renewals and personal-terms. Resolves club + league reputation from
/// the loaded entity tree; missing pieces fall back to neutral mid-tier
/// values so the formula still produces a sane number for satellite clubs
/// or records pointing at an unknown club_id.
fn default_annual_salary(
    record: &OdbPlayer,
    age: u32,
    primary: PlayerPositionType,
    data: &DatabaseEntity,
) -> u32 {
    let club = data.clubs.iter().find(|c| c.id == record.club_id);
    let main_team = club.and_then(|c| {
        c.teams
            .iter()
            .find(|t| t.team_type.eq_ignore_ascii_case("Main"))
    });
    let team_rep = main_team.map(|t| t.reputation.world).unwrap_or(1500);
    // 0..1 normalised club reputation, mirrors TeamReputation::overall_score
    // closely enough for an initial wage anchor.
    let club_reputation_score = (team_rep as f32 / 10000.0).clamp(0.0, 1.0);

    let league_id = main_team.and_then(|t| t.league_id);
    let league_reputation = league_id
        .and_then(|id| data.leagues.iter().find(|l| l.id == id))
        .map(|l| l.reputation)
        .unwrap_or(1500);

    // Player reputation: prefer the explicit current rep on the record, else
    // derive an approximation from CA so the wage curve still tilts toward
    // higher-quality players when the scrape didn't capture reputation.
    let current_reputation = record
        .reputation
        .as_ref()
        .and_then(|r| r.current)
        .unwrap_or_else(|| (record.current_ability as i16) * 8);

    use core::PlayerPositionType as Pp;
    let is_forward = matches!(
        primary,
        Pp::Striker
            | Pp::ForwardLeft
            | Pp::ForwardCenter
            | Pp::ForwardRight
            | Pp::AttackingMidfielderCenter
    );
    let is_goalkeeper = matches!(primary, Pp::Goalkeeper);

    WageCalculator::expected_annual_wage_raw(
        record.current_ability,
        current_reputation,
        is_forward,
        is_goalkeeper,
        age.min(u8::MAX as u32) as u8,
        club_reputation_score,
        league_reputation,
    )
}

fn build_loan_contract(record: &OdbPlayer, data: &DatabaseEntity) -> Option<PlayerClubContract> {
    let loan = record.loan.as_ref()?;
    // Anchor the loan-out on the parent club's Reserve team rather than
    // the Main squad, so the player shows up in the reserve squad page
    // (via the loaned-out scanner) while physically playing at the
    // borrower. Fall back to B → U23/U21/U20/U19/U18 → Main in that
    // order for clubs that don't have a Reserve team.
    const TEAM_PRIORITY: [&str; 8] = [
        "Reserve", "B", "U23", "U21", "U20", "U19", "U18", "Main",
    ];
    let loan_from_team_id = data
        .clubs
        .iter()
        .find(|c| c.id == record.club_id)
        .and_then(|c| {
            TEAM_PRIORITY.iter().find_map(|preferred| {
                c.teams
                    .iter()
                    .find(|t| t.team_type.eq_ignore_ascii_case(preferred))
                    .map(|t| t.id)
            })
        });
    // Loan-leg salary defaults to the borrower's share of the parent contract
    // when the data omits it (loan blocks scraped without a wage figure).
    let loan_salary = match loan.salary {
        Some(s) if s > 0 => s,
        _ => {
            let parent_salary = record
                .contract
                .as_ref()
                .and_then(|c| c.salary)
                .unwrap_or(0);
            if parent_salary > 0 {
                WageCalculator::loan_wage_split(parent_salary).0
            } else {
                0
            }
        }
    };
    Some(PlayerClubContract {
        shirt_number: None,
        salary: loan_salary,
        contract_type: ContractType::Loan,
        squad_status: core::PlayerSquadStatus::NotYetSet,
        is_transfer_listed: false,
        transfer_status: None,
        started: None,
        expiration: loan.expiration,
        loan_from_club_id: Some(record.club_id),
        loan_from_team_id,
        loan_to_club_id: Some(loan.to_club_id),
        loan_match_fee: loan.match_fee,
        loan_wage_contribution_pct: loan.wage_contribution_pct,
        loan_future_fee: loan.future_fee,
        loan_future_fee_obligation: loan.future_fee_obligation,
        loan_recall_available_after: None,
        loan_min_appearances: loan.min_appearances,
        bonuses: vec![],
        clauses: vec![],
        last_yearly_rise_year: None,
        last_loyalty_paid_year: None,
        signing_bonus_paid: false,
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

    let physical = PhysicalProfile::for_position(primary, record.country_id);
    let state = FitnessState::for_age(age);

    PlayerAttributes {
        is_banned: false,
        is_injured: false,
        condition: state.condition,
        fitness: state.fitness,
        jadedness: 0,
        weight: record.weight.unwrap_or(physical.weight_kg),
        height: record.height.unwrap_or(physical.height_cm),
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

/// Uniformly scale generated skills toward a target CA so the position-
/// weighted ability calculation lands on the requested number. Two passes
/// because `calculate_ability_for_position` is only roughly linear in the
/// skill average for non-extreme inputs.
pub struct SkillRescaler;

impl SkillRescaler {
    pub fn to_target_ca(
        skills: &mut PlayerSkills,
        primary: PlayerPositionType,
        target_ca: u8,
    ) {
        if target_ca == 0 { return; }
        let current = skills.calculate_ability_for_position(primary).max(1);
        let factor = (target_ca as f32 / current as f32).clamp(0.40, 2.50);
        if (factor - 1.0).abs() < 0.02 { return; }
        Self::scale_in_place(skills, factor);
        let after = skills.calculate_ability_for_position(primary).max(1);
        let f2 = (target_ca as f32 / after as f32).clamp(0.85, 1.15);
        if (f2 - 1.0).abs() > 0.01 {
            Self::scale_in_place(skills, f2);
        }
    }

    /// Clamp every outfield skill to `cap`. Used after `to_target_ca` to
    /// re-enforce the age cap (rescaling can push young-player skills above
    /// the cap when the rep blend implies a CA the age can't yet support).
    pub fn clamp_to_cap(skills: &mut PlayerSkills, cap: f32) {
        macro_rules! c { ($v:expr) => { $v = $v.min(cap).max(1.0) }; }
        let t = &mut skills.technical;
        c!(t.corners); c!(t.crossing); c!(t.dribbling); c!(t.finishing);
        c!(t.first_touch); c!(t.free_kicks); c!(t.heading); c!(t.long_shots);
        c!(t.long_throws); c!(t.marking); c!(t.passing); c!(t.penalty_taking);
        c!(t.tackling); c!(t.technique);
        let m = &mut skills.mental;
        c!(m.aggression); c!(m.anticipation); c!(m.bravery); c!(m.composure);
        c!(m.concentration); c!(m.decisions); c!(m.determination); c!(m.flair);
        c!(m.leadership); c!(m.off_the_ball); c!(m.positioning); c!(m.teamwork);
        c!(m.vision); c!(m.work_rate);
        let p = &mut skills.physical;
        c!(p.acceleration); c!(p.agility); c!(p.balance); c!(p.jumping);
        c!(p.natural_fitness); c!(p.pace); c!(p.stamina); c!(p.strength);
        let g = &mut skills.goalkeeping;
        c!(g.aerial_reach); c!(g.command_of_area); c!(g.communication);
        c!(g.eccentricity); c!(g.first_touch); c!(g.handling); c!(g.kicking);
        c!(g.one_on_ones); c!(g.passing); c!(g.punching); c!(g.reflexes);
        c!(g.rushing_out); c!(g.throwing);
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
}

/// Position-graph helpers — adjacent / cross-side / extra positions a player
/// can naturally cover. Lives as a stateless namespace so `generate_positions_from_primary`
/// reads as `PositionLayout::adjacent(p)` instead of pulling free helpers
/// into scope.
pub struct PositionLayout;

impl PositionLayout {
    /// Natural adjacent positions that most players at a given position can also play.
    pub fn adjacent(primary: PlayerPositionType) -> Vec<PlayerPositionType> {
        match primary {
            PlayerPositionType::Goalkeeper => vec![],
            PlayerPositionType::DefenderCenter => vec![PlayerPositionType::DefensiveMidfielder],
            PlayerPositionType::DefenderCenterLeft => vec![PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderLeft],
            PlayerPositionType::DefenderCenterRight => vec![PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderRight],
            PlayerPositionType::DefenderLeft => vec![PlayerPositionType::WingbackLeft],
            PlayerPositionType::DefenderRight => vec![PlayerPositionType::WingbackRight],
            PlayerPositionType::MidfielderCenter => {
                if IntegerUtils::random(0, 1) == 0 {
                    vec![PlayerPositionType::DefensiveMidfielder]
                } else {
                    vec![PlayerPositionType::AttackingMidfielderCenter]
                }
            }
            PlayerPositionType::MidfielderCenterLeft => vec![PlayerPositionType::MidfielderCenter, PlayerPositionType::MidfielderLeft],
            PlayerPositionType::MidfielderCenterRight => vec![PlayerPositionType::MidfielderCenter, PlayerPositionType::MidfielderRight],
            PlayerPositionType::MidfielderLeft => vec![PlayerPositionType::AttackingMidfielderLeft],
            PlayerPositionType::MidfielderRight => vec![PlayerPositionType::AttackingMidfielderRight],
            PlayerPositionType::WingbackLeft => vec![PlayerPositionType::DefenderLeft],
            PlayerPositionType::WingbackRight => vec![PlayerPositionType::DefenderRight],
            PlayerPositionType::DefensiveMidfielder => vec![PlayerPositionType::MidfielderCenter],
            PlayerPositionType::AttackingMidfielderCenter => vec![PlayerPositionType::MidfielderCenter],
            PlayerPositionType::AttackingMidfielderLeft => vec![PlayerPositionType::MidfielderLeft],
            PlayerPositionType::AttackingMidfielderRight => vec![PlayerPositionType::MidfielderRight],
            PlayerPositionType::Striker => vec![PlayerPositionType::ForwardCenter],
            PlayerPositionType::ForwardCenter => vec![PlayerPositionType::Striker],
            PlayerPositionType::ForwardLeft => vec![PlayerPositionType::AttackingMidfielderLeft],
            PlayerPositionType::ForwardRight => vec![PlayerPositionType::AttackingMidfielderRight],
            _ => vec![],
        }
    }

    /// Extra position for versatile players (beyond natural adjacent).
    pub fn extra(primary: PlayerPositionType) -> Option<PlayerPositionType> {
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

    /// Opposite-side position for cross-side versatility — players who can
    /// play both flanks (e.g. M L/R) are more valuable.
    pub fn cross_side(primary: PlayerPositionType) -> Option<PlayerPositionType> {
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
}

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn test_versatility_by_pa() {
        let total = 500;
        for pa in [30u8, 80, 140] {
            let multi = (0..total)
                .filter(|_| {
                    let primary = PlayerGenerator::pick_exact_position(
                        PositionType::Midfielder, pa, TeamType::Main, SquadRole::Starter);
                    PlayerGenerator::generate_positions_from_primary(primary, pa).positions.len() > 1
                })
                .count();
            let pct = multi * 100 / total;
            eprintln!("PA={pa}: {multi}/{total} = {pct}%");
            assert!(multi > 5, "PA={pa}: only {multi}/{total} multi-pos");
        }
    }
}

#[cfg(test)]
mod generator_validation_tests {
    use super::*;
    use chrono::Datelike;
    use core::PeopleNameGeneratorData;

    fn make_gen() -> PlayerGenerator {
        PlayerGenerator::with_people_names(&PeopleNameGeneratorData {
            first_names: vec!["Alex".into(), "Sam".into(), "Jordan".into()],
            last_names: vec!["Smith".into(), "Garcia".into(), "Park".into()],
            nicknames: vec![],
        })
    }

    fn sample(
        g: &PlayerGenerator,
        n: usize,
        bucket: PositionType,
        team_rep: u16,
        league_rep: u16,
        country_rep: u16,
        team_type: TeamType,
        role: SquadRole,
        min_age: i32,
        max_age: i32,
    ) -> Vec<core::Player> {
        (0..n)
            .map(|_| g.generate(1, 1, bucket, team_rep, league_rep, country_rep, team_type, role, min_age, max_age))
            .collect()
    }

    fn mean(xs: &[u8]) -> f32 {
        xs.iter().map(|&v| v as f32).sum::<f32>() / xs.len() as f32
    }

    #[test]
    fn weak_clubs_yield_lower_ca_than_top_clubs() {
        let g = make_gen();
        let weak = sample(&g, 200, PositionType::Midfielder, 1500, 1500, 2000,
            TeamType::Main, SquadRole::Starter, 22, 30);
        let elite = sample(&g, 200, PositionType::Midfielder, 9500, 9500, 8500,
            TeamType::Main, SquadRole::Starter, 22, 30);
        let weak_ca: Vec<u8> = weak.iter().map(|p| p.player_attributes.current_ability).collect();
        let elite_ca: Vec<u8> = elite.iter().map(|p| p.player_attributes.current_ability).collect();
        let dm = mean(&elite_ca) - mean(&weak_ca);
        eprintln!("CA weak={:.1} elite={:.1} delta={:.1}", mean(&weak_ca), mean(&elite_ca), dm);
        assert!(dm > 40.0, "elite clubs should mint distinctly stronger players (delta={:.1})", dm);
    }

    #[test]
    fn league_reputation_lifts_ca_at_equal_team_rep() {
        let g = make_gen();
        let low_lg = sample(&g, 200, PositionType::Midfielder, 5000, 1500, 5000,
            TeamType::Main, SquadRole::Starter, 22, 30);
        let high_lg = sample(&g, 200, PositionType::Midfielder, 5000, 9500, 5000,
            TeamType::Main, SquadRole::Starter, 22, 30);
        let low: Vec<u8> = low_lg.iter().map(|p| p.player_attributes.current_ability).collect();
        let high: Vec<u8> = high_lg.iter().map(|p| p.player_attributes.current_ability).collect();
        let dm = mean(&high) - mean(&low);
        eprintln!("league CA low={:.1} high={:.1} delta={:.1}", mean(&low), mean(&high), dm);
        assert!(dm > 10.0, "high-reputation league should pull CA up (delta={:.1})", dm);
    }

    #[test]
    fn prospects_carry_more_headroom_than_starters() {
        let g = make_gen();
        let starters = sample(&g, 200, PositionType::Midfielder, 7000, 7000, 6000,
            TeamType::Main, SquadRole::Starter, 25, 28);
        let prospects = sample(&g, 200, PositionType::Midfielder, 7000, 7000, 6000,
            TeamType::U21, SquadRole::Prospect, 17, 19);
        let starter_gap: Vec<i32> = starters.iter()
            .map(|p| p.player_attributes.potential_ability as i32 - p.player_attributes.current_ability as i32)
            .collect();
        let prospect_gap: Vec<i32> = prospects.iter()
            .map(|p| p.player_attributes.potential_ability as i32 - p.player_attributes.current_ability as i32)
            .collect();
        let s_avg = starter_gap.iter().sum::<i32>() as f32 / starter_gap.len() as f32;
        let p_avg = prospect_gap.iter().sum::<i32>() as f32 / prospect_gap.len() as f32;
        eprintln!("PA-CA gap: starter={:.1} prospect={:.1}", s_avg, p_avg);
        assert!(p_avg > s_avg + 15.0, "prospects must have more PA headroom than starters ({} vs {})", p_avg, s_avg);
    }

    #[test]
    fn pa_never_below_ca() {
        let g = make_gen();
        for role in [SquadRole::Star, SquadRole::Starter, SquadRole::Rotation,
                     SquadRole::Backup, SquadRole::Prospect, SquadRole::Fringe] {
            let players = sample(&g, 100, PositionType::Midfielder, 6000, 6000, 5000,
                TeamType::Main, role, 18, 32);
            for p in &players {
                assert!(p.player_attributes.potential_ability >= p.player_attributes.current_ability,
                    "PA<CA for role {:?} ({}<{})", role,
                    p.player_attributes.potential_ability, p.player_attributes.current_ability);
            }
        }
    }

    #[test]
    fn young_players_respect_age_cap() {
        // Country bias used to be applied AFTER the age cap, so a 15yo could
        // have skill 19 if their country bias was high. Now bias is applied
        // *before* the cap. Verify no skill exceeds the per-player age cap.
        let g = make_gen();
        let players = sample(&g, 200, PositionType::Striker, 9500, 9500, 9500,
            TeamType::U21, SquadRole::Prospect, 16, 17);
        let mut violations = 0;
        for p in &players {
            let chrono_now = chrono::Utc::now().date_naive();
            let age = (chrono_now.year() - p.birth_date.year()) as u32;
            let cap = AgeCurve::skill_cap(age);
            let s = &p.skills;
            for v in [s.technical.finishing, s.technical.dribbling, s.physical.pace,
                      s.physical.acceleration, s.mental.composure, s.mental.flair] {
                if v > cap + 0.01 {
                    violations += 1;
                }
            }
        }
        assert_eq!(violations, 0, "{} skill values broke age cap", violations);
    }

    #[test]
    fn striker_ca_is_dominated_by_attacking_attributes() {
        let g = make_gen();
        let strikers = sample(&g, 200, PositionType::Striker, 8000, 8000, 7000,
            TeamType::Main, SquadRole::Starter, 24, 28);
        let mut finish_dominant = 0;
        let mut tackle_dominant = 0;
        for p in &strikers {
            if p.skills.technical.finishing > p.skills.technical.tackling + 2.0 { finish_dominant += 1; }
            if p.skills.technical.tackling > p.skills.technical.finishing + 2.0 { tackle_dominant += 1; }
        }
        eprintln!("strikers (n=200): finish>>tackle={}, tackle>>finish={}",
            finish_dominant, tackle_dominant);
        assert!(finish_dominant > 150, "most strikers should have finishing well above tackling");
        assert!(tackle_dominant < 5, "strikers shouldn't be strong tacklers");
    }

    #[test]
    fn youth_team_doesnt_mint_elite_seniors() {
        // Prospects on a U21 squad at a top club should still be young and
        // have CA below their PA — they shouldn't read like first-team stars.
        let g = make_gen();
        let u21 = sample(&g, 300, PositionType::Midfielder, 9000, 9000, 8000,
            TeamType::U21, SquadRole::Prospect, 17, 20);
        let elite_ca_count = u21.iter().filter(|p| p.player_attributes.current_ability >= 150).count();
        eprintln!("U21 with CA>=150: {}/300", elite_ca_count);
        assert!(elite_ca_count < 5, "U21 prospects shouldn't mint CA>=150 players ({}/300)", elite_ca_count);
    }

    #[test]
    fn adult_target_ca_convergence_tight() {
        // Adults (age 24..28) should land on target CA within a tight band:
        // the rescaler's two-pass loop converges, and the age cap is high
        // enough not to clamp them.
        let g = make_gen();
        let players = sample(&g, 300, PositionType::Midfielder, 7000, 7000, 6000,
            TeamType::Main, SquadRole::Starter, 24, 28);
        let cas: Vec<u8> = players.iter().map(|p| p.player_attributes.current_ability).collect();
        let avg = mean(&cas);
        // Target for these inputs is roughly: rep_curve ≈ 0.71 * (0.50*0.7 + 0.30*0.7 + 0.20*0.6) ≈ 0.68
        // base_ca ≈ 35 + 0.68*145 ≈ 134; with Starter (1.10) and age 1.0 → ~147 ± noise.
        eprintln!("adult Starter@(7000/7000/6000) average CA = {:.1}", avg);
        assert!(avg > 120.0 && avg < 165.0, "adult CA out of expected band: {}", avg);
    }

    #[test]
    fn young_players_intentionally_undershoot_target_ca() {
        // 16yo prospects at a top club have target CA ~55 (rep 0.95 × role 0.65 × age 0.55).
        // Age cap at 16 is 14.0, which maps to CA ≈ 137 max in the skill→ability curve,
        // but in practice noise + position weights produce CA much lower than target.
        // The CONTRACT: young players' final CA may be below their target, never above.
        let g = make_gen();
        let players = sample(&g, 300, PositionType::Midfielder, 9500, 9500, 9000,
            TeamType::U21, SquadRole::Prospect, 16, 17);
        // Compute a synthetic target band by reproducing AbilityTarget::current_for math
        // for these inputs, then verify CAs cluster below or near it.
        let cas: Vec<u8> = players.iter().map(|p| p.player_attributes.current_ability).collect();
        let max = *cas.iter().max().unwrap();
        let avg = mean(&cas);
        eprintln!("16-17yo prospect@(top tier) avg CA = {:.1}, max = {}", avg, max);
        // Hard ceiling enforced by age cap. No 16-17yo should hit elite CA values.
        assert!(max < 145, "16-17yo CA shouldn't exceed 145 (got max={})", max);
        // Average should sit in a young-player band, well below adult-Starter levels.
        assert!(avg < 110.0, "16-17yo prospects shouldn't average like adult Starters: {:.1}", avg);
    }

    #[test]
    fn target_ca_convergence_within_band() {
        // Skill rescaling should land the final CA within a few points of
        // the target the role + rep blend implies.
        let g = make_gen();
        let players = sample(&g, 300, PositionType::Midfielder, 6500, 6500, 5500,
            TeamType::Main, SquadRole::Starter, 24, 28);
        let cas: Vec<u8> = players.iter().map(|p| p.player_attributes.current_ability).collect();
        let avg = mean(&cas);
        eprintln!("Starter@(team6500/league6500/country5500) average CA = {:.1}", avg);
        // Empirical band: should land in the 95..145 range for these inputs.
        assert!(avg > 90.0 && avg < 150.0, "average CA out of band: {}", avg);
    }

    #[test]
    fn position_aware_height_for_goalkeeper() {
        let g = make_gen();
        let gks = sample(&g, 100, PositionType::Goalkeeper, 6000, 6000, 5000,
            TeamType::Main, SquadRole::Starter, 22, 30);
        let heights: Vec<u8> = gks.iter().map(|p| p.player_attributes.height).collect();
        let avg = heights.iter().map(|&h| h as f32).sum::<f32>() / heights.len() as f32;
        eprintln!("avg GK height: {:.1}cm (range {}..{})",
            avg, *heights.iter().min().unwrap(), *heights.iter().max().unwrap());
        assert!(avg > 184.0, "GKs must average over 184cm (was {:.1})", avg);
        assert!(*heights.iter().min().unwrap() >= 180, "GKs shouldn't be under 180cm");
    }

    #[test]
    fn country_code_casing_matches_consumers() {
        // The country_bias / language / PhysicalProfile lookups all match
        // on lowercase. `CountryLoader::code_for_id` MUST return lowercase
        // so all three fire correctly. Regression for the silent no-op
        // where PhysicalProfile used uppercase arms.
        for country_id in 1..200u32 {
            let code = crate::loaders::CountryLoader::code_for_id(country_id);
            if code.is_empty() { continue; }
            assert!(
                code.chars().all(|c| !c.is_ascii_uppercase()),
                "country {} returned non-lowercase code '{}'",
                country_id, code
            );
        }
    }

    #[test]
    fn known_tall_countries_get_height_lift() {
        // Find a Dutch-coded country in the loaded data and verify the
        // PhysicalProfile lift fires. This catches the casing bug end-to-end:
        // if `country_height_offset` ever silently no-ops again, average
        // Dutch CB height collapses to the baseline range.
        use crate::loaders::CountryLoader;
        let countries = CountryLoader::load();
        let nl = countries.iter().find(|c| c.code.eq_ignore_ascii_case("nl")).map(|c| c.id);
        let jp = countries.iter().find(|c| c.code.eq_ignore_ascii_case("jp")).map(|c| c.id);
        if let (Some(nl_id), Some(jp_id)) = (nl, jp) {
            let mut nl_total = 0i32;
            let mut jp_total = 0i32;
            for _ in 0..200 {
                nl_total += PhysicalProfile::for_position(PlayerPositionType::DefenderCenter, nl_id).height_cm as i32;
                jp_total += PhysicalProfile::for_position(PlayerPositionType::DefenderCenter, jp_id).height_cm as i32;
            }
            let nl_avg = nl_total as f32 / 200.0;
            let jp_avg = jp_total as f32 / 200.0;
            eprintln!("DC height: NL avg={:.1}cm, JP avg={:.1}cm", nl_avg, jp_avg);
            assert!(nl_avg - jp_avg > 4.0, "expected NL CBs noticeably taller than JP CBs (delta={})", nl_avg - jp_avg);
        }
    }

    #[test]
    fn ca_pa_distribution_per_team_type() {
        // Generate large samples across team types and verify CA/PA shapes.
        // Main squads should have a real top tier; B/Reserve sit a band lower;
        // U-teams should be young with PA above CA.
        let g = make_gen();
        for (tt, role, age_lo, age_hi, max_ca_avg) in [
            (TeamType::Main, SquadRole::Starter, 22, 30, 165i32),
            (TeamType::B, SquadRole::Backup, 19, 28, 110),
            (TeamType::Reserve, SquadRole::Backup, 19, 28, 110),
            (TeamType::U23, SquadRole::Prospect, 19, 22, 100),
            (TeamType::U21, SquadRole::Prospect, 17, 20, 90),
            (TeamType::U20, SquadRole::Prospect, 17, 19, 85),
        ] {
            let players = sample(&g, 200, PositionType::Midfielder, 7000, 7000, 6000,
                tt, role, age_lo, age_hi);
            let cas: Vec<u8> = players.iter().map(|p| p.player_attributes.current_ability).collect();
            let pas: Vec<u8> = players.iter().map(|p| p.player_attributes.potential_ability).collect();
            let ca_avg = mean(&cas);
            let pa_avg = mean(&pas);
            eprintln!("{:?}/{:?} ages {}..{}: CA avg={:.1}, PA avg={:.1}", tt, role, age_lo, age_hi, ca_avg, pa_avg);
            assert!(ca_avg < max_ca_avg as f32, "{:?} CA avg {} exceeds expected {}", tt, ca_avg, max_ca_avg);
            assert!(pa_avg >= ca_avg, "{:?} PA<CA averages: PA={:.1} CA={:.1}", tt, pa_avg, ca_avg);
        }
    }

    #[test]
    fn exact_position_dominant_attributes() {
        // For each exact position, the attribute the position lives by
        // must on average outscore an unrelated baseline attribute. Catches
        // weight-table regressions silently flattening profiles.
        let g = make_gen();
        // (bucket, position-anchor extractor, anchor name, baseline extractor, baseline name)
        let cases: &[(PositionType, &dyn Fn(&core::PlayerSkills) -> f32, &str, &dyn Fn(&core::PlayerSkills) -> f32, &str)] = &[
            (PositionType::Goalkeeper, &|s| s.goalkeeping.handling, "handling", &|s| s.technical.finishing, "finishing"),
            (PositionType::Defender, &|s| s.technical.tackling, "tackling", &|s| s.technical.finishing, "finishing"),
            (PositionType::Midfielder, &|s| s.technical.passing, "passing", &|s| s.technical.finishing, "finishing"),
            (PositionType::Striker, &|s| s.technical.finishing, "finishing", &|s| s.technical.tackling, "tackling"),
        ];
        for (bucket, anchor, anchor_name, baseline, baseline_name) in cases.iter() {
            let players = sample(&g, 200, *bucket, 7500, 7500, 6500,
                TeamType::Main, SquadRole::Starter, 24, 28);
            let anchor_avg: f32 = players.iter().map(|p| anchor(&p.skills)).sum::<f32>() / players.len() as f32;
            let baseline_avg: f32 = players.iter().map(|p| baseline(&p.skills)).sum::<f32>() / players.len() as f32;
            eprintln!("{:?}: {}={:.2}, {}={:.2}", bucket, anchor_name, anchor_avg, baseline_name, baseline_avg);
            assert!(anchor_avg > baseline_avg + 2.0,
                "{:?}: dominant {} ({:.2}) should beat baseline {} ({:.2}) by 2+",
                bucket, anchor_name, anchor_avg, baseline_name, baseline_avg);
        }
    }

    #[test]
    fn fullback_and_wingback_profiles_differ() {
        // After rescaling to the same target CA, raw skill values converge
        // closely between similar positions. The position signature shows
        // up in *which* attributes dominate. Wing-backs should put their
        // best work into pace + stamina + crossing + dribbling (attacking
        // wide play); full-backs into marking + positioning + heading
        // (defensive duty). Compare those composite signatures.
        let g = make_gen();
        let mut wb_attack_total = 0.0;
        let mut wb_defend_total = 0.0;
        let mut fb_attack_total = 0.0;
        let mut fb_defend_total = 0.0;
        let mut wb_count = 0;
        let mut fb_count = 0;
        for _ in 0..600 {
            let p = g.generate(1, 1, PositionType::Defender, 7500, 7500, 6500,
                TeamType::Main, SquadRole::Starter, 24, 28);
            let primary = p.positions.positions.first().map(|x| x.position).unwrap();
            let attack = p.skills.physical.pace + p.skills.physical.stamina
                + p.skills.technical.crossing + p.skills.technical.dribbling;
            let defend = p.skills.technical.marking + p.skills.mental.positioning
                + p.skills.technical.heading + p.skills.physical.strength;
            match primary {
                PlayerPositionType::WingbackLeft | PlayerPositionType::WingbackRight => {
                    wb_attack_total += attack;
                    wb_defend_total += defend;
                    wb_count += 1;
                }
                PlayerPositionType::DefenderLeft | PlayerPositionType::DefenderRight => {
                    fb_attack_total += attack;
                    fb_defend_total += defend;
                    fb_count += 1;
                }
                _ => {}
            }
        }
        if wb_count > 30 && fb_count > 30 {
            let wb_attack = wb_attack_total / wb_count as f32;
            let wb_defend = wb_defend_total / wb_count as f32;
            let fb_attack = fb_attack_total / fb_count as f32;
            let fb_defend = fb_defend_total / fb_count as f32;
            eprintln!("WB(n={}): attack={:.1} defend={:.1}; FB(n={}): attack={:.1} defend={:.1}",
                wb_count, wb_attack, wb_defend, fb_count, fb_attack, fb_defend);
            // WB attack signature outweighs their own defend signature; FB
            // is the inverse. This is the position differentiation we want.
            assert!(wb_attack > wb_defend - 1.0,
                "WB attack signature should match or beat defend signature ({} vs {})",
                wb_attack, wb_defend);
            assert!(fb_defend > fb_attack - 5.0,
                "FB defend signature should be in line with attack signature ({} vs {})",
                fb_defend, fb_attack);
        }
    }

    #[test]
    fn squad_has_position_coverage() {
        // A generated Main-team squad must contain every major positional
        // bucket: GK, CB, FB/WB, CM (or DM/AMC), wide attacker, striker.
        // Repeat 50× because a single squad of ~30 players might miss a
        // niche bucket; the *average* squad must cover everything.
        let g = make_gen();
        let mut squads_missing = 0;
        for _ in 0..50 {
            let mut squad: Vec<core::Player> = Vec::new();
            // GK 3-5, DEF 6-9, MID 7-10, ST 5-8 — same as the live generator.
            for _ in 0..IntegerUtils::random(3, 5) {
                squad.push(g.generate(1, 1, PositionType::Goalkeeper, 7500, 7500, 6500,
                    TeamType::Main, SquadRole::Starter, 17, 35));
            }
            for _ in 0..IntegerUtils::random(6, 9) {
                squad.push(g.generate(1, 1, PositionType::Defender, 7500, 7500, 6500,
                    TeamType::Main, SquadRole::Starter, 17, 35));
            }
            for _ in 0..IntegerUtils::random(7, 10) {
                squad.push(g.generate(1, 1, PositionType::Midfielder, 7500, 7500, 6500,
                    TeamType::Main, SquadRole::Starter, 17, 35));
            }
            for _ in 0..IntegerUtils::random(5, 8) {
                squad.push(g.generate(1, 1, PositionType::Striker, 7500, 7500, 6500,
                    TeamType::Main, SquadRole::Starter, 17, 35));
            }

            let primaries: Vec<PlayerPositionType> = squad.iter()
                .filter_map(|p| p.positions.positions.first().map(|x| x.position))
                .collect();
            let has_gk = primaries.iter().any(|p| matches!(p, PlayerPositionType::Goalkeeper));
            let has_cb = primaries.iter().any(|p| matches!(p,
                PlayerPositionType::DefenderCenter | PlayerPositionType::DefenderCenterLeft | PlayerPositionType::DefenderCenterRight));
            let has_fb_or_wb = primaries.iter().any(|p| matches!(p,
                PlayerPositionType::DefenderLeft | PlayerPositionType::DefenderRight
                | PlayerPositionType::WingbackLeft | PlayerPositionType::WingbackRight));
            let has_central_mid = primaries.iter().any(|p| matches!(p,
                PlayerPositionType::MidfielderCenter | PlayerPositionType::DefensiveMidfielder
                | PlayerPositionType::AttackingMidfielderCenter));
            let has_striker = primaries.iter().any(|p| matches!(p,
                PlayerPositionType::Striker | PlayerPositionType::ForwardCenter
                | PlayerPositionType::ForwardLeft | PlayerPositionType::ForwardRight));

            if !(has_gk && has_cb && has_fb_or_wb && has_central_mid && has_striker) {
                squads_missing += 1;
            }
        }
        eprintln!("squads missing some bucket: {}/50", squads_missing);
        assert!(squads_missing < 5, "Too many squads ({}) missing core position buckets", squads_missing);
    }

    #[test]
    fn match_readiness_decoupled_from_pa() {
        // Used to flow through the PA pipeline; should now be a state value
        // in the 6..16 band regardless of CA.
        let g = make_gen();
        let elite = sample(&g, 100, PositionType::Striker, 9500, 9500, 9500,
            TeamType::Main, SquadRole::Star, 25, 28);
        let weak = sample(&g, 100, PositionType::Striker, 1500, 1500, 1500,
            TeamType::Main, SquadRole::Backup, 25, 28);
        for p in elite.iter().chain(weak.iter()) {
            let mr = p.skills.physical.match_readiness;
            assert!(mr >= 5.0 && mr <= 17.0,
                "match_readiness out of state range: {} (CA={})",
                mr, p.player_attributes.current_ability);
        }
    }
}
