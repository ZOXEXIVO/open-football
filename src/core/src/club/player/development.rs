//! Player skill development system.
//!
//! Key principles:
//! 1. Position-aware: skills relevant to the player's position develop faster
//!    and have a higher ceiling. Irrelevant skills stay low.
//! 2. Age curve: physical skills peak 24-28, decline from ~30; mental skills
//!    can grow into the 30s; technical skills plateau in the late 20s.
//! 3. Personality: professionalism, ambition, determination drive growth rate.
//! 4. Match experience: playing competitive matches accelerates development.
//! 5. Potential ceiling: PA gates maximum achievable level; per-skill ceilings
//!    based on PA x position weight create realistic skill profiles.
//! 6. Workload: tired, jaded or injured players don't grow normally.
//!
//! ## Testing seam
//!
//! The public entry point [`Player::process_development`] uses the global
//! thread-local RNG, which makes results irreproducible. The internal
//! [`Player::process_development_with`] variant accepts any
//! [`RollSource`] so tests can drive a deterministic stream of rolls and
//! assert on stable outputs.

use crate::club::player::player::Player;
use crate::utils::DateUtils;
use crate::PlayerPositionType;
use chrono::NaiveDate;

// ── Skill registry ──────────────────────────────────────────────────────
//
// Internally we operate on a flat [f32; 50] for speed. To keep the index
// constants in lockstep with the actual `PlayerSkills` fields, they are
// defined relative to a single source of truth: the `SkillKey` enum.
//
// Adding or reordering a variant in `SkillKey` automatically shifts the
// SK_* constants. The round-trip test in this module proves the
// `skills_to_array` / `write_skills_back` mapping covers every variant.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillKey {
    // Technical 0..14
    Corners, Crossing, Dribbling, Finishing, FirstTouch,
    FreeKicks, Heading, LongShots, LongThrows, Marking,
    Passing, PenaltyTaking, Tackling, Technique,
    // Mental 14..28
    Aggression, Anticipation, Bravery, Composure, Concentration,
    Decisions, Determination, Flair, Leadership, OffTheBall,
    Positioning, Teamwork, Vision, WorkRate,
    // Physical 28..37 (MatchReadiness sits at the end of the band — it's
    // managed by the training/match system, not the development tick)
    Acceleration, Agility, Balance, Jumping, NaturalFitness,
    Pace, Stamina, Strength, MatchReadiness,
    // Goalkeeping 37..50
    GkAerialReach, GkCommandOfArea, GkCommunication, GkEccentricity,
    GkFirstTouch, GkHandling, GkKicking, GkOneOnOnes, GkPassing,
    GkPunching, GkReflexes, GkRushingOut, GkThrowing,
}

impl SkillKey {
    pub const fn idx(self) -> usize { self as usize }
}

const SKILL_COUNT: usize = 50;

const SK_CORNERS: usize = SkillKey::Corners.idx();
const SK_CROSSING: usize = SkillKey::Crossing.idx();
const SK_DRIBBLING: usize = SkillKey::Dribbling.idx();
const SK_FINISHING: usize = SkillKey::Finishing.idx();
const SK_FIRST_TOUCH: usize = SkillKey::FirstTouch.idx();
const SK_FREE_KICKS: usize = SkillKey::FreeKicks.idx();
const SK_HEADING: usize = SkillKey::Heading.idx();
const SK_LONG_SHOTS: usize = SkillKey::LongShots.idx();
const SK_LONG_THROWS: usize = SkillKey::LongThrows.idx();
const SK_MARKING: usize = SkillKey::Marking.idx();
const SK_PASSING: usize = SkillKey::Passing.idx();
const SK_PENALTY_TAKING: usize = SkillKey::PenaltyTaking.idx();
const SK_TACKLING: usize = SkillKey::Tackling.idx();
const SK_TECHNIQUE: usize = SkillKey::Technique.idx();
const SK_AGGRESSION: usize = SkillKey::Aggression.idx();
const SK_ANTICIPATION: usize = SkillKey::Anticipation.idx();
const SK_BRAVERY: usize = SkillKey::Bravery.idx();
const SK_COMPOSURE: usize = SkillKey::Composure.idx();
const SK_CONCENTRATION: usize = SkillKey::Concentration.idx();
const SK_DECISIONS: usize = SkillKey::Decisions.idx();
const SK_DETERMINATION: usize = SkillKey::Determination.idx();
const SK_FLAIR: usize = SkillKey::Flair.idx();
const SK_LEADERSHIP: usize = SkillKey::Leadership.idx();
const SK_OFF_THE_BALL: usize = SkillKey::OffTheBall.idx();
const SK_POSITIONING: usize = SkillKey::Positioning.idx();
const SK_TEAMWORK: usize = SkillKey::Teamwork.idx();
const SK_VISION: usize = SkillKey::Vision.idx();
const SK_WORK_RATE: usize = SkillKey::WorkRate.idx();
const SK_ACCELERATION: usize = SkillKey::Acceleration.idx();
const SK_AGILITY: usize = SkillKey::Agility.idx();
const SK_BALANCE: usize = SkillKey::Balance.idx();
const SK_JUMPING: usize = SkillKey::Jumping.idx();
const SK_NATURAL_FITNESS: usize = SkillKey::NaturalFitness.idx();
const SK_PACE: usize = SkillKey::Pace.idx();
const SK_STAMINA: usize = SkillKey::Stamina.idx();
const SK_STRENGTH: usize = SkillKey::Strength.idx();
const SK_MATCH_READINESS: usize = SkillKey::MatchReadiness.idx();
const SK_GK_AERIAL_REACH: usize = SkillKey::GkAerialReach.idx();
const SK_GK_COMMAND_OF_AREA: usize = SkillKey::GkCommandOfArea.idx();
const SK_GK_COMMUNICATION: usize = SkillKey::GkCommunication.idx();
const SK_GK_ECCENTRICITY: usize = SkillKey::GkEccentricity.idx();
const SK_GK_FIRST_TOUCH: usize = SkillKey::GkFirstTouch.idx();
const SK_GK_HANDLING: usize = SkillKey::GkHandling.idx();
const SK_GK_KICKING: usize = SkillKey::GkKicking.idx();
const SK_GK_ONE_ON_ONES: usize = SkillKey::GkOneOnOnes.idx();
const SK_GK_PASSING: usize = SkillKey::GkPassing.idx();
const SK_GK_PUNCHING: usize = SkillKey::GkPunching.idx();
const SK_GK_REFLEXES: usize = SkillKey::GkReflexes.idx();
const SK_GK_RUSHING_OUT: usize = SkillKey::GkRushingOut.idx();
const SK_GK_THROWING: usize = SkillKey::GkThrowing.idx();

// Compile-time invariant: the enum must have exactly SKILL_COUNT variants
// and the GK band must end at SKILL_COUNT - 1.
const _: () = {
    assert!(SK_GK_THROWING == SKILL_COUNT - 1);
};

