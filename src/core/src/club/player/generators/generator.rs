use crate::shared::FullName;
use crate::utils::IntegerUtils;
use crate::{
    Mental, PeopleNameGeneratorData, PersonAttributes, PersonBehaviour, PersonBehaviourState,
    Physical, Player, PlayerAttributes, PlayerClubContract, PlayerDecisionHistory, PlayerHappiness,
    PlayerMailbox, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerPreferredFoot,
    PlayerSkills, PlayerStatistics, PlayerStatisticsHistory, PlayerStatus, PlayerTraining,
    PlayerTrainingHistory, Relations, Technical,
};
use chrono::{Datelike, NaiveDate};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

static PLAYER_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(100_000));

const SKILL_COUNT: usize = 37;

// Skill index constants (flat array order)
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

/// Box-Muller normal distribution
fn random_normal() -> f32 {
    let u1 = rand::random::<f32>().max(1e-10);
    let u2 = rand::random::<f32>();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
}


#[derive(Copy, Clone)]
enum PositionType {
    Goalkeeper,
    Defender,
    Midfielder,
    Striker,
}

fn position_type_from(pos: PlayerPositionType) -> PositionType {
    match pos {
        PlayerPositionType::Goalkeeper => PositionType::Goalkeeper,
        PlayerPositionType::Sweeper
        | PlayerPositionType::DefenderLeft
        | PlayerPositionType::DefenderCenterLeft
        | PlayerPositionType::DefenderCenter
        | PlayerPositionType::DefenderCenterRight
        | PlayerPositionType::DefenderRight => PositionType::Defender,
        PlayerPositionType::WingbackLeft
        | PlayerPositionType::WingbackRight
        | PlayerPositionType::DefensiveMidfielder
        | PlayerPositionType::MidfielderLeft
        | PlayerPositionType::MidfielderCenterLeft
        | PlayerPositionType::MidfielderCenter
        | PlayerPositionType::MidfielderCenterRight
        | PlayerPositionType::MidfielderRight => PositionType::Midfielder,
        PlayerPositionType::AttackingMidfielderLeft
        | PlayerPositionType::AttackingMidfielderCenter
        | PlayerPositionType::AttackingMidfielderRight
        | PlayerPositionType::ForwardLeft
        | PlayerPositionType::ForwardCenter
        | PlayerPositionType::ForwardRight
        | PlayerPositionType::Striker => PositionType::Striker,
    }
}

