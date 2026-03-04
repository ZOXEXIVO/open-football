//! Player skill development system modeled after Football Manager (SI Games).
//!
//! Key principles:
//! 1. **Position-aware** — skills relevant to the player's position develop faster
//!    and have a higher ceiling. Irrelevant skills stay low.
//! 2. **Age curve** — physical skills peak 24-28, decline from ~30; mental skills
//!    can grow into the 30s; technical skills plateau in the late 20s.
//! 3. **Personality** — professionalism, ambition, determination drive growth rate.
//! 4. **Match experience** — playing competitive matches accelerates development.
//! 5. **Potential ceiling** — PA gates maximum achievable level; per-skill ceilings
//!    based on PA × position weight create realistic skill profiles.

use crate::club::player::player::Player;
use crate::utils::DateUtils;
use crate::PlayerPositionType;
use chrono::NaiveDate;

// ── Skill indices (flat [f32; 37] layout) ───────────────────────────────

const SKILL_COUNT: usize = 37;

// Technical 0..14
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
// Mental 14..28
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
// Physical 28..37
const SK_ACCELERATION: usize = 28;
const SK_AGILITY: usize = 29;
const SK_BALANCE: usize = 30;
const SK_JUMPING: usize = 31;
const SK_NATURAL_FITNESS: usize = 32;
const SK_PACE: usize = 33;
const SK_STAMINA: usize = 34;
const SK_STRENGTH: usize = 35;
const SK_MATCH_READINESS: usize = 36;

// ── Skill category ──────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum SkillCategory {
    Technical,
    Mental,
    Physical,
}

fn skill_category(idx: usize) -> SkillCategory {
    match idx {
        SK_ACCELERATION | SK_AGILITY | SK_BALANCE | SK_JUMPING | SK_NATURAL_FITNESS | SK_PACE
        | SK_STAMINA | SK_STRENGTH | SK_MATCH_READINESS => SkillCategory::Physical,
        SK_AGGRESSION | SK_ANTICIPATION | SK_BRAVERY | SK_COMPOSURE | SK_CONCENTRATION
        | SK_DECISIONS | SK_DETERMINATION | SK_FLAIR | SK_LEADERSHIP | SK_OFF_THE_BALL
        | SK_POSITIONING | SK_TEAMWORK | SK_VISION | SK_WORK_RATE => SkillCategory::Mental,
        _ => SkillCategory::Technical,
    }
}

// ── Position group for development weights ──────────────────────────────

#[derive(Copy, Clone)]
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
// 1. Per-skill CEILING = base_ceiling * weight  (so key skills can reach high, irrelevant stay low)
// 2. Per-skill GROWTH RATE multiplier (key skills develop faster)
//
// Range: 0.3 (irrelevant) to 1.5 (core skill)
// Default: 0.8 for unspecified skills