// ── Skill category ──────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SkillCategory {
    Technical,
    Mental,
    Physical,
    /// GK-specific skills: peak later (28-33), decline slowly — GKs have
    /// long careers.
    Goalkeeping,
}

fn skill_category(idx: usize) -> SkillCategory {
    match idx {
        SK_ACCELERATION | SK_AGILITY | SK_BALANCE | SK_JUMPING | SK_NATURAL_FITNESS | SK_PACE
        | SK_STAMINA | SK_STRENGTH | SK_MATCH_READINESS => SkillCategory::Physical,
        SK_AGGRESSION | SK_ANTICIPATION | SK_BRAVERY | SK_COMPOSURE | SK_CONCENTRATION
        | SK_DECISIONS | SK_DETERMINATION | SK_FLAIR | SK_LEADERSHIP | SK_OFF_THE_BALL
        | SK_POSITIONING | SK_TEAMWORK | SK_VISION | SK_WORK_RATE => SkillCategory::Mental,
        SK_GK_AERIAL_REACH..=SK_GK_THROWING => SkillCategory::Goalkeeping,
        _ => SkillCategory::Technical,
    }
}

// ── Position group for development weights ──────────────────────────────
//
// IMPORTANT: This grouping intentionally diverges from
// `PlayerPositionType::position_group()` for `DefensiveMidfielder`.
// The canonical position group treats DM as a Defender (because they
// drop deep, screen the back four, and are evaluated using defensive
// weights). For *development*, however, a DM grows the same skill set
// as a central midfielder: passing, vision, stamina, decisions. Treating
// them as a defender for development would slow their growth on the
// skills that actually define their role. The divergence is contained
// to this file; ability calculations elsewhere keep using
// `position_group()`.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PosGroup {
    Goalkeeper,
    Defender,
    Midfielder,
    Forward,
}

fn pos_group_from(pos: PlayerPositionType) -> PosGroup {
    match pos {
        PlayerPositionType::Goalkeeper => PosGroup::Goalkeeper,
        PlayerPositionType::Sweeper
        | PlayerPositionType::DefenderLeft
        | PlayerPositionType::DefenderCenterLeft
        | PlayerPositionType::DefenderCenter
        | PlayerPositionType::DefenderCenterRight
        | PlayerPositionType::DefenderRight => PosGroup::Defender,
        PlayerPositionType::WingbackLeft
        | PlayerPositionType::WingbackRight
        | PlayerPositionType::DefensiveMidfielder
        | PlayerPositionType::MidfielderLeft
        | PlayerPositionType::MidfielderCenterLeft
        | PlayerPositionType::MidfielderCenter
        | PlayerPositionType::MidfielderCenterRight
        | PlayerPositionType::MidfielderRight => PosGroup::Midfielder,
        PlayerPositionType::AttackingMidfielderLeft
        | PlayerPositionType::AttackingMidfielderCenter
        | PlayerPositionType::AttackingMidfielderRight
        | PlayerPositionType::ForwardLeft
        | PlayerPositionType::ForwardCenter
        | PlayerPositionType::ForwardRight
        | PlayerPositionType::Striker => PosGroup::Forward,
    }
}

// ── Position-based development weights ──────────────────────────────────
//
// These weights serve TWO purposes:
// 1. Per-skill CEILING = base_ceiling * weight (key skills can reach high,
//    irrelevant stay low).
// 2. Per-skill GROWTH RATE multiplier (key skills develop faster).
//
// Range: 0.3 (irrelevant) to 1.5 (core skill)
// Default: 0.8 for unspecified skills

fn position_dev_weights(group: PosGroup) -> [f32; SKILL_COUNT] {
    let mut w = [0.8f32; SKILL_COUNT];

    // GK-specific skills default to 0 for outfield players: they don't train them.
    for i in SK_GK_AERIAL_REACH..=SK_GK_THROWING {
        w[i] = 0.0;
    }

    match group {
        PosGroup::Goalkeeper => {
            w[SK_POSITIONING] = 1.5;
            w[SK_CONCENTRATION] = 1.4;
            w[SK_AGILITY] = 1.4;
            w[SK_ANTICIPATION] = 1.3;
            w[SK_COMPOSURE] = 1.3;
            w[SK_JUMPING] = 1.3;
            w[SK_BRAVERY] = 1.2;
            w[SK_DECISIONS] = 1.1;
            w[SK_STRENGTH] = 1.0;
            // Modern GK
            w[SK_FIRST_TOUCH] = 1.0;
            w[SK_PASSING] = 1.0;
            w[SK_TECHNIQUE] = 0.9;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_PACE] = 0.8;
            w[SK_STAMINA] = 0.8;
            // Irrelevant outfield skills
            w[SK_FINISHING] = 0.3;
            w[SK_LONG_SHOTS] = 0.3;
            w[SK_CROSSING] = 0.3;
            w[SK_CORNERS] = 0.3;
            w[SK_FREE_KICKS] = 0.35;
            w[SK_HEADING] = 0.35;
            w[SK_OFF_THE_BALL] = 0.35;
            w[SK_DRIBBLING] = 0.4;
            w[SK_LONG_THROWS] = 0.5;
            w[SK_TACKLING] = 0.35;
            w[SK_MARKING] = 0.35;
            w[SK_WORK_RATE] = 0.5;
            w[SK_FLAIR] = 0.4;
            w[SK_ACCELERATION] = 0.6;
            // Goalkeeping-specific attributes
            w[SK_GK_HANDLING] = 1.5;
            w[SK_GK_REFLEXES] = 1.5;
            w[SK_GK_ONE_ON_ONES] = 1.4;
            w[SK_GK_AERIAL_REACH] = 1.3;
            w[SK_GK_COMMAND_OF_AREA] = 1.3;
            w[SK_GK_COMMUNICATION] = 1.3;
            w[SK_GK_RUSHING_OUT] = 1.2;
            w[SK_GK_PUNCHING] = 1.2;
            w[SK_GK_KICKING] = 1.1;
            w[SK_GK_THROWING] = 1.1;
            w[SK_GK_FIRST_TOUCH] = 1.0;
            w[SK_GK_PASSING] = 1.0;
            w[SK_GK_ECCENTRICITY] = 0.6;
        }
        PosGroup::Defender => {
            w[SK_TACKLING] = 1.4;
            w[SK_MARKING] = 1.4;
            w[SK_POSITIONING] = 1.4;
            w[SK_HEADING] = 1.3;
            w[SK_STRENGTH] = 1.2;
            w[SK_CONCENTRATION] = 1.2;
            w[SK_ANTICIPATION] = 1.2;
            w[SK_BRAVERY] = 1.2;
            w[SK_JUMPING] = 1.1;
            w[SK_PACE] = 1.0;
            w[SK_PASSING] = 1.0;
            w[SK_TEAMWORK] = 1.0;
            w[SK_DECISIONS] = 1.0;
            w[SK_COMPOSURE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_STAMINA] = 1.0;
            // Less relevant for defenders
            w[SK_FINISHING] = 0.4;
            w[SK_DRIBBLING] = 0.5;
            w[SK_FLAIR] = 0.4;
            w[SK_LONG_SHOTS] = 0.4;
            w[SK_OFF_THE_BALL] = 0.5;
            w[SK_CORNERS] = 0.4;
            w[SK_FREE_KICKS] = 0.4;
        }
        PosGroup::Midfielder => {
            w[SK_PASSING] = 1.4;
            w[SK_VISION] = 1.3;
            w[SK_STAMINA] = 1.2;
            w[SK_TECHNIQUE] = 1.2;
            w[SK_FIRST_TOUCH] = 1.2;
            w[SK_DECISIONS] = 1.2;
            w[SK_TEAMWORK] = 1.2;
            w[SK_WORK_RATE] = 1.2;
            w[SK_DRIBBLING] = 1.0;
            w[SK_TACKLING] = 1.0;
            w[SK_POSITIONING] = 1.0;
            w[SK_COMPOSURE] = 1.0;
            w[SK_ANTICIPATION] = 1.0;
            w[SK_CONCENTRATION] = 1.0;
            w[SK_PACE] = 1.0;
            w[SK_ACCELERATION] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_BALANCE] = 1.0;
            // Less relevant
            w[SK_HEADING] = 0.6;
            w[SK_LONG_THROWS] = 0.5;
            w[SK_FINISHING] = 0.6;
        }
        PosGroup::Forward => {
            w[SK_FINISHING] = 1.5;
            w[SK_OFF_THE_BALL] = 1.4;
            w[SK_DRIBBLING] = 1.3;
            w[SK_PACE] = 1.2;
            w[SK_COMPOSURE] = 1.2;
            w[SK_FIRST_TOUCH] = 1.2;
            w[SK_ANTICIPATION] = 1.2;
            w[SK_ACCELERATION] = 1.2;
            w[SK_HEADING] = 1.0;
            w[SK_TECHNIQUE] = 1.0;
            w[SK_STRENGTH] = 1.0;
            w[SK_AGILITY] = 1.0;
            w[SK_BALANCE] = 1.0;
            w[SK_DECISIONS] = 1.0;
            w[SK_DETERMINATION] = 1.0;
            w[SK_BRAVERY] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            // Less relevant
            w[SK_TACKLING] = 0.35;
            w[SK_MARKING] = 0.35;
            w[SK_POSITIONING] = 0.5;
            w[SK_CONCENTRATION] = 0.6;
            w[SK_LONG_THROWS] = 0.4;
        }
    }
    w
}