/// Position distribution weights. Higher = more CA budget allocated to this skill.
/// These are NOT multipliers — they are proportional shares of the CA budget.
/// A weight of 1.8 gets ~2.25x the budget of a weight of 0.8, producing naturally
/// higher skills for key attributes without collapsing weak ones.
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
/// Roles aggressively reshape the weight distribution — a Poacher and a Target Man
/// should look fundamentally different, not subtly different.
/// Boost magnitudes: +0.8 to +1.5 for key skills, −0.5 to −1.0 for suppressed skills.
fn apply_role_archetype(weights: &mut [f32; SKILL_COUNT], position: &PositionType) {
    let roll = rand::random::<f32>();

    match position {
        PositionType::Goalkeeper => {
            if roll < 0.35 {
                // Shot Stopper: agility, reflexes, concentration
                weights[SK_AGILITY] += 1.0; weights[SK_ANTICIPATION] += 0.8;
                weights[SK_CONCENTRATION] += 0.8; weights[SK_POSITIONING] += 0.5;
                weights[SK_PASSING] -= 0.5; weights[SK_FIRST_TOUCH] -= 0.5;
                weights[SK_PACE] -= 0.5;
            } else if roll < 0.60 {
                // Sweeper Keeper: distribution, pace, bravery
                weights[SK_PACE] += 1.2; weights[SK_PASSING] += 1.0;
                weights[SK_FIRST_TOUCH] += 0.8; weights[SK_BRAVERY] += 0.8;
                weights[SK_POSITIONING] -= 0.5; weights[SK_CONCENTRATION] -= 0.3;
            } else if roll < 0.85 {
                // Commanding: aerial, leadership, strength
                weights[SK_JUMPING] += 1.0; weights[SK_STRENGTH] += 1.0;
                weights[SK_LEADERSHIP] += 0.8; weights[SK_BRAVERY] += 0.8;
                weights[SK_PACE] -= 0.5; weights[SK_AGILITY] -= 0.3;
                weights[SK_PASSING] -= 0.3;
            } else {
                // Traditional: balanced, slight concentration edge
                weights[SK_POSITIONING] += 0.4; weights[SK_CONCENTRATION] += 0.4;
            }
        }
        PositionType::Defender => {
            if roll < 0.25 {
                // Ball-Playing: passing, composure, technique
                weights[SK_PASSING] += 1.2; weights[SK_FIRST_TOUCH] += 1.0;
                weights[SK_COMPOSURE] += 0.8; weights[SK_TECHNIQUE] += 0.8;
                weights[SK_AGGRESSION] -= 0.8; weights[SK_HEADING] -= 0.5;
                weights[SK_STRENGTH] -= 0.3;
            } else if roll < 0.50 {
                // Stopper: aggressive, strong, brave
                weights[SK_AGGRESSION] += 1.0; weights[SK_HEADING] += 1.0;
                weights[SK_STRENGTH] += 0.8; weights[SK_BRAVERY] += 0.8;
                weights[SK_PASSING] -= 0.8; weights[SK_TECHNIQUE] -= 0.8;
                weights[SK_FLAIR] -= 0.5; weights[SK_DRIBBLING] -= 0.5;
            } else if roll < 0.75 {
                // Athletic: fast, mobile, stamina
                weights[SK_PACE] += 1.2; weights[SK_ACCELERATION] += 1.0;
                weights[SK_AGILITY] += 0.8; weights[SK_STAMINA] += 0.5;
                weights[SK_STRENGTH] -= 0.8; weights[SK_HEADING] -= 0.5;
            } else {
                // No-Nonsense: marking, tackling specialist
                weights[SK_MARKING] += 1.0; weights[SK_TACKLING] += 0.8;
                weights[SK_POSITIONING] += 0.8;
                weights[SK_DRIBBLING] -= 0.8; weights[SK_FLAIR] -= 0.8;
                weights[SK_VISION] -= 0.5; weights[SK_TECHNIQUE] -= 0.3;
            }
        }
        PositionType::Midfielder => {
            if roll < 0.20 {
                // Playmaker: vision, technique, composure
                weights[SK_VISION] += 1.2; weights[SK_PASSING] += 1.0;
                weights[SK_TECHNIQUE] += 1.0; weights[SK_COMPOSURE] += 0.8;
                weights[SK_FIRST_TOUCH] += 0.5;
                weights[SK_TACKLING] -= 0.8; weights[SK_AGGRESSION] -= 0.8;
                weights[SK_STRENGTH] -= 0.5; weights[SK_HEADING] -= 0.5;
            } else if roll < 0.40 {
                // Box-to-Box: stamina, work rate, strength
                weights[SK_STAMINA] += 1.2; weights[SK_WORK_RATE] += 1.0;
                weights[SK_TACKLING] += 0.8; weights[SK_STRENGTH] += 0.8;
                weights[SK_PACE] += 0.5;
                weights[SK_FLAIR] -= 0.8; weights[SK_VISION] -= 0.5;
                weights[SK_TECHNIQUE] -= 0.3;
            } else if roll < 0.60 {
                // Ball Winner: tackling, aggression, marking
                weights[SK_TACKLING] += 1.5; weights[SK_MARKING] += 1.2;
                weights[SK_AGGRESSION] += 1.0; weights[SK_STRENGTH] += 0.8;
                weights[SK_POSITIONING] += 0.5;
                weights[SK_TECHNIQUE] -= 0.8; weights[SK_VISION] -= 0.8;
                weights[SK_FLAIR] -= 0.8; weights[SK_DRIBBLING] -= 0.5;
            } else if roll < 0.80 {
                // Winger: pace, crossing, dribbling, flair
                weights[SK_PACE] += 1.5; weights[SK_CROSSING] += 1.2;
                weights[SK_DRIBBLING] += 1.0; weights[SK_ACCELERATION] += 0.8;
                weights[SK_FLAIR] += 0.8;
                weights[SK_TACKLING] -= 0.8; weights[SK_MARKING] -= 0.8;
                weights[SK_HEADING] -= 0.8; weights[SK_STRENGTH] -= 0.5;
            } else {
                // Mezzala: dribbling, movement, technique
                weights[SK_DRIBBLING] += 1.0; weights[SK_OFF_THE_BALL] += 0.8;
                weights[SK_TECHNIQUE] += 0.8; weights[SK_ACCELERATION] += 0.5;
                weights[SK_MARKING] -= 0.5; weights[SK_HEADING] -= 0.5;
            }
        }
        PositionType::Striker => {
            if roll < 0.25 {
                // Poacher: clinical finisher, movement, composure
                weights[SK_FINISHING] += 1.5; weights[SK_OFF_THE_BALL] += 1.2;
                weights[SK_ANTICIPATION] += 0.8; weights[SK_COMPOSURE] += 0.8;
                weights[SK_DRIBBLING] -= 0.8; weights[SK_PASSING] -= 0.8;
                weights[SK_VISION] -= 0.8; weights[SK_LONG_SHOTS] -= 0.5;
            } else if roll < 0.45 {
                // Target Man: heading, strength, first touch
                weights[SK_HEADING] += 1.5; weights[SK_STRENGTH] += 1.5;
                weights[SK_FIRST_TOUCH] += 0.8; weights[SK_BRAVERY] += 0.8;
                weights[SK_PACE] -= 1.0; weights[SK_ACCELERATION] -= 0.8;
                weights[SK_DRIBBLING] -= 0.5; weights[SK_AGILITY] -= 0.5;
            } else if roll < 0.65 {
                // Speed Merchant: pace, acceleration, agility
                weights[SK_PACE] += 1.5; weights[SK_ACCELERATION] += 1.2;
                weights[SK_DRIBBLING] += 0.8; weights[SK_AGILITY] += 0.8;
                weights[SK_OFF_THE_BALL] += 0.5;
                weights[SK_HEADING] -= 0.8; weights[SK_STRENGTH] -= 0.8;
                weights[SK_LONG_SHOTS] -= 0.5;
            } else if roll < 0.85 {
                // Complete Forward: well-rounded, every skill useful
                weights[SK_TECHNIQUE] += 0.5; weights[SK_PASSING] += 0.5;
                weights[SK_VISION] += 0.5; weights[SK_DECISIONS] += 0.5;
                weights[SK_FIRST_TOUCH] += 0.5; weights[SK_HEADING] += 0.3;
            } else {
                // Deep-Lying Forward: link-up play, creativity
                weights[SK_FIRST_TOUCH] += 1.2; weights[SK_PASSING] += 1.0;
                weights[SK_VISION] += 1.0; weights[SK_TECHNIQUE] += 0.8;
                weights[SK_FINISHING] -= 0.5; weights[SK_OFF_THE_BALL] -= 0.5;
                weights[SK_HEADING] -= 0.8; weights[SK_PACE] -= 0.3;
            }
        }
    }
}