fn position_dev_weights(group: PosGroup) -> [f32; SKILL_COUNT] {
    let mut w = [0.8f32; SKILL_COUNT];
    match group {
        PosGroup::Goalkeeper => {
            // Core GK skills — high ceiling and fast development
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
            // Irrelevant outfield skills — low ceiling, barely develop
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

// ── FM-style age curve ──────────────────────────────────────────────────
//
// Returns a *base development rate* per week.  Positive = growth, negative = decline.
//
// FM curve shape:
//   Physical:  rapid growth 16-22 → plateau 23-27 → noticeable decline 28-30 → steep 31+
//   Technical: rapid growth 16-20 → moderate 21-26 → plateau 27-29 → slow decline 30+
//   Mental:    steady growth 16-32 → very slow decline 33+

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
    }
}

// ── Personality-driven development multiplier ───────────────────────────

fn personality_multiplier(professionalism: f32, ambition: f32, determination: f32, work_rate: f32) -> f32 {
    let weighted = professionalism * 0.40
        + ambition * 0.25
        + determination * 0.20
        + work_rate * 0.15;
    // Map 0-20 → 0.4-1.6
    let norm = weighted / 20.0;
    0.4 + norm * 1.2
}

// ── Match-experience multiplier ─────────────────────────────────────────
//
// Counts both official and friendly appearances. Official matches have full
// weight; friendly appearances contribute at 30% because the competitive
// intensity and development stimulus is lower.

fn match_experience_multiplier(
    started: u16,
    sub_apps: u16,
    friendly_started: u16,
    friendly_subs: u16,
) -> f32 {
    let official = started as f32 + sub_apps as f32 * 0.4;
    let friendly = (friendly_started as f32 + friendly_subs as f32 * 0.4) * 0.3;
    let effective = official + friendly;
    (0.70 + effective * 0.020).min(1.40)
}

// ── Official match bonus ────────────────────────────────────────────────
//
// Competitive (official) matches develop players faster than friendlies due
// to higher pressure, intensity, and stakes. This multiplier rewards players
// who get regular official match time.
//
// Range: 0.90 (only friendlies) → 1.0 (no games) → 1.15 (only official)

fn official_match_bonus(official_games: u16, friendly_games: u16) -> f32 {
    let total = official_games + friendly_games;
    if total == 0 {
        return 1.0;
    }
    let official_ratio = official_games as f32 / total as f32;
    0.90 + official_ratio * 0.25
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
// Per-skill: measures how far THIS skill is from ITS ceiling.
// Skills near their ceiling barely grow. Skills far below grow fast.

fn skill_gap_factor(current_skill: f32, skill_ceiling: f32) -> f32 {
    if skill_ceiling <= current_skill || skill_ceiling <= 1.0 {
        return 0.05; // at or above ceiling
    }
    let gap_ratio = (skill_ceiling - current_skill) / skill_ceiling;
    // Sqrt curve: stays high for longer, drops sharply near ceiling
    (gap_ratio * 2.0).sqrt().clamp(0.1, 1.5)
}

// ── Decline protection ──────────────────────────────────────────────────

fn decline_protection(natural_fitness: f32, professionalism: f32) -> f32 {
    let nf_norm = natural_fitness / 20.0;
    let pr_norm = professionalism / 20.0;
    let protection = nf_norm * 0.50 + pr_norm * 0.50;
    1.0 - protection * 0.50
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
        _ => 0,
    }
}

// ── Flat array helpers ──────────────────────────────────────────────────

fn skills_to_array(player: &Player) -> [f32; SKILL_COUNT] {
    let t = &player.skills.technical;
    let m = &player.skills.mental;
    let p = &player.skills.physical;
    [
        t.corners, t.crossing, t.dribbling, t.finishing, t.first_touch,
        t.free_kicks, t.heading, t.long_shots, t.long_throws, t.marking,
        t.passing, t.penalty_taking, t.tackling, t.technique,
        m.aggression, m.anticipation, m.bravery, m.composure, m.concentration,
        m.decisions, m.determination, m.flair, m.leadership, m.off_the_ball,
        m.positioning, m.teamwork, m.vision, m.work_rate,
        p.acceleration, p.agility, p.balance, p.jumping, p.natural_fitness,
        p.pace, p.stamina, p.strength, p.match_readiness,
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
}

// ═══════════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════════

impl Player {
    /// FM-style weekly development tick.
    ///
    /// Key difference from naive approach: each skill has its OWN ceiling and
    /// growth rate based on position weights. A striker's finishing develops fast
    /// toward a high ceiling while their tackling barely moves.
    pub fn process_development(&mut self, now: NaiveDate) {
        let age = DateUtils::age(self.birth_date, now);
        let pa = self.player_attributes.potential_ability as f32;

        // Position-based development weights
        let pos = self.position();
        let pos_group = pos_group_from(pos);
        let dev_weights = position_dev_weights(pos_group);

        // Base ceiling from PA (PA 200 → ceiling 20.0)
        let base_ceiling = (pa / 200.0 * 20.0).clamp(1.0, 20.0);

        // ── Compute shared multipliers ────────────────────────────────

        let personality = personality_multiplier(
            self.attributes.professionalism,
            self.attributes.ambition,
            self.skills.mental.determination,
            self.skills.mental.work_rate,
        );

        let official_games = self.statistics.total_games();
        let friendly_games = self.friendly_statistics.total_games();

        let match_exp = match_experience_multiplier(
            self.statistics.played,
            self.statistics.played_subs,
            self.friendly_statistics.played,
            self.friendly_statistics.played_subs,
        );

        let official_bonus = official_match_bonus(official_games, friendly_games);

        let rating_mult = rating_multiplier(self.statistics.average_rating, official_games);

        let decline_prot = decline_protection(
            self.skills.physical.natural_fitness,
            self.attributes.professionalism,
        );

        // ── Process each skill ────────────────────────────────────────

        let mut skills = skills_to_array(self);

        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS {
                continue; // managed by training/match system
            }

            let cat = skill_category(i);
            let peak_offset = individual_peak_offset(i);
            let effective_age = (age as i16 - peak_offset as i16).clamp(14, 45) as u8;

            // Per-skill ceiling: position weight determines how high this skill can go
            let skill_ceiling = (base_ceiling * dev_weights[i]).clamp(1.0, 20.0);

            // Per-skill gap factor (replaces global PA-CA gap)
            let gap = skill_gap_factor(skills[i], skill_ceiling);

            // Base rate from age curve
            let (min_rate, max_rate) = base_weekly_rate(effective_age, cat);
            let base = min_rate + rand::random::<f32>() * (max_rate - min_rate);

            // Position weight also scales growth rate: key skills develop faster
            let pos_rate_mult = dev_weights[i];

            let change = if base > 0.0 {
                // Growth: scale by all positive multipliers + position relevance
                base * personality * match_exp * official_bonus * rating_mult * gap * pos_rate_mult
            } else {
                // Decline: position-irrelevant skills decline slightly faster
                // Key skills are more "maintained" by regular use
                let decline_pos_mult = (2.0 - dev_weights[i]).clamp(0.5, 1.5);
                base * decline_prot * decline_pos_mult
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
    }
}