// ── Age curve ───────────────────────────────────────────────────────────
//
// Returns a base development rate per week. Positive = growth, negative =
// decline. The pair is (min_rate, max_rate); the per-tick value is rolled
// uniformly inside that band.
//
// Curve shape:
//   Physical:  rapid growth 16-22 -> plateau 23-27 -> noticeable decline 28-30 -> steep 31+
//   Technical: rapid growth 16-20 -> moderate 21-26 -> plateau 27-29 -> slow decline 30+
//   Mental:    steady growth 16-32 -> very slow decline 33+
//   Goalkeeping: later peak (28-33) and slower decline than outfield categories.

fn base_weekly_rate(age: u8, cat: SkillCategory) -> (f32, f32) {
    match cat {
        SkillCategory::Physical => match age {
            0..=15  => ( 0.010, 0.025),
            16..=17 => ( 0.015, 0.035),
            18..=19 => ( 0.010, 0.025),
            20..=22 => ( 0.006, 0.015),
            23..=27 => ( 0.002, 0.008),
            28..=29 => (-0.003, 0.003),
            30..=31 => (-0.008,-0.001),
            32..=33 => (-0.012,-0.003),
            _       => (-0.018,-0.005),
        },
        SkillCategory::Technical => match age {
            0..=15  => ( 0.025, 0.060),
            16..=17 => ( 0.040, 0.100),
            18..=19 => ( 0.035, 0.080),
            20..=22 => ( 0.020, 0.050),
            23..=26 => ( 0.010, 0.028),
            27..=29 => ( 0.003, 0.012),
            30..=32 => (-0.006, 0.003),
            33..=35 => (-0.012,-0.002),
            _       => (-0.018,-0.004),
        },
        SkillCategory::Mental => match age {
            0..=15  => ( 0.015, 0.040),
            16..=17 => ( 0.025, 0.060),
            18..=19 => ( 0.022, 0.055),
            20..=22 => ( 0.018, 0.045),
            23..=26 => ( 0.012, 0.030),
            27..=29 => ( 0.008, 0.020),
            30..=32 => ( 0.005, 0.015),
            33..=35 => ( 0.002, 0.008),
            _       => (-0.003, 0.003),
        },
        SkillCategory::Goalkeeping => match age {
            0..=15  => ( 0.012, 0.030),
            16..=17 => ( 0.030, 0.070),
            18..=19 => ( 0.025, 0.060),
            20..=22 => ( 0.020, 0.050),
            23..=26 => ( 0.015, 0.035),
            27..=29 => ( 0.010, 0.025),
            30..=33 => ( 0.004, 0.015),
            34..=36 => (-0.002, 0.005),
            _       => (-0.008,-0.001),
        },
    }
}

// ── Personality-driven development multiplier ───────────────────────────

fn personality_multiplier(professionalism: f32, ambition: f32, determination: f32, work_rate: f32) -> f32 {
    let weighted = professionalism * 0.40
        + ambition * 0.25
        + determination * 0.20
        + work_rate * 0.15;
    // Map 0-20 -> 0.4-1.6
    let norm = weighted / 20.0;
    0.4 + norm * 1.2
}

// ── Match-experience multiplier ─────────────────────────────────────────
//
// Counts both official and friendly appearances. Official matches have full
// weight; friendly appearances contribute at only 20% because the competitive
// intensity and development stimulus is much lower. Loaning a young player
// for 30 league games is far more impactful than 30 U20 games.

fn match_experience_multiplier(
    started: u16,
    sub_apps: u16,
    friendly_started: u16,
    friendly_subs: u16,
) -> f32 {
    let official = started as f32 + sub_apps as f32 * 0.4;
    let friendly = (friendly_started as f32 + friendly_subs as f32 * 0.4) * 0.2;
    let effective = official + friendly;
    (0.70 + effective * 0.020).min(1.40)
}

// ── Official match bonus ────────────────────────────────────────────────
//
// Competitive (official league/cup) matches develop players significantly
// faster than friendlies or youth-team games due to higher pressure,
// intensity, and stakes.
//
// Range: 0.75 (only friendlies) -> 1.0 (no games) -> 1.30 (only official)