/// Age factor applied to the TOTAL CA budget (not per-skill).
/// Youth players have lower CA that grows with development.
/// This is the only age adjustment — no per-skill age curves.
fn age_ca_factor(age: u32) -> f32 {
    match age {
        0..=12 => 0.35,
        13 => 0.42,
        14 => 0.50,
        15 => 0.58,
        16 => 0.65,
        17 => 0.72,
        18 => 0.80,
        19 => 0.86,
        20 => 0.92,
        21 => 0.96,
        22..=29 => 1.0,
        30..=31 => 0.97,
        32..=33 => 0.93,
        34..=35 => 0.87,
        _ => 0.80,
    }
}

/// Convert CA points allocated to a single skill into a 1-20 skill value.
/// Non-linear: high skills are exponentially more expensive.
///   ca_share ~0.5 → skill ~3
///   ca_share ~1.0 → skill ~5
///   ca_share ~2.0 → skill ~8
///   ca_share ~3.0 → skill ~10
///   ca_share ~4.5 → skill ~14
///   ca_share ~5.5 → skill ~17
///   ca_share ~6.0 → skill ~18
fn ca_to_skill(ca_share: f32) -> f32 {
    // ca_share = total_ca * (weight / sum_weights), typically in [0.5, 12] range.
    // Tuned so uniform-weight CA 110 → ~10, CA 60 → ~6, CA 200 → ~16.
    (ca_share.max(0.0).powf(0.7) * 4.2).clamp(1.0, 20.0)
}