fn official_match_bonus(official_games: u16, friendly_games: u16) -> f32 {
    let total = official_games + friendly_games;
    if total == 0 {
        return 1.0;
    }
    let official_ratio = official_games as f32 / total as f32;
    0.75 + official_ratio * 0.55
}

// ── Average match rating bonus ──────────────────────────────────────────

fn rating_multiplier(avg_rating: f32, total_games: u16) -> f32 {
    if total_games == 0 {
        return 1.0;
    }
    (1.0 + (avg_rating - 7.0) * 0.10).clamp(0.85, 1.25)
}

// ── Potential gap factor ────────────────────────────────────────────────
//
// Per-skill: how far this skill is from its ceiling. Skills near their
// ceiling barely grow. Skills far below grow fast.

fn skill_gap_factor(current_skill: f32, skill_ceiling: f32) -> f32 {
    if skill_ceiling <= current_skill || skill_ceiling <= 1.0 {
        return 0.05;
    }
    let gap_ratio = (skill_ceiling - current_skill) / skill_ceiling;
    // Sqrt curve: stays high for longer, drops sharply near ceiling.
    (gap_ratio * 2.0).sqrt().clamp(0.1, 1.5)
}

// ── Competition quality multiplier ──────────────────────────────────────
//
// Players in stronger leagues develop faster: better opposition, higher
// tactical demands, greater physical intensity. A player getting 30 apps
// in a semi-pro division grows slower than one getting 30 apps in La Liga.

fn competition_quality_multiplier(league_reputation: u16) -> f32 {
    if league_reputation == 0 {
        return 0.75;
    }
    let normalized = (league_reputation as f32 / 10000.0).clamp(0.0, 1.0);
    (0.70 + normalized * 0.45).clamp(0.70, 1.15)
}

// ── Decline protection ──────────────────────────────────────────────────

fn decline_protection(natural_fitness: f32, professionalism: f32) -> f32 {
    let nf_norm = natural_fitness / 20.0;
    let pr_norm = professionalism / 20.0;
    let protection = nf_norm * 0.50 + pr_norm * 0.50;
    1.0 - protection * 0.50
}

// ── Workload / fatigue / readiness ──────────────────────────────────────

/// State of the player's body for the purposes of weekly development.
/// Drives whether growth happens at all and at what intensity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitnessState {
    /// No injury and no recovery: full development.
    Fit,
    /// Coming back from an injury. No growth, no decline either: the body
    /// is busy healing.
    Recovering,
    /// Currently injured. Skip the development tick entirely.
    Injured,
}

/// Multiplier applied to *growth* rates based on the player's chronic
/// workload state. A drained player learns less even when they show up.
///
/// `condition_pct` is 0..100, `jadedness` is 0..10000.
fn workload_growth_modifier(condition_pct: u32, jadedness: i16) -> f32 {
    let cond = (condition_pct as f32 / 100.0).clamp(0.0, 1.0);
    // Very low condition (<40%) drags hardest; full condition is neutral.
    let cond_mult = (0.55 + cond * 0.45).clamp(0.55, 1.0);

    let jad = (jadedness.max(0) as f32 / 10000.0).clamp(0.0, 1.0);
    // No jadedness = neutral; max jadedness blunts growth ~35%.
    let jad_mult = (1.0 - jad * 0.35).clamp(0.65, 1.0);

    (cond_mult * jad_mult).clamp(0.40, 1.0)
}

/// Multiplier applied to *decline* rates (only used when the per-tick
/// roll is negative). A burned-out player decays a little faster.
fn workload_decline_amplifier(condition_pct: u32, jadedness: i16) -> f32 {
    let cond = (condition_pct as f32 / 100.0).clamp(0.0, 1.0);
    let jad = (jadedness.max(0) as f32 / 10000.0).clamp(0.0, 1.0);
    // Up to +25% decline for chronically tired/jaded players.
    1.0 + (1.0 - cond) * 0.10 + jad * 0.15
}

/// Match readiness (0-20) feeds match-driven growth a small extra push.
/// A player kept match-sharp benefits more from the same minutes than one
/// who's been out of the rhythm.
fn match_readiness_multiplier(match_readiness: f32) -> f32 {
    let mr = (match_readiness / 20.0).clamp(0.0, 1.0);
    // 0 readiness -> 0.90, 20 -> 1.10
    0.90 + mr * 0.20
}

// ── Per-skill peak age offset ───────────────────────────────────────────

fn individual_peak_offset(idx: usize) -> i8 {
    match idx {
        SK_PACE | SK_ACCELERATION => -1,
        SK_AGILITY | SK_BALANCE => -1,
        SK_STRENGTH | SK_JUMPING => 1,
        SK_STAMINA => 0,
        SK_NATURAL_FITNESS => 2,
        SK_LEADERSHIP | SK_COMPOSURE => 3,
        SK_DECISIONS | SK_VISION | SK_POSITIONING => 2,
        SK_ANTICIPATION => 1,
        SK_FLAIR | SK_DRIBBLING => -1,
        // GK: experience-based skills peak later
        SK_GK_COMMAND_OF_AREA | SK_GK_COMMUNICATION => 3,
        SK_GK_ONE_ON_ONES | SK_GK_RUSHING_OUT => 2,
        SK_GK_HANDLING | SK_GK_PUNCHING => 1,
        // GK: reflexes/aerial reach are more physical, peak earlier
        SK_GK_REFLEXES | SK_GK_AERIAL_REACH => -1,
        _ => 0,
    }
}

// ── Flat array helpers ──────────────────────────────────────────────────

fn skills_to_array(player: &Player) -> [f32; SKILL_COUNT] {
    let t = &player.skills.technical;
    let m = &player.skills.mental;
    let p = &player.skills.physical;
    let g = &player.skills.goalkeeping;
    [
        t.corners, t.crossing, t.dribbling, t.finishing, t.first_touch,
        t.free_kicks, t.heading, t.long_shots, t.long_throws, t.marking,
        t.passing, t.penalty_taking, t.tackling, t.technique,
        m.aggression, m.anticipation, m.bravery, m.composure, m.concentration,
        m.decisions, m.determination, m.flair, m.leadership, m.off_the_ball,
        m.positioning, m.teamwork, m.vision, m.work_rate,
        p.acceleration, p.agility, p.balance, p.jumping, p.natural_fitness,
        p.pace, p.stamina, p.strength, p.match_readiness,
        g.aerial_reach, g.command_of_area, g.communication, g.eccentricity,
        g.first_touch, g.handling, g.kicking, g.one_on_ones, g.passing,
        g.punching, g.reflexes, g.rushing_out, g.throwing,
    ]
}

fn write_skills_back(player: &mut Player, arr: &[f32; SKILL_COUNT]) {
    let t = &mut player.skills.technical;
    t.corners = arr[SK_CORNERS]; t.crossing = arr[SK_CROSSING];
    t.dribbling = arr[SK_DRIBBLING]; t.finishing = arr[SK_FINISHING];
    t.first_touch = arr[SK_FIRST_TOUCH]; t.free_kicks = arr[SK_FREE_KICKS];
    t.heading = arr[SK_HEADING]; t.long_shots = arr[SK_LONG_SHOTS];
    t.long_throws = arr[SK_LONG_THROWS]; t.marking = arr[SK_MARKING];
    t.passing = arr[SK_PASSING]; t.penalty_taking = arr[SK_PENALTY_TAKING];
    t.tackling = arr[SK_TACKLING]; t.technique = arr[SK_TECHNIQUE];

    let m = &mut player.skills.mental;
    m.aggression = arr[SK_AGGRESSION]; m.anticipation = arr[SK_ANTICIPATION];
    m.bravery = arr[SK_BRAVERY]; m.composure = arr[SK_COMPOSURE];
    m.concentration = arr[SK_CONCENTRATION]; m.decisions = arr[SK_DECISIONS];
    m.determination = arr[SK_DETERMINATION]; m.flair = arr[SK_FLAIR];
    m.leadership = arr[SK_LEADERSHIP]; m.off_the_ball = arr[SK_OFF_THE_BALL];
    m.positioning = arr[SK_POSITIONING]; m.teamwork = arr[SK_TEAMWORK];
    m.vision = arr[SK_VISION]; m.work_rate = arr[SK_WORK_RATE];

    let p = &mut player.skills.physical;
    p.acceleration = arr[SK_ACCELERATION]; p.agility = arr[SK_AGILITY];
    p.balance = arr[SK_BALANCE]; p.jumping = arr[SK_JUMPING];
    p.natural_fitness = arr[SK_NATURAL_FITNESS]; p.pace = arr[SK_PACE];
    p.stamina = arr[SK_STAMINA]; p.strength = arr[SK_STRENGTH];
    p.match_readiness = arr[SK_MATCH_READINESS];

    let g = &mut player.skills.goalkeeping;
    g.aerial_reach = arr[SK_GK_AERIAL_REACH]; g.command_of_area = arr[SK_GK_COMMAND_OF_AREA];
    g.communication = arr[SK_GK_COMMUNICATION]; g.eccentricity = arr[SK_GK_ECCENTRICITY];
    g.first_touch = arr[SK_GK_FIRST_TOUCH]; g.handling = arr[SK_GK_HANDLING];
    g.kicking = arr[SK_GK_KICKING]; g.one_on_ones = arr[SK_GK_ONE_ON_ONES];
    g.passing = arr[SK_GK_PASSING]; g.punching = arr[SK_GK_PUNCHING];
    g.reflexes = arr[SK_GK_REFLEXES]; g.rushing_out = arr[SK_GK_RUSHING_OUT];
    g.throwing = arr[SK_GK_THROWING];
}

// ── Deterministic roll source ───────────────────────────────────────────
//
// Production code uses `ThreadRolls` which forwards to `rand::random()`.
// Tests use `FixedRolls` (every roll returns the same value) or
// `SeqRolls` (returns a scripted sequence) so a development tick produces
// stable, inspectable output.

/// Source of uniform random numbers in `[0.0, 1.0)`. Implementations are
/// expected to be cheap and stateful (each call advances the stream).
pub trait RollSource {
    fn roll_unit(&mut self) -> f32;
}

/// Default production roll source backed by the thread-local RNG.
pub struct ThreadRolls;

impl RollSource for ThreadRolls {
    #[inline]
    fn roll_unit(&mut self) -> f32 { rand::random::<f32>() }
}

/// Roll source that returns the same value on every call. Useful when
/// tests want to pin the per-skill roll to either the lower or upper
/// edge of the age-curve band.
#[derive(Debug, Clone, Copy)]
pub struct FixedRolls(pub f32);

impl RollSource for FixedRolls {
    #[inline]
    fn roll_unit(&mut self) -> f32 { self.0 }
}

// ═══════════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════════

/// Per-category coach training effectiveness, normalized to a multiplier
/// centered on ~1.0. A bad coach (average attribute 5/20) produces ~0.75;
/// an elite coach (18/20) produces ~1.35. For players under 23, the club's
/// best `working_with_youngsters` attribute adds a further +0-15% bonus.
#[derive(Debug, Clone, Copy)]
pub struct CoachingEffect {
    pub technical: f32,
    pub mental: f32,
    pub physical: f32,
    pub goalkeeping: f32,
    /// Bonus multiplier applied on top of the category multiplier for
    /// players under 23.
    pub youth_bonus: f32,
}

impl CoachingEffect {
    pub fn neutral() -> Self {
        Self {
            technical: 1.0,
            mental: 1.0,
            physical: 1.0,
            goalkeeping: 1.0,
            youth_bonus: 1.0,
        }
    }

    /// Build from the best coach attribute found at the club (0-20 scale)
    /// and the youth coaching quality (0.0-1.0 normalized).
    pub fn from_scores(
        technical: u8,
        mental: u8,
        fitness: u8,
        goalkeeping: u8,
        youth_quality_0_1: f32,
    ) -> Self {
        let m = |attr: u8| -> f32 {
            // 0 -> 0.60, 10 -> 1.0, 20 -> 1.40 (linear)
            (0.6 + (attr as f32 / 20.0) * 0.8).clamp(0.55, 1.45)
        };
        Self {
            technical: m(technical),
            mental: m(mental),
            physical: m(fitness),
            goalkeeping: m(goalkeeping),
            youth_bonus: (1.0 + youth_quality_0_1 * 0.15).clamp(1.0, 1.18),
        }
    }

    fn for_category(&self, cat: SkillCategory) -> f32 {
        match cat {
            SkillCategory::Technical => self.technical,
            SkillCategory::Mental => self.mental,
            SkillCategory::Physical => self.physical,
            SkillCategory::Goalkeeping => self.goalkeeping,
        }
    }
}

impl Player {
    /// Weekly development tick. See module docs for the model.
    ///
    /// Routes through the deterministic roll seam under the hood, using
    /// the thread-local RNG. Tests should call
    /// [`Player::process_development_with`] with a deterministic source.
    pub fn process_development(
        &mut self,
        now: NaiveDate,
        league_reputation: u16,
        coach: &CoachingEffect,
        club_rep_0_to_1: f32,
    ) {
        self.process_development_with(now, league_reputation, coach, club_rep_0_to_1, &mut ThreadRolls);
    }