/// Age-based maximum skill cap.
/// Young players cannot reach elite skill levels regardless of talent — they
/// need years of training and match experience. Mirrors Football Manager behavior.
fn age_skill_cap(age: u32) -> f32 {
    // In real FM, even wonderkids rarely exceed 16-17 at age 20.
    // Only fully mature players (25+) can reach 20 in any attribute.
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

fn apply_affinities(skills: &mut [f32; SKILL_COUNT]) {
    if skills[SK_PASSING] > 12.0 {
        let bonus = (skills[SK_PASSING] - 12.0) * 0.12;
        skills[SK_VISION] += bonus;
        skills[SK_FIRST_TOUCH] += bonus;
    }
    if skills[SK_AGGRESSION] > 12.0 {
        let bonus = (skills[SK_AGGRESSION] - 12.0) * 0.10;
        skills[SK_BRAVERY] += bonus;
        skills[SK_COMPOSURE] -= bonus * 0.5;
    }
    if skills[SK_PACE] > 12.0 {
        let bonus = (skills[SK_PACE] - 12.0) * 0.12;
        skills[SK_ACCELERATION] += bonus;
    }
    if skills[SK_FINISHING] > 12.0 {
        let bonus = (skills[SK_FINISHING] - 12.0) * 0.08;
        skills[SK_COMPOSURE] += bonus;
        skills[SK_ANTICIPATION] += bonus;
    }
    if skills[SK_DRIBBLING] > 12.0 {
        let bonus = (skills[SK_DRIBBLING] - 12.0) * 0.10;
        skills[SK_FLAIR] += bonus;
        skills[SK_AGILITY] += bonus;
    }
    if skills[SK_LEADERSHIP] > 12.0 {
        let bonus = (skills[SK_LEADERSHIP] - 12.0) * 0.08;
        skills[SK_DETERMINATION] += bonus;
        skills[SK_TEAMWORK] += bonus;
    }
}

/// Pick a few random skills per group to spike up and a few to dip down,
/// so that even flat low-level profiles have visible individual variation.
fn apply_talent_spikes(skills: &mut [f32; SKILL_COUNT], mean_skill: f32) {
    // Spike magnitude scales with base level — low-ability players get
    // proportionally larger spikes so the differences survive integer rounding
    let spike_up = (1.5 + (10.0 - mean_skill).max(0.0) * 0.2).min(4.0);
    let spike_down = spike_up * 0.6;

    // (group_start, group_end, spikes_up, spikes_down)
    let groups: [(usize, usize, usize, usize); 3] = [
        (0, 14, 2, 2),   // Technical: 14 skills, 2 up / 2 down
        (14, 28, 3, 2),   // Mental: 14 skills, 3 up / 2 down
        (28, SKILL_COUNT, 2, 1), // Physical: 9 skills, 2 up / 1 down
    ];

    for &(start, end, n_up, n_down) in &groups {
        let len = end - start;

        // Pick random indices within the group
        let mut indices: Vec<usize> = (start..end).collect();

        // Fisher-Yates shuffle
        for i in (1..len).rev() {
            let j = (rand::random::<f32>() * (i + 1) as f32) as usize % (i + 1);
            indices.swap(i, j);
        }

        // First n_up get boosted
        for &idx in indices.iter().take(n_up) {
            skills[idx] += spike_up * (0.7 + rand::random::<f32>() * 0.6);
        }

        // Next n_down get reduced
        for &idx in indices.iter().skip(n_up).take(n_down) {
            skills[idx] -= spike_down * (0.5 + rand::random::<f32>() * 0.5);
        }
    }
}

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

pub struct PlayerGenerator;

impl PlayerGenerator {
    pub fn generate(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        level: u8,
        people_names: &PeopleNameGeneratorData,
    ) -> Player {
        Self::generate_with_facilities(
            country_id, now, position, level, people_names,
            0.35, 0.35, 0.35, // Average defaults
        )
    }

    /// Generate a youth player with FM-style facility modifiers:
    /// - youth_facility_quality: affects starting CA (skill quality of intake)
    /// - academy_quality: affects PA ceiling (potential of intake)
    /// - recruitment_quality: affects gem chance (finding exceptional talent)
    pub fn generate_with_facilities(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        level: u8,
        people_names: &PeopleNameGeneratorData,
        youth_facility_quality: f32,
        academy_quality: f32,
        recruitment_quality: f32,
    ) -> Player {
        let year = IntegerUtils::random(now.year() - 14, now.year() - 12) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 28) as u32;
        let age = (now.year() as u32).saturating_sub(year);

        // Academy level → reputation factor (level 1-10 maps to 0.05-0.50)
        let base_rep_factor = (level as f32 / 20.0).clamp(0.05, 0.50);

        // FM-style: Youth Facilities boost the effective rep_factor for skill generation
        // Poor youth facilities (0.05) → -20% CA, Best (1.0) → +30% CA
        // This means Man City's youth intake starts with better skills than Accrington's
        let youth_boost = 0.80 + youth_facility_quality * 0.50; // 0.83 to 1.30
        let rep_factor = (base_rep_factor * youth_boost).clamp(0.05, 0.65);

        let pos_type = position_type_from(position);
        let skills = Self::generate_skills(&pos_type, age, rep_factor);

        let current_ability = skills.calculate_ability_for_position(position);

        // FM-style: Youth Recruitment affects gem chance
        // Poor recruitment (0.05) → 3% gem, Average (0.35) → 8%, Exceptional (0.95) → 25%
        let gem_chance = 0.02 + recruitment_quality * 0.24;

        let gem_roll = rand::random::<f32>();
        let is_gem = gem_roll < gem_chance;

        // Academy quality is the primary driver of PA ceiling.
        // Poor academy (0.05): PA cap ~80,  headroom ~10-20 above CA
        // Average (0.35):      PA cap ~120, headroom ~15-35
        // Good (0.55):         PA cap ~145, headroom ~20-45
        // Excellent (0.75):    PA cap ~170, headroom ~25-55
        // Best (1.0):          PA cap ~200, headroom ~30-65
        let mut academy_pa_cap = (60.0 + academy_quality * 140.0) as i32; // 67..200

        // Rare prodigy: ~0.5% chance any club produces a 160+ PA talent
        // Even a village club can occasionally birth a Messi
        if rand::random::<f32>() < 0.005 {
            academy_pa_cap = academy_pa_cap.max(IntegerUtils::random(150, 170));
        }

        let potential_ability = if is_gem {
            // Gems: academy quality sets the ceiling, rep_factor fine-tunes
            let gem_min = (current_ability as i32 + 20).min(academy_pa_cap);
            let gem_max = (academy_pa_cap as f32 * (0.85 + rep_factor * 0.30)) as i32;
            IntegerUtils::random(gem_min, gem_max.clamp(gem_min, 200)).min(200) as u8
        } else {
            // Normal players: academy quality determines both headroom and cap
            let base_headroom = 10.0 + academy_quality * 55.0; // 10.8..65
            let headroom = (base_headroom * (0.6 + rep_factor * 0.8)) as i32; // rep_factor scales it
            let raw_pa = current_ability as i32 + IntegerUtils::random(5, headroom.max(6));
            raw_pa.min(academy_pa_cap).min(200) as u8
        };

        // Higher PA → higher chance of secondary position
        let positions = Self::generate_positions(position, potential_ability);

        // Generate name from country data
        let full_name = {
            let first = if people_names.first_names.is_empty() {
                String::from("Player")
            } else {
                people_names.first_names
                    [IntegerUtils::random(0, people_names.first_names.len() as i32 - 1) as usize]
                    .clone()
            };
            let last = if people_names.last_names.is_empty() {
                format!("{}", IntegerUtils::random(1, 99999))
            } else {
                people_names.last_names
                    [IntegerUtils::random(0, people_names.last_names.len() as i32 - 1) as usize]
                    .clone()
            };

            if !people_names.nicknames.is_empty() && IntegerUtils::random(0, 9) == 0 {
                let nick = &people_names.nicknames
                    [IntegerUtils::random(0, people_names.nicknames.len() as i32 - 1) as usize];
                FullName::with_nickname(first, last, nick.clone())
            } else {
                FullName::new(first, last)
            }
        };

        let preferred_foot = match IntegerUtils::random(0, 10) {
            0 => PlayerPreferredFoot::Both,
            1..=3 => PlayerPreferredFoot::Left,
            _ => PlayerPreferredFoot::Right,
        };

        let birth_date = NaiveDate::from_ymd_opt(year as i32, month, day).unwrap();

        // Youth contract
        let expiration =
            NaiveDate::from_ymd_opt(now.year() + IntegerUtils::random(2, 4), 6, 30).unwrap();
        let salary = (100 + (rep_factor * 500.0) as u32) as u32;
        let contract = PlayerClubContract::new_youth(salary, expiration);

        Player {
            id: PLAYER_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst),
            full_name,
            birth_date,
            country_id,
            behaviour: PersonBehaviour {
                state: PersonBehaviourState::Normal,
            },
            attributes: PersonAttributes {
                adaptability: rand::random::<f32>() * 20.0,
                ambition: rand::random::<f32>() * 20.0,
                controversy: rand::random::<f32>() * 10.0,
                loyalty: rand::random::<f32>() * 20.0,
                pressure: rand::random::<f32>() * 20.0,
                professionalism: rand::random::<f32>() * 20.0,
                sportsmanship: rand::random::<f32>() * 20.0,
                temperament: rand::random::<f32>() * 20.0,
            },
            happiness: PlayerHappiness::new(),
            statuses: PlayerStatus { statuses: vec![] },
            skills,
            contract: Some(contract),
            contract_loan: None,
            positions,
            preferred_foot,
            player_attributes: PlayerAttributes {
                is_banned: false,
                is_injured: false,
                condition: IntegerUtils::random(7500, 9000) as i16,
                fitness: IntegerUtils::random(5000, 8000) as i16,
                jadedness: 0,
                weight: IntegerUtils::random(55, 85) as u8,
                height: IntegerUtils::random(160, 200) as u8,
                value: 0,
                current_reputation: (rep_factor * 500.0) as i16,
                home_reputation: (rep_factor * 800.0) as i16,
                world_reputation: (rep_factor * 200.0) as i16,
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
            },
            mailbox: PlayerMailbox::new(),
            training: PlayerTraining::new(),
            training_history: PlayerTrainingHistory::new(),
            relations: Relations::new(),
            statistics: PlayerStatistics::default(),
            friendly_statistics: PlayerStatistics::default(),
            cup_statistics: PlayerStatistics::default(),
            statistics_history: PlayerStatisticsHistory::new(),
            decision_history: PlayerDecisionHistory::new(),
            languages: Vec::new(), // Academy youth — languages set at graduation
            last_transfer_date: None,
            plan: None,
        }
    }

    /// Per-player group bias: randomly tilts the balance between technical, mental, physical.
    /// Two players with the same CA and position will have different group emphasis.
    /// E.g. one striker might be "technical but physically weak", another "strong but clumsy".
    fn apply_group_bias(skills: &mut [f32; SKILL_COUNT]) {
        // Random multiplier per group: 0.85 to 1.15
        let tech_bias = 0.85 + rand::random::<f32>() * 0.30;
        let mental_bias = 0.85 + rand::random::<f32>() * 0.30;
        let phys_bias = 0.80 + rand::random::<f32>() * 0.40;

        for i in 0..14 {
            skills[i] *= tech_bias;
        }
        for i in 14..28 {
            skills[i] *= mental_bias;
        }
        for i in 28..SK_MATCH_READINESS {
            skills[i] *= phys_bias;
        }
    }

    /// Apply personal strengths and weaknesses: gives every player a recognizable identity.
    /// Picks 2-4 skills to boost (+2 to +5) and 2-4 to weaken (−2 to −4).
    /// Strength candidates are biased toward high-weight skills (role-appropriate).
    /// Weakness candidates are biased toward low-weight skills (role-inappropriate).
    fn apply_strengths_weaknesses(
        skills: &mut [f32; SKILL_COUNT],
        weights: &[f32; SKILL_COUNT],
    ) {
        let distributable = SK_MATCH_READINESS; // 36 real skills

        // Build sorted indices by weight for biased selection
        let mut indices: Vec<usize> = (0..distributable).collect();

        // Shuffle with Fisher-Yates for randomness
        for i in (1..distributable).rev() {
            let j = (rand::random::<f32>() * (i + 1) as f32) as usize % (i + 1);
            indices.swap(i, j);
        }

        // Sort first half by weight descending (strength candidates)
        // Sort second half by weight ascending (weakness candidates)
        let mid = distributable / 2;
        indices[..mid].sort_by(|&a, &b| weights[b].partial_cmp(&weights[a]).unwrap_or(std::cmp::Ordering::Equal));
        indices[mid..].sort_by(|&a, &b| weights[a].partial_cmp(&weights[b]).unwrap_or(std::cmp::Ordering::Equal));

        let n_strengths = 2 + (rand::random::<f32>() * 3.0) as usize; // 2-4
        let n_weaknesses = 2 + (rand::random::<f32>() * 3.0) as usize; // 2-4

        // Strengths: pick from the first half (biased toward high-weight skills)
        for &idx in indices[..n_strengths].iter() {
            let boost = 2.0 + rand::random::<f32>() * 3.0; // +2 to +5
            skills[idx] += boost;
        }

        // Weaknesses: pick from the second half (biased toward low-weight skills)
        // Don't overlap with strengths
        let mut weakness_count = 0;
        for &idx in indices[mid..].iter() {
            if weakness_count >= n_weaknesses {
                break;
            }
            // Skip if this was already a strength
            if indices[..n_strengths].contains(&idx) {
                continue;
            }
            let penalty = 2.0 + rand::random::<f32>() * 2.0; // −2 to −4
            skills[idx] -= penalty;
            weakness_count += 1;
        }
    }

    /// CA-budget skill generation (FM-like model).
    ///
    /// Instead of generating each skill independently from a mean (which collapses
    /// to identical values at low levels), this distributes a limited CA budget
    /// across skills using position weights as proportional shares.
    ///
    /// Pipeline:
    ///   1. Generate raw CA from rep_factor
    ///   2. Apply age factor to CA (single reduction, not per-skill)
    ///   3. Get position weights + role archetype
    ///   4. Normalize weights into proportional shares
    ///   5. Distribute CA budget: each skill gets ca_share = total_ca * (weight / sum)
    ///   6. Convert ca_share → skill value via non-linear curve (high skills cost more)
    ///   7. Apply per-skill noise (small, ±1.5 max)
    ///   8. Apply affinities and talent spikes
    ///   9. Clamp to [1, 20] with minimum floor
    fn generate_skills(position: &PositionType, age: u32, rep_factor: f32) -> PlayerSkills {
        // Step 1: Raw CA from reputation
        //   rep 0.05 (academy level 1) → CA ~20
        //   rep 0.25 (mid)             → CA ~65
        //   rep 0.50 (max academy)     → CA ~110
        let raw_ca = 10.0 + rep_factor * 200.0;

        // Step 2: Age reduces the effective CA (youth haven't developed yet)
        let ca = raw_ca * age_ca_factor(age);

        // Step 3: Position weights + role archetype for variety
        let mut weights = position_weights(position);
        apply_role_archetype(&mut weights, position);

        // Clamp weights to [0.4, 2.2] — no weight can starve a skill to nothing
        for w in weights.iter_mut() {
            *w = w.clamp(0.4, 2.2);
        }

        // Step 4: Normalize weights into proportional shares
        // Exclude SK_MATCH_READINESS from distribution (it's not a real skill)
        let distributable_count = SKILL_COUNT - 1; // 36 real skills
        let total_weight: f32 = weights[..distributable_count].iter().sum();

        // Step 5: Distribute CA budget proportionally
        let mut skills = [0.0f32; SKILL_COUNT];

        for i in 0..distributable_count {
            let share = weights[i] / total_weight;
            let ca_for_skill = ca * share;

            // Step 6: Non-linear conversion — high skills are exponentially expensive
            skills[i] = ca_to_skill(ca_for_skill);
        }

        // Match readiness starts at a reasonable default
        skills[SK_MATCH_READINESS] = 10.0 + rand::random::<f32>() * 5.0;

        // Step 7: Per-player group bias — tilts tech/mental/physical balance
        // Makes two same-position players feel different ("technical but weak" vs "athletic but clumsy")
        Self::apply_group_bias(&mut skills);

        // Step 8: Per-skill noise (applied AFTER base + group bias)
        for skill in skills[..distributable_count].iter_mut() {
            *skill = (*skill + random_normal() * 1.2).clamp(1.0, 20.0);
        }

        // Step 9: Personal strengths & weaknesses — gives every player a recognizable identity
        // "this guy is fast but dumb", "pure finisher", "technical but physically weak"
        Self::apply_strengths_weaknesses(&mut skills, &weights);

        // Step 10: Affinities (correlated skill boosts)
        apply_affinities(&mut skills);

        // Step 11: Talent spikes for extra individuality
        let avg_skill = skills[..distributable_count].iter().sum::<f32>() / distributable_count as f32;
        apply_talent_spikes(&mut skills, avg_skill);

        // Step 12: Final clamp with minimum floor and age cap
        let min_floor = (3.0 + rep_factor * 3.0).clamp(3.0, 5.0);
        let physical_floor_base = (4.0 + rep_factor * 4.0).clamp(6.0, 8.0);
        let cap = age_skill_cap(age);
        for i in 0..distributable_count {
            if i >= 28 {
                let jitter = (random_normal() * 2.5).clamp(-3.0, 3.0);
                let floor = (physical_floor_base + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else {
                skills[i] = skills[i].clamp(min_floor, cap);
            }
        }

        skills_from_array(&skills)
    }

    /// Generate position profile. Primary position always 20.
    /// Higher PA → higher chance of having a secondary position.
    /// PA 60 → ~5%, PA 120 → ~15%, PA 160+ → ~30%
    fn generate_positions(primary: PlayerPositionType, potential_ability: u8) -> PlayerPositions {
        let mut positions = vec![PlayerPosition { position: primary, level: 20 }];

        // ~40% chance of one natural adjacent position
        let adjacent = natural_adjacent_positions(primary);
        if !adjacent.is_empty() && IntegerUtils::random(0, 99) < 40 {
            let pick = adjacent[IntegerUtils::random(0, adjacent.len() as i32 - 1) as usize];
            let level = IntegerUtils::random(14, 18) as u8;
            positions.push(PlayerPosition { position: pick, level });
        }

        // Higher PA → additional chance of a versatile position
        let pa = potential_ability as i32;
        let versatility_pct = (pa * pa / 800).min(35);
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
}

/// Natural adjacent positions that most players at a given position can also play.
fn natural_adjacent_positions(primary: PlayerPositionType) -> Vec<PlayerPositionType> {
    match primary {
        PlayerPositionType::Goalkeeper => vec![],
        PlayerPositionType::DefenderCenter => {
            if IntegerUtils::random(0, 2) == 0 {
                vec![PlayerPositionType::DefenderCenterLeft, PlayerPositionType::DefenderCenterRight]
            } else if IntegerUtils::random(0, 1) == 0 {
                vec![PlayerPositionType::DefenderCenterLeft]
            } else {
                vec![PlayerPositionType::DefenderCenterRight]
            }
        }
        PlayerPositionType::DefenderCenterLeft => vec![PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderLeft],
        PlayerPositionType::DefenderCenterRight => vec![PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderRight],
        PlayerPositionType::DefenderLeft => vec![PlayerPositionType::WingbackLeft],
        PlayerPositionType::DefenderRight => vec![PlayerPositionType::WingbackRight],
        PlayerPositionType::MidfielderCenter => {
            if IntegerUtils::random(0, 2) == 0 {
                vec![PlayerPositionType::MidfielderCenterLeft, PlayerPositionType::MidfielderCenterRight]
            } else if IntegerUtils::random(0, 1) == 0 {
                vec![PlayerPositionType::MidfielderCenterLeft]
            } else {
                vec![PlayerPositionType::MidfielderCenterRight]
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
fn pick_extra_position(primary: PlayerPositionType) -> Option<PlayerPositionType> {
    match primary {
        PlayerPositionType::Goalkeeper => None,
        PlayerPositionType::DefenderCenter => Some(PlayerPositionType::DefensiveMidfielder),
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