    /// Same as [`process_development`] but the per-skill rolls come from
    /// `rolls`. This is the testable seam — pin the rolls to a known
    /// value and the output becomes a pure function of the inputs.
    pub fn process_development_with(
        &mut self,
        now: NaiveDate,
        league_reputation: u16,
        coach: &CoachingEffect,
        club_rep_0_to_1: f32,
        rolls: &mut impl RollSource,
    ) {
        let age = DateUtils::age(self.birth_date, now);
        let pa = self.player_attributes.potential_ability as f32;

        // Body state gates everything else.
        let fitness = if self.player_attributes.is_injured {
            FitnessState::Injured
        } else if self.player_attributes.is_in_recovery() {
            FitnessState::Recovering
        } else {
            FitnessState::Fit
        };

        // Injured players don't develop. Their skills are frozen until they
        // come back — no growth, no decline. The CA recalculation is also
        // skipped because the underlying skills haven't moved.
        if fitness == FitnessState::Injured {
            return;
        }

        let pos = self.position();
        let pos_group = pos_group_from(pos);
        let dev_weights = position_dev_weights(pos_group);

        // Base ceiling from PA (PA 200 -> ceiling 20.0)
        let base_ceiling = (pa / 200.0 * 20.0).clamp(1.0, 20.0);

        // ── Compute shared multipliers ────────────────────────────────

        let personality = personality_multiplier(
            self.attributes.professionalism,
            self.attributes.ambition,
            self.skills.mental.determination,
            self.skills.mental.work_rate,
        );

        let official_games = self.statistics.total_games() + self.cup_statistics.total_games();
        let friendly_games = self.friendly_statistics.total_games();

        let match_exp = match_experience_multiplier(
            self.statistics.played + self.cup_statistics.played,
            self.statistics.played_subs + self.cup_statistics.played_subs,
            self.friendly_statistics.played,
            self.friendly_statistics.played_subs,
        );

        let official_bonus = official_match_bonus(official_games, friendly_games);

        let rating_mult = rating_multiplier(self.statistics.average_rating, official_games);

        let decline_prot = decline_protection(
            self.skills.physical.natural_fitness,
            self.attributes.professionalism,
        );

        let comp_quality = competition_quality_multiplier(league_reputation);

        // Extra boost while the player catches up to a clearly better club.
        let step_up_mult = self.step_up_development_multiplier(now, club_rep_0_to_1);

        // Workload / fitness / readiness modifiers.
        let condition_pct = self.player_attributes.condition_percentage();
        let jadedness = self.player_attributes.jadedness;
        let workload_growth = workload_growth_modifier(condition_pct, jadedness);
        let workload_decline = workload_decline_amplifier(condition_pct, jadedness);
        let readiness_mult = match_readiness_multiplier(self.skills.physical.match_readiness);

        // Recovering from an injury: the body is healing, not adapting.
        // Mental skills (study video, learn the playbook) can still nudge
        // forward at a reduced rate; everything else is frozen.
        let recovering = fitness == FitnessState::Recovering;

        // ── Process each skill ────────────────────────────────────────

        let mut skills = skills_to_array(self);

        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS {
                continue; // managed by training/match system
            }

            let cat = skill_category(i);

            if recovering && cat != SkillCategory::Mental {
                continue;
            }

            let peak_offset = individual_peak_offset(i);
            let effective_age = (age as i16 - peak_offset as i16).clamp(14, 45) as u8;

            // Per-skill ceiling: position weight determines how high this skill can go.
            let skill_ceiling = (base_ceiling * dev_weights[i]).clamp(1.0, 20.0);

            // Per-skill gap factor (replaces global PA-CA gap).
            let gap = skill_gap_factor(skills[i], skill_ceiling);

            // Base rate from age curve.
            let (min_rate, max_rate) = base_weekly_rate(effective_age, cat);
            let roll = rolls.roll_unit().clamp(0.0, 1.0);
            let base = min_rate + roll * (max_rate - min_rate);

            // Position weight scales growth rate: key skills develop faster.
            let pos_rate_mult = dev_weights[i];

            // Coach effectiveness by category, plus a youth bonus for
            // players under 23 (using Head of Youth Development attribute).
            let coach_mult = coach.for_category(cat);
            let youth_coach_mult = if age < 23 { coach.youth_bonus } else { 1.0 };

            let change = if base > 0.0 {
                // Growth: scale by all positive multipliers + position relevance + competition quality
                base * personality
                    * match_exp
                    * official_bonus
                    * rating_mult
                    * gap
                    * pos_rate_mult
                    * comp_quality
                    * coach_mult
                    * youth_coach_mult
                    * step_up_mult
                    * workload_growth
                    * readiness_mult
            } else {
                // Decline: position-irrelevant skills decline slightly faster;
                // key skills are more "maintained" by regular use. Great
                // coaches slow decline a little (load + technique management).
                // Workload amplifier accelerates decline for chronically tired
                // players.
                let decline_pos_mult = (2.0 - dev_weights[i]).clamp(0.5, 1.5);
                let decline_coach_protection = ((coach_mult - 1.0) * 0.5 + 1.0).clamp(0.6, 1.0);
                base * decline_prot * decline_pos_mult * decline_coach_protection * workload_decline
            };

            let new_val = skills[i] + change;

            skills[i] = if change > 0.0 {
                new_val.min(skill_ceiling).clamp(1.0, 20.0)
            } else {
                new_val.clamp(1.0, 20.0)
            };
        }

        write_skills_back(self, &skills);

        // ── Recalculate current_ability from updated skills ───────────

        let position = self.position();
        self.player_attributes.current_ability =
            self.skills.calculate_ability_for_position(position);

        // PA must never be lower than CA. Generation can occasionally produce
        // CA > PA, which would otherwise crush all per-skill ceilings.
        if self.player_attributes.potential_ability < self.player_attributes.current_ability {
            self.player_attributes.potential_ability = self.player_attributes.current_ability;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::position::{PlayerPosition, PlayerPositions};
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerSkills};
    use chrono::NaiveDate;

    // ── Test helpers ──────────────────────────────────────────────────

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn person_pro(prof: f32, ambition: f32) -> PersonAttributes {
        PersonAttributes {
            professionalism: prof,
            ambition,
            ..PersonAttributes::default()
        }
    }

    fn baseline_skills() -> PlayerSkills {
        // Mid-range outfield baseline — meaningfully below the per-skill
        // ceiling of a high-PA player so growth has room to happen.
        let mut s = PlayerSkills::default();
        s.technical.passing = 10.0;
        s.technical.first_touch = 10.0;
        s.technical.dribbling = 10.0;
        s.technical.finishing = 10.0;
        s.technical.tackling = 10.0;
        s.technical.marking = 10.0;
        s.technical.heading = 10.0;
        s.technical.technique = 10.0;
        s.technical.crossing = 10.0;
        s.technical.long_shots = 10.0;
        s.technical.long_throws = 10.0;
        s.technical.corners = 10.0;
        s.technical.free_kicks = 10.0;
        s.technical.penalty_taking = 10.0;

        s.mental.work_rate = 14.0;
        s.mental.determination = 14.0;
        s.mental.composure = 10.0;
        s.mental.decisions = 10.0;
        s.mental.positioning = 10.0;
        s.mental.anticipation = 10.0;
        s.mental.vision = 10.0;
        s.mental.teamwork = 10.0;
        s.mental.concentration = 10.0;
        s.mental.bravery = 10.0;
        s.mental.aggression = 10.0;
        s.mental.flair = 10.0;
        s.mental.leadership = 10.0;
        s.mental.off_the_ball = 10.0;

        s.physical.pace = 12.0;
        s.physical.acceleration = 12.0;
        s.physical.agility = 12.0;
        s.physical.balance = 12.0;
        s.physical.stamina = 12.0;
        s.physical.strength = 12.0;
        s.physical.jumping = 12.0;
        s.physical.natural_fitness = 14.0;
        s.physical.match_readiness = 15.0;

        s
    }

    fn gk_skills() -> PlayerSkills {
        let mut s = baseline_skills();
        // Give GK a meaningful goalkeeping baseline.
        s.goalkeeping.handling = 10.0;
        s.goalkeeping.reflexes = 10.0;
        s.goalkeeping.aerial_reach = 10.0;
        s.goalkeeping.one_on_ones = 10.0;
        s.goalkeeping.command_of_area = 10.0;
        s.goalkeeping.communication = 10.0;
        s.goalkeeping.kicking = 10.0;
        s.goalkeeping.first_touch = 10.0;
        s.goalkeeping.passing = 10.0;
        s.goalkeeping.throwing = 10.0;
        s.goalkeeping.punching = 10.0;
        s.goalkeeping.rushing_out = 10.0;
        s.goalkeeping.eccentricity = 10.0;
        s
    }

    fn positions(p: PlayerPositionType) -> PlayerPositions {
        PlayerPositions {
            positions: vec![PlayerPosition { position: p, level: 20 }],
        }
    }

    fn make_player(
        birth: NaiveDate,
        pos: PlayerPositionType,
        skills: PlayerSkills,
        pa: u8,
        person: PersonAttributes,
    ) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.potential_ability = pa;
        // Start CA below PA so per-skill growth has room.
        attrs.current_ability = (pa as f32 * 0.5) as u8;
        attrs.condition = 9500;
        attrs.jadedness = 1000;
        attrs.injury_proneness = 5;

        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person)
            .skills(skills)
            .positions(positions(pos))
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    // ── Round-trip ────────────────────────────────────────────────────

    #[test]
    fn skill_array_round_trip_preserves_all_fields() {
        let mut p = make_player(
            d(2000, 1, 1),
            PlayerPositionType::Striker,
            baseline_skills(),
            150,
            PersonAttributes::default(),
        );
        // Stamp every skill with a unique value so a missing field shows up
        // as a stale default rather than a coincidence.
        let mut tagged = [0.0f32; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            tagged[i] = 1.0 + (i as f32) * 0.1; // 1.0, 1.1, 1.2, ... 5.9
        }
        write_skills_back(&mut p, &tagged);

        let round_tripped = skills_to_array(&p);
        for i in 0..SKILL_COUNT {
            assert!(
                (round_tripped[i] - tagged[i]).abs() < 1e-6,
                "skill index {} did not round-trip: wrote {}, read {}",
                i, tagged[i], round_tripped[i]
            );
        }
    }

    #[test]
    fn skill_count_matches_enum() {
        assert_eq!(SkillKey::GkThrowing as usize + 1, SKILL_COUNT);
    }

    // ── Pure helper checks ────────────────────────────────────────────

    #[test]
    fn workload_growth_modifier_drops_with_fatigue() {
        let fresh = workload_growth_modifier(100, 0);
        let drained = workload_growth_modifier(35, 8000);
        assert!(fresh > drained, "fresh {} should exceed drained {}", fresh, drained);
        assert!(drained <= 0.7);
        assert!(fresh >= 0.99);
    }

    #[test]
    fn match_readiness_multiplier_scales_inside_band() {
        assert!(match_readiness_multiplier(0.0) < match_readiness_multiplier(20.0));
        assert!((match_readiness_multiplier(20.0) - 1.10).abs() < 1e-4);
        assert!((match_readiness_multiplier(0.0) - 0.90).abs() < 1e-4);
    }

    #[test]
    fn skill_gap_factor_is_zero_at_or_above_ceiling() {
        assert_eq!(skill_gap_factor(20.0, 15.0), 0.05);
        assert_eq!(skill_gap_factor(15.0, 15.0), 0.05);
        assert!(skill_gap_factor(5.0, 15.0) > 0.5);
    }

    #[test]
    fn defensive_midfielder_uses_midfielder_dev_weights() {
        // This pins the deliberate divergence from
        // PlayerPositionType::position_group(): for development the DM is
        // a midfielder, not a defender.
        assert_eq!(
            pos_group_from(PlayerPositionType::DefensiveMidfielder),
            PosGroup::Midfielder
        );
    }

    // ── Position-specific ceilings ────────────────────────────────────

    #[test]
    fn striker_finishing_ceiling_exceeds_tackling_ceiling() {
        // Verify the position weights produce the expected ceiling shape.
        let w = position_dev_weights(PosGroup::Forward);
        assert!(w[SK_FINISHING] > w[SK_TACKLING]);
        assert!(w[SK_FINISHING] >= 1.4);
        assert!(w[SK_TACKLING] <= 0.5);
    }

    #[test]
    fn defender_marking_grows_faster_than_finishing() {
        let w = position_dev_weights(PosGroup::Defender);
        assert!(w[SK_MARKING] > w[SK_FINISHING]);
        assert!(w[SK_TACKLING] > w[SK_FINISHING]);
    }

    // ── Behavioral tests using the deterministic roll seam ────────────

    fn high_roll() -> FixedRolls { FixedRolls(1.0) }

    #[test]
    fn young_professional_grows_more_than_low_professionalism_peer() {
        let birth = d(2008, 1, 1); // age ~17 on 2025-06-01
        let now = d(2025, 6, 1);
        let pa = 170u8;

        let mut pro = make_player(
            birth, PlayerPositionType::Striker, baseline_skills(), pa,
            person_pro(18.0, 16.0),
        );
        let mut sloth = make_player(
            birth, PlayerPositionType::Striker, baseline_skills(), pa,
            person_pro(4.0, 6.0),
        );
        // Same starting CA so the gap factor is identical.
        sloth.skills.mental.work_rate = 6.0;
        sloth.skills.mental.determination = 6.0;

        let pre_pro_finishing = pro.skills.technical.finishing;
        let pre_sloth_finishing = sloth.skills.technical.finishing;

        let coach = CoachingEffect::neutral();
        pro.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());
        sloth.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());

        let pro_gain = pro.skills.technical.finishing - pre_pro_finishing;
        let sloth_gain = sloth.skills.technical.finishing - pre_sloth_finishing;
        assert!(
            pro_gain > sloth_gain,
            "pro_gain={}, sloth_gain={}",
            pro_gain, sloth_gain
        );
    }

    #[test]
    fn old_player_declines_physically_but_can_still_grow_mentally() {
        // 36-year-old with neutral coaching, same fixed roll for both
        // categories. Mental should still nudge up, physical should fall.
        let now = d(2025, 6, 1);
        let birth = d(1989, 1, 1); // 36
        let mut p = make_player(
            birth,
            PlayerPositionType::DefenderCenter,
            baseline_skills(),
            150,
            person_pro(15.0, 12.0),
        );
        let pre_pace = p.skills.physical.pace;
        let pre_leadership = p.skills.mental.leadership;

        let coach = CoachingEffect::neutral();
        // Use the midpoint roll so the band is interpreted at its center —
        // physical at 36 is unambiguously negative; mental at 36 is around 0.
        p.process_development_with(now, 5000, &coach, 0.5, &mut FixedRolls(0.5));

        assert!(
            p.skills.physical.pace <= pre_pace,
            "old pace should not grow: pre={}, post={}",
            pre_pace, p.skills.physical.pace
        );
        // Leadership has a +3 peak offset so a 36yo is effectively 33 for it.
        assert!(
            p.skills.mental.leadership >= pre_leadership,
            "leadership should hold or grow: pre={}, post={}",
            pre_leadership, p.skills.mental.leadership
        );
    }

    #[test]
    fn injured_player_skips_development_entirely() {
        let mut p = make_player(
            d(2008, 1, 1),
            PlayerPositionType::Striker,
            baseline_skills(),
            170,
            person_pro(18.0, 16.0),
        );
        p.player_attributes.is_injured = true;
        p.player_attributes.injury_days_remaining = 30;

        let snapshot = skills_to_array(&p);
        let coach = CoachingEffect::neutral();
        p.process_development_with(d(2025, 6, 1), 8000, &coach, 0.6, &mut high_roll());

        let after = skills_to_array(&p);
        for i in 0..SKILL_COUNT {
            assert!(
                (snapshot[i] - after[i]).abs() < 1e-6,
                "injured player skill {} changed: {} -> {}",
                i, snapshot[i], after[i]
            );
        }
    }

    #[test]
    fn recovering_player_only_gains_mental() {
        let mut p = make_player(
            d(2008, 1, 1),
            PlayerPositionType::Striker,
            baseline_skills(),
            170,
            person_pro(18.0, 16.0),
        );
        p.player_attributes.recovery_days_remaining = 14;
        // is_injured is already false (Default), so this puts the player in
        // the recovery phase.

        let pre_finishing = p.skills.technical.finishing;
        let pre_pace = p.skills.physical.pace;
        let pre_decisions = p.skills.mental.decisions;

        let coach = CoachingEffect::neutral();
        p.process_development_with(d(2025, 6, 1), 8000, &coach, 0.6, &mut high_roll());

        assert_eq!(p.skills.technical.finishing, pre_finishing,
            "recovering player should not gain technical");
        assert_eq!(p.skills.physical.pace, pre_pace,
            "recovering player should not gain physical");
        assert!(p.skills.mental.decisions >= pre_decisions,
            "recovering player can still gain mental");
    }

    #[test]
    fn fatigued_jaded_player_grows_less_than_fresh_peer() {
        let birth = d(2006, 1, 1); // ~19yo
        let now = d(2025, 6, 1);
        let pa = 170u8;

        let mut fresh = make_player(
            birth, PlayerPositionType::Striker, baseline_skills(), pa,
            person_pro(15.0, 14.0),
        );
        let mut drained = make_player(
            birth, PlayerPositionType::Striker, baseline_skills(), pa,
            person_pro(15.0, 14.0),
        );
        drained.player_attributes.condition = 3500;
        drained.player_attributes.jadedness = 8000;
        drained.skills.physical.match_readiness = 5.0;

        let pre_fresh = fresh.skills.technical.finishing;
        let pre_drained = drained.skills.technical.finishing;

        let coach = CoachingEffect::neutral();
        fresh.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());
        drained.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());

        let fresh_gain = fresh.skills.technical.finishing - pre_fresh;
        let drained_gain = drained.skills.technical.finishing - pre_drained;
        assert!(
            fresh_gain > drained_gain,
            "fresh {} should grow more than drained {}",
            fresh_gain, drained_gain
        );
    }

    #[test]
    fn deterministic_seeded_rolls_produce_stable_output() {
        let now = d(2025, 6, 1);
        let coach = CoachingEffect::neutral();
        let mut p1 = make_player(
            d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
            baseline_skills(), 160, person_pro(15.0, 12.0),
        );
        let mut p2 = make_player(
            d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
            baseline_skills(), 160, person_pro(15.0, 12.0),
        );

        p1.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));
        p2.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));

        let a = skills_to_array(&p1);
        let b = skills_to_array(&p2);
        for i in 0..SKILL_COUNT {
            assert!(
                (a[i] - b[i]).abs() < 1e-6,
                "deterministic skill {} differed: {} vs {}",
                i, a[i], b[i]
            );
        }
    }

    #[test]
    fn goalkeeping_skills_use_later_peak_curve() {
        // At age 30, a GK should still gain on goalkeeping skills, while
        // an outfield 30yo's physical skills are flat or declining.
        let coach = CoachingEffect::neutral();
        let now = d(2025, 6, 1);

        let mut gk = make_player(
            d(1995, 1, 1), // 30
            PlayerPositionType::Goalkeeper,
            gk_skills(),
            160,
            person_pro(15.0, 12.0),
        );
        let pre_handling = gk.skills.goalkeeping.handling;
        gk.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.7));
        assert!(
            gk.skills.goalkeeping.handling > pre_handling,
            "30yo GK handling should still grow: pre={} post={}",
            pre_handling, gk.skills.goalkeeping.handling
        );

        let mut out = make_player(
            d(1995, 1, 1), // 30
            PlayerPositionType::Striker,
            baseline_skills(),
            160,
            person_pro(15.0, 12.0),
        );
        let pre_pace = out.skills.physical.pace;
        out.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));
        assert!(
            out.skills.physical.pace <= pre_pace,
            "30yo outfield pace should not grow: pre={} post={}",
            pre_pace, out.skills.physical.pace
        );
    }

    #[test]
    fn coaching_effect_amplifies_growth() {
        let now = d(2025, 6, 1);
        let mut weak = make_player(
            d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
            baseline_skills(), 160, person_pro(15.0, 12.0),
        );
        let mut strong = make_player(
            d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
            baseline_skills(), 160, person_pro(15.0, 12.0),
        );

        let no_coach = CoachingEffect::neutral();
        let elite = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

        let pre_weak = weak.skills.technical.passing;
        let pre_strong = strong.skills.technical.passing;

        weak.process_development_with(now, 6000, &no_coach, 0.4, &mut high_roll());
        strong.process_development_with(now, 6000, &elite, 0.4, &mut high_roll());

        let weak_gain = weak.skills.technical.passing - pre_weak;
        let strong_gain = strong.skills.technical.passing - pre_strong;
        assert!(
            strong_gain > weak_gain,
            "elite coach gain {} should exceed neutral coach gain {}",
            strong_gain, weak_gain
        );
    }
}
