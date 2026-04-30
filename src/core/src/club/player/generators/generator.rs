use crate::club::player::load::PlayerLoad;
use crate::club::player::rapport::PlayerRapport;
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

/// Bump the procedural id sequence so the next generated player gets an id
/// strictly greater than `min_exclusive`. No-op if the counter is already
/// past it. This is the single source of truth for player-id allocation:
/// both the database loader (initial world generation) and the core
/// generator (academy intake, U18/U19 fallback) draw from it via
/// `next_player_id`. Two independent counters seeded to the same starting
/// value will hand out the same ids — the bug that put the academy youth
/// "Afran Ramazanov" at the same id as ODB veteran "Sandro Tsitaishvili".
/// One counter, one truth.
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

/// Allocate the next procedural player id. Atomic and monotonically
/// increasing — never returns the same value twice within a process.
/// Called by every generator path that mints a new `Player`.
pub fn next_player_id() -> u32 {
    PLAYER_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst)
}

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
/// Roles aggressively reshape the weight distribution — a Poacher and a Target Man
/// should look fundamentally different, not subtly different.
/// Boost magnitudes: +0.8 to +1.5 for key skills, −0.5 to −1.0 for suppressed skills.
fn apply_role_archetype(weights: &mut [f32; SKILL_COUNT], position: &PositionType) {
    let roll = rand::random::<f32>();

    match position {
        PositionType::Goalkeeper => {
            if roll < 0.35 {
                // Shot Stopper: agility, reflexes, concentration
                weights[SK_AGILITY] += 0.5; weights[SK_ANTICIPATION] += 0.4;
                weights[SK_CONCENTRATION] += 0.4; weights[SK_POSITIONING] += 0.3;
                weights[SK_PASSING] -= 0.3; weights[SK_FIRST_TOUCH] -= 0.3;
            } else if roll < 0.60 {
                // Sweeper Keeper: distribution, pace, bravery
                weights[SK_PACE] += 0.5; weights[SK_PASSING] += 0.5;
                weights[SK_FIRST_TOUCH] += 0.4; weights[SK_BRAVERY] += 0.3;
                weights[SK_POSITIONING] -= 0.2; weights[SK_CONCENTRATION] -= 0.2;
            } else if roll < 0.85 {
                // Commanding: aerial, leadership, strength
                weights[SK_JUMPING] += 0.5; weights[SK_STRENGTH] += 0.5;
                weights[SK_LEADERSHIP] += 0.4; weights[SK_BRAVERY] += 0.4;
                weights[SK_AGILITY] -= 0.2; weights[SK_PASSING] -= 0.2;
            } else {
                // Traditional: balanced, slight concentration edge
                weights[SK_POSITIONING] += 0.3; weights[SK_CONCENTRATION] += 0.3;
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

/// Subtle per-skill peak timing modifier within a group.
/// Range: 0.92 to 1.05 (fine-tuning, not a major multiplier).
fn age_curve(skill_idx: usize, age: u32) -> f32 {
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
        0.92 + (1.0 - 0.92) * t
    } else if age <= peak_end {
        1.03
    } else {
        let t = ((age_f - peak_end as f32) / (40.0 - peak_end as f32)).clamp(0.0, 1.0);
        1.03 + (0.92 - 1.03) * t
    }
}

/// Age-based maximum skill cap.
/// Young players cannot reach elite skill levels regardless of talent — they
/// need years of training and match experience.
fn age_skill_cap(age: u32) -> f32 {
    // Even wonderkids rarely exceed 16-17 at age 20.
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
        goalkeeping: Default::default(),
    }
}

/// Generate GK-specific skills from the PA budget.
/// Role archetypes: Shot Stopper, Sweeper Keeper, Commanding, All-Rounder
fn generate_gk_skills(pa_final: f32, age: u32) -> crate::Goalkeeping {
    use crate::Goalkeeping;

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

    let roll = rand::random::<f32>();
    let w: [f32; 13] = if roll < 0.35 {
        // Shot Stopper
        [0.9, 0.9, 0.8, 0.4, 0.6, 1.6, 0.7, 1.3, 0.6, 1.1, 1.7, 0.8, 0.7]
    } else if roll < 0.60 {
        // Sweeper Keeper
        [0.8, 1.0, 1.0, 1.2, 1.5, 1.1, 1.3, 1.2, 1.4, 0.7, 1.1, 1.5, 1.2]
    } else if roll < 0.82 {
        // Commanding
        [1.6, 1.5, 1.4, 0.5, 0.7, 1.2, 0.9, 1.0, 0.7, 1.3, 1.1, 0.9, 0.8]
    } else {
        // All-Rounder
        [1.0, 1.0, 1.0, 0.7, 1.0, 1.2, 1.0, 1.1, 0.9, 0.9, 1.2, 1.0, 0.9]
    };

    let mut gk = [0.0f32; 13];
    for i in 0..13 {
        let pos_mean = gk_mean + (w[i] - 1.0) * spread;
        gk[i] = (pos_mean + random_normal() * noise).clamp(1.0, 20.0);
    }

    let core_floor = (pa_final * 0.45).clamp(3.0, 10.0);
    let general_floor = (pa_final * 0.25).clamp(2.0, 7.0);
    gk[5] = gk[5].max(core_floor);   // handling
    gk[10] = gk[10].max(core_floor);  // reflexes
    gk[7] = gk[7].max(core_floor);   // one_on_ones
    for v in gk.iter_mut() { *v = v.max(general_floor).clamp(1.0, 20.0); }

    Goalkeeping {
        aerial_reach: gk[0], command_of_area: gk[1], communication: gk[2],
        eccentricity: gk[3], first_touch: gk[4], handling: gk[5],
        kicking: gk[6], one_on_ones: gk[7], passing: gk[8],
        punching: gk[9], reflexes: gk[10], rushing_out: gk[11], throwing: gk[12],
    }
}

/// Inputs to youth-player generation. The academy realism overhaul moved
/// from facility-only positional parameters to a single context so callers
/// can express *why* a player should turn out a certain way: the club's
/// reputation, the league's prestige, the country's football ecosystem,
/// pathway prestige, coaching, and physical facility ratings all contribute.
///
/// All `_score` fields are normalised to 0.0..1.0 so the generator math
/// stays in one continuous space — no special-cased "if elite" branches.
#[derive(Debug, Clone, Copy)]
pub struct AcademyGenerationContext {
    /// Academy facility rating, 1..20 (matches `FacilityLevel::to_rating`).
    pub academy_level: u8,
    /// Youth-team facilities (0..1). Drives starting CA / day-to-day
    /// coaching environment.
    pub youth_facility_quality: f32,
    /// Academy programme quality (0..1). Drives PA ceiling.
    pub academy_quality: f32,
    /// Recruitment network reach (0..1). Drives gem chance and the size of
    /// the candidate pool, *not* the average elite output.
    pub recruitment_quality: f32,
    /// Best `working_with_youngsters` on staff (0..1). Modest PA bonus.
    pub youth_coaching_quality: f32,
    /// Main team's blended reputation (0..1) — Real Madrid = ~0.95,
    /// regional minnow = ~0.05.
    pub club_reputation_score: f32,
    /// League reputation (0..1) — top-flight Premier League ≈ 0.95,
    /// fourth tier ≈ 0.15.
    pub league_reputation_score: f32,
    /// Country football ecosystem (0..1) — Brazil/Spain ~0.95,
    /// micro-nations ~0.05.
    pub country_reputation_score: f32,
    /// Internal academy pathway prestige (0..1) — lifts when the pathway
    /// keeps producing graduates, drops when it stalls.
    pub pathway_reputation_score: f32,
}

impl AcademyGenerationContext {
    /// Average-quality fallback. Used by tests and any caller that doesn't
    /// know the surrounding club state.
    pub fn average() -> Self {
        AcademyGenerationContext {
            academy_level: 7,
            youth_facility_quality: 0.35,
            academy_quality: 0.35,
            recruitment_quality: 0.35,
            youth_coaching_quality: 0.35,
            club_reputation_score: 0.30,
            league_reputation_score: 0.30,
            country_reputation_score: 0.30,
            pathway_reputation_score: 0.45,
        }
    }

    /// Build from raw 0..10000 reputation values + 0..1 facility/staff
    /// multipliers. Both the in-game academy intake and the initial U18/U19
    /// world generation funnel through here.
    #[allow(clippy::too_many_arguments)]
    pub fn from_components(
        academy_level: u8,
        youth_facility_quality: f32,
        academy_quality: f32,
        recruitment_quality: f32,
        youth_coaching_quality: f32,
        main_team_reputation: u16,
        league_reputation: u16,
        country_reputation: u16,
        pathway_reputation: u8,
    ) -> Self {
        AcademyGenerationContext {
            academy_level,
            youth_facility_quality: youth_facility_quality.clamp(0.0, 1.0),
            academy_quality: academy_quality.clamp(0.0, 1.0),
            recruitment_quality: recruitment_quality.clamp(0.0, 1.0),
            youth_coaching_quality: youth_coaching_quality.clamp(0.0, 1.0),
            club_reputation_score: (main_team_reputation as f32 / 10000.0).clamp(0.0, 1.0),
            league_reputation_score: (league_reputation as f32 / 10000.0).clamp(0.0, 1.0),
            country_reputation_score: (country_reputation as f32 / 10000.0).clamp(0.0, 1.0),
            pathway_reputation_score: (pathway_reputation as f32 / 100.0).clamp(0.0, 1.0),
        }
    }

    /// Combined-potential score (0..1). Single continuous signal that
    /// drives PA ceiling and gem rolls. Weights are tuned so:
    /// - top European club at top-flight, top country: ~0.85
    /// - mid-table top-flight, top country: ~0.55
    /// - lower-division, weaker country: ~0.20
    /// - regional minnow with poor facilities: ~0.05
    pub fn combined_potential_score(&self) -> f32 {
        let s = 0.30 * self.club_reputation_score
            + 0.18 * self.league_reputation_score
            + 0.10 * self.country_reputation_score
            + 0.22 * self.academy_quality
            + 0.10 * self.pathway_reputation_score
            + 0.10 * self.youth_coaching_quality;
        s.clamp(0.0, 1.0)
    }
}

/// Per-intake state passed across one annual academy class. Lets the
/// generator dampen successive elite rolls so that even a top academy
/// rarely ships three world-class prospects in the same year.
#[derive(Debug, Default, Clone, Copy)]
pub struct AcademyIntakeState {
    /// Players in this intake with PA >= 160.
    pub elite_seen: u8,
    /// Players in this intake with PA >= 180.
    pub world_class_seen: u8,
}

impl AcademyIntakeState {
    pub fn new() -> Self {
        AcademyIntakeState::default()
    }

    /// Multiplier applied to gem chance / prodigy odds for the next pick.
    /// Each elite already in the bag halves the next attempt; each
    /// world-class one halves it again. Multiplicative so the effect
    /// compounds inside a single intake.
    pub fn elite_damping_factor(&self) -> f32 {
        let elite = self.elite_seen as f32;
        let wc = self.world_class_seen as f32;
        (0.5_f32.powf(elite) * 0.5_f32.powf(wc)).max(0.05)
    }

    pub fn record(&mut self, pa: u8) {
        if pa >= 180 {
            self.world_class_seen = self.world_class_seen.saturating_add(1);
        } else if pa >= 160 {
            self.elite_seen = self.elite_seen.saturating_add(1);
        }
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
        let mut ctx = AcademyGenerationContext::average();
        ctx.academy_level = level;
        Self::generate_with_context(country_id, now, position, people_names, &ctx, 14, 14, None)
    }

    /// Reputation-aware academy intake. The single entry point for both the
    /// in-game academy (`ClubAcademy::produce_youth_players`) and the
    /// world-init U18/U19 generator. `intake_state` is `Some` when callers
    /// want elite-cluster damping across one annual class; `None` for
    /// one-off generation.
    pub fn generate_with_context(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        people_names: &PeopleNameGeneratorData,
        gen_ctx: &AcademyGenerationContext,
        min_age: i32,
        max_age: i32,
        intake_state: Option<&mut AcademyIntakeState>,
    ) -> Player {
        let year = IntegerUtils::random(now.year() - max_age, now.year() - min_age) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 28) as u32;
        let age = (now.year() as u32).saturating_sub(year);

        let level = gen_ctx.academy_level;
        let youth_facility_quality = gen_ctx.youth_facility_quality;
        let academy_quality = gen_ctx.academy_quality;
        let recruitment_quality = gen_ctx.recruitment_quality;

        // Combined potential score blends club / league / country reputation,
        // academy programme, pathway prestige, and coaching into a single
        // 0..1 driver. Same continuous signal whether the club is Real
        // Madrid (~0.85) or a regional minnow (~0.05) — no special-cased
        // tier branches.
        let cps = gen_ctx.combined_potential_score();

        // Per-intake elite-cluster damping: each prior elite (PA>=160)
        // hit in the same intake squeezes the next one's gem/talent rolls,
        // so a single class doesn't accidentally graduate three world-class
        // youngsters at once.
        let elite_damping = intake_state
            .as_ref()
            .map(|s| s.elite_damping_factor())
            .unwrap_or(1.0);

        // Floor (raw_ca) — physical facilities + youth coaching dominate the
        // floor; reputation contributes only modestly. Cambodian academy
        // with great facilities still produces a competent CA player.
        let norm_level = (level as f32 / 20.0).clamp(0.0, 1.0);
        let base_rep_factor = (norm_level.powf(1.2) * 0.45).clamp(0.01, 0.45);
        let youth_boost = 0.80 + youth_facility_quality * 0.40
            + gen_ctx.youth_coaching_quality * 0.10; // 0.83..1.30
        let cps_floor_lift = 1.0 + cps * 0.22; // up to +22% from reputation
        let rep_factor = (base_rep_factor * youth_boost * cps_floor_lift).clamp(0.01, 0.55);
        let raw_ca = 10.0 + rep_factor * 200.0;

        // Gem chance: recruitment widens the candidate pool, but elite
        // candidates only show up where reputation can attract them. Even
        // CPS=0 still leaves a tiny floor so small clubs can produce a
        // standout — just very rarely.
        let gem_chance = (0.0035
            + recruitment_quality * 0.012
            + cps * 0.022)
            * elite_damping;
        let is_gem = rand::random::<f32>() < gem_chance;

        // PA ceiling: a single continuous curve of CPS — academies at the
        // bottom cap around 110, mid clubs at 145, top clubs at 180+ before
        // any prodigy roll. Note: even at CPS=1 the cap stays under 190;
        // PA 190+ requires the prodigy path, never the regular one.
        let mut academy_pa_cap = (110.0 + cps.powf(1.15) * 78.0) as i32; // ~110..188

        // Rare prodigy: gates beyond the standard cap. The high tiers (PA
        // 175+, 190+) require the prodigy roll AND meaningful CPS, so a
        // hopeless minnow does not regularly mint world-class kids.
        let prodigy_roll = rand::random::<f32>() / elite_damping.max(1e-3);
        if prodigy_roll < 0.00005 && cps >= 0.55 {
            // Generational, only at well-resourced clubs.
            academy_pa_cap = academy_pa_cap.max(IntegerUtils::random(178, 195));
        } else if prodigy_roll < 0.00025 && cps >= 0.40 {
            // World-class — credible top-flight or strong second-flight.
            academy_pa_cap = academy_pa_cap.max(IntegerUtils::random(160, 180));
        } else if prodigy_roll < 0.0010 {
            // Very high potential — open to any club, including small ones,
            // because even a Norwich or Brentford can occasionally turn up
            // a future star.
            academy_pa_cap = academy_pa_cap.max(IntegerUtils::random(145, 165));
        }

        // PA base anchored on raw CA so we don't fight the age reduction.
        let pa_base = raw_ca as i32;

        let potential_ability = if is_gem {
            let gem_min = (pa_base + 10).min(academy_pa_cap - 10).max(pa_base);
            let gem_max = (academy_pa_cap as f32 * (0.85 + cps * 0.15)) as i32;
            IntegerUtils::random(gem_min, gem_max.clamp(gem_min, 200)).min(200) as u8
        } else {
            // Talent factor max climbs with CPS so top academies regularly
            // graduate the high-PA bands without needing a gem roll. CPS=0
            // tops out at ~0.85, CPS=1 at ~1.50.
            let talent_roll = rand::random::<f32>();
            let talent_max = 0.85 + cps * 0.65;
            let talent_factor = 0.35 + talent_roll.powi(2) * (talent_max - 0.35);
            let jittered_base = (raw_ca as f32 * talent_factor) as i32;

            // Headroom: bigger at well-resourced clubs (academy programme
            // + reputation pull) so the right-tail still has reach.
            let base_headroom = 8.0 + academy_quality * 28.0 + cps * 12.0; // ~9..48
            let headroom = (base_headroom * (0.70 + academy_quality * 0.30)) as i32;
            let raw_pa = jittered_base + IntegerUtils::random(0, headroom.max(5));
            raw_pa.max(20).min(academy_pa_cap).min(200) as u8
        };

        if let Some(state) = intake_state {
            state.record(potential_ability);
        }

        let pos_type = position_type_from(position);
        let skills = Self::generate_skills(&pos_type, age, rep_factor, potential_ability);

        let current_ability = skills.calculate_ability_for_position(position);

        // PA must never be lower than CA — position-weighted skill calculation
        // can produce CA above the raw PA when skills align well with the position
        let potential_ability = potential_ability.max(current_ability);

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
        let salary = (500 + (rep_factor * 5000.0) as u32) as u32;
        let contract = PlayerClubContract::new_youth(salary, expiration);

        Player {
            id: next_player_id(),
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
                consistency: 4.0 + rand::random::<f32>() * 14.0,
                important_matches: 4.0 + rand::random::<f32>() * 14.0,
                dirtiness: rand::random::<f32>() * 20.0,
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
            favorite_clubs: Vec::new(),
            sold_from: None,
            sell_on_obligations: Vec::new(),
            traits: Vec::new(), // academy youth start with no traits; grow via training
            is_force_match_selection: false,
            rapport: PlayerRapport::new(),
            promises: Vec::new(),
            interactions: crate::club::player::interaction::ManagerInteractionLog::new(),
            pending_signing: None,
            generated: true,
            retired: false,
            load: PlayerLoad::new(),
            pending_contract_ask: None,
            last_intl_caps_paid: 0,
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

    /// PA-anchored skill generation (same model as database generator).
    ///
    /// PA maps to a "fully developed" skill level. Position weights create
    /// differentiation via ADDITIVE spread so even low-PA youth have clear
    /// strengths/weaknesses (a young GK has high positioning, low finishing).
    ///
    /// Pipeline:
    ///   1. PA → pa_final (1-20 scale target)
    ///   2. Per-group age ratios (tech/mental/physical develop at different rates)
    ///   3. Additive position spread from group mean
    ///   4. Per-skill noise and age curve
    ///   5. Cohesion, floors, affinities, talent spikes
    ///   6. Clamp to [1, age_cap]
    fn generate_skills(position: &PositionType, age: u32, rep_factor: f32, potential_ability: u8) -> PlayerSkills {
        let pa = potential_ability as f32;
        // PA → target skill level at peak (PA 1→1, PA 100→10.5, PA 200→20)
        let pa_final = (pa - 1.0) / 199.0 * 19.0 + 1.0;

        // Age-dependent development ratio per skill group
        let tech_age_ratio = match age {
            0..=17 =>  0.75,
            18..=19 => 0.82,
            20..=22 => 0.90,
            23..=26 => 0.95,
            27..=29 => 1.0,
            30..=32 => 0.97,
            _ =>       0.93,
        };
        let mental_age_ratio = match age {
            0..=17 =>  0.55,
            18..=19 => 0.62,
            20..=22 => 0.72,
            23..=26 => 0.85,
            27..=29 => 0.95,
            30..=32 => 1.0,
            _ =>       1.0,
        };
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
        // Ensures differentiation at ALL ability levels including youth:
        //   PA 50 (pa_final ~5.7): spread ~2.8 → key ~8, weak ~3
        //   PA 100 (pa_final ~10.5): spread ~5.2 → key ~14, weak ~5
        let spread = (pa_final * 0.5).max(2.5);

        let mut pos_w = position_weights(position);
        apply_role_archetype(&mut pos_w, position);

        // Noise per group — youth have more technical noise (raw talent variation)
        let base_noise = 1.5 + rep_factor * 1.0;
        let tech_noise = if age <= 18 { base_noise + 2.0 } else { base_noise + 0.5 };
        let mental_noise = base_noise * 0.5;
        let phys_noise = base_noise * 1.5;

        let mut skills = [0.0f32; SKILL_COUNT];

        for i in 0..SKILL_COUNT {
            let (group_mean, noise) = match skill_group(i) {
                0 => (tech_mean, tech_noise),
                1 => (mental_mean, mental_noise),
                _ => (phys_mean, phys_noise),
            };

            // Additive position spread: key skills (w>1) get bonus, weak (w<1) get penalty
            let pos_mean = group_mean + (pos_w[i] - 1.0) * spread;
            let base = pos_mean + random_normal() * noise;
            let raw = base * age_curve(i, age);
            skills[i] = raw.clamp(1.0, 20.0);
        }

        // Mental cohesion: pull toward group mean (mentality is unified)
        let m_avg: f32 = skills[14..28].iter().sum::<f32>() / 14.0;
        for i in 14..28 {
            skills[i] = skills[i] * 0.70 + m_avg * 0.30;
        }

        // Physical cohesion: light pull toward group mean
        let p_count = (SKILL_COUNT - 28) as f32;
        let p_avg: f32 = skills[28..SKILL_COUNT].iter().sum::<f32>() / p_count;
        for i in 28..SKILL_COUNT {
            skills[i] = skills[i] * 0.85 + p_avg * 0.15;
        }

        // Match readiness default
        skills[SK_MATCH_READINESS] = 10.0 + rand::random::<f32>() * 5.0;

        // Per-player group bias for variety
        Self::apply_group_bias(&mut skills);

        // Personal strengths & weaknesses
        Self::apply_strengths_weaknesses(&mut skills, &pos_w);

        // Affinities (correlated skill boosts)
        apply_affinities(&mut skills);

        // Talent spikes for extra individuality
        let distributable_count = SKILL_COUNT - 1;
        let avg_skill = skills[..distributable_count].iter().sum::<f32>() / distributable_count as f32;
        apply_talent_spikes(&mut skills, avg_skill);

        // PA-based floors
        let key_floor = (pa_final * 0.40).clamp(1.0, 9.0);
        let universal_floor = (2.0 + pa_final * 0.2).clamp(4.0, 6.0);
        let physical_floor_base = (3.0 + pa_final * 0.35).clamp(6.0, 9.0);
        let trained_floor = (pa_final * 0.35 + 3.0).clamp(6.0, 9.0);
        let footballer_tech_floor = (pa_final * 0.30 + 2.0).clamp(4.0, 9.0);
        let cap = age_skill_cap(age);

        for i in 0..distributable_count {
            if pos_w[i] >= 1.2 {
                skills[i] = skills[i].max(key_floor);
            }
            if skill_group(i) == 2 {
                let jitter = (random_normal() * 2.0).clamp(-2.0, 2.0);
                let floor = (physical_floor_base + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else if skill_group(i) == 0 && pos_w[i] >= 0.8 {
                let jitter = (random_normal() * 1.5).clamp(-2.0, 2.0);
                let floor = (trained_floor + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else if skill_group(i) == 0 {
                let jitter = (random_normal() * 1.0).clamp(-1.0, 1.0);
                let floor = (footballer_tech_floor + jitter).max(4.0);
                skills[i] = skills[i].clamp(floor, cap);
            } else {
                skills[i] = skills[i].clamp(universal_floor, cap);
            }
        }

        let mut result = skills_from_array(&skills);

        // Generate GK-specific skills for goalkeepers
        if matches!(position, PositionType::Goalkeeper) {
            result.goalkeeping = generate_gk_skills(pa_final, age);
        }

        result
    }

    /// Generate position profile. Primary position always 20.
    /// Higher PA → higher chance of having a secondary position.
    /// PA 60 → ~5%, PA 120 → ~15%, PA 160+ → ~30%
    ///
    /// DCL/DCR and MCL/MCR are formation slots, not primary positions.
    /// DC players automatically get DCL/DCR, MC players get MCL/MCR.
    /// Wide players have a chance of cross-side versatility (e.g. ML + MR).
    fn generate_positions(primary: PlayerPositionType, potential_ability: u8) -> PlayerPositions {
        let mut positions = vec![PlayerPosition { position: primary, level: 20 }];

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

        // ~40% chance of one natural adjacent position
        let adjacent = natural_adjacent_positions(primary);
        if !adjacent.is_empty() && IntegerUtils::random(0, 99) < 40 {
            let pick = adjacent[IntegerUtils::random(0, adjacent.len() as i32 - 1) as usize];
            let level = IntegerUtils::random(14, 18) as u8;
            positions.push(PlayerPosition { position: pick, level });
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
        // DC: DCL/DCR are auto-added as formation slots; adjacent is DM
        PlayerPositionType::DefenderCenter => vec![PlayerPositionType::DefensiveMidfielder],
        // DCL/DCR kept for compatibility
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
/// Players who can play both flanks (e.g. M L/R) are more versatile and valuable.
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
mod academy_realism_tests {
    use super::{AcademyGenerationContext, AcademyIntakeState, PlayerGenerator};
    use crate::{PeopleNameGeneratorData, PlayerPositionType};
    use chrono::NaiveDate;

    fn empty_names() -> PeopleNameGeneratorData {
        PeopleNameGeneratorData {
            first_names: vec!["A".into()],
            last_names: vec!["B".into()],
            nicknames: vec![],
        }
    }

    fn now() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
    }

    fn generate_batch(ctx: &AcademyGenerationContext, n: usize) -> Vec<u8> {
        let names = empty_names();
        (0..n)
            .map(|_| {
                let p = PlayerGenerator::generate_with_context(
                    1,
                    now(),
                    PlayerPositionType::MidfielderCenter,
                    &names,
                    ctx,
                    14,
                    14,
                    None,
                );
                p.player_attributes.potential_ability
            })
            .collect()
    }

    fn weak_minnow_ctx() -> AcademyGenerationContext {
        AcademyGenerationContext::from_components(
            3, 0.10, 0.10, 0.10, 0.10,
            500, 800, 1500, 30,
        )
    }

    fn elite_ctx() -> AcademyGenerationContext {
        AcademyGenerationContext::from_components(
            20, 1.0, 1.0, 1.0, 1.0,
            9500, 9500, 9500, 90,
        )
    }

    fn cps_score(ctx: &AcademyGenerationContext) -> f32 {
        ctx.combined_potential_score()
    }

    #[test]
    fn cps_separates_minnow_from_elite() {
        let weak = weak_minnow_ctx();
        let elite = elite_ctx();
        assert!(cps_score(&weak) < 0.20);
        assert!(cps_score(&elite) > 0.85);
    }

    #[test]
    fn weak_club_mostly_low_pa() {
        let ctx = weak_minnow_ctx();
        let pas = generate_batch(&ctx, 400);
        let elite_count = pas.iter().filter(|&&pa| pa >= 140).count();
        let world_class = pas.iter().filter(|&&pa| pa >= 180).count();
        let avg = pas.iter().map(|&pa| pa as u32).sum::<u32>() / pas.len() as u32;

        assert!(avg < 90, "weak academy avg PA {avg} too high");
        assert!(
            elite_count <= pas.len() / 20,
            "weak academy produced too many 140+ PA: {elite_count} of {}",
            pas.len()
        );
        assert_eq!(
            world_class, 0,
            "weak academy minted a world-class prospect ({world_class}); expected zero in this batch"
        );
    }

    #[test]
    fn elite_club_better_average_but_few_world_class() {
        let weak_pas = generate_batch(&weak_minnow_ctx(), 400);
        let elite_pas = generate_batch(&elite_ctx(), 400);
        let weak_avg = weak_pas.iter().map(|&pa| pa as u32).sum::<u32>() / weak_pas.len() as u32;
        let elite_avg = elite_pas.iter().map(|&pa| pa as u32).sum::<u32>() / elite_pas.len() as u32;
        let world_class = elite_pas.iter().filter(|&&pa| pa >= 180).count();

        assert!(
            elite_avg >= weak_avg + 30,
            "elite avg PA {elite_avg} should be substantially higher than weak {weak_avg}"
        );
        assert!(
            world_class <= elite_pas.len() / 10,
            "even elite academy shouldn't produce {} (>10%) world-class prospects in 400 picks",
            world_class
        );
    }

    #[test]
    fn same_facilities_different_reputation_diverge() {
        // Identical physical facility ratings, only the reputation/league
        // signal differs. The realism overhaul means CA/PA distributions
        // should respond.
        let mid_facilities = (0.55, 0.55, 0.55, 0.55);
        let small = AcademyGenerationContext::from_components(
            11, mid_facilities.0, mid_facilities.1, mid_facilities.2, mid_facilities.3,
            800, 1000, 1500, 50,
        );
        let big = AcademyGenerationContext::from_components(
            11, mid_facilities.0, mid_facilities.1, mid_facilities.2, mid_facilities.3,
            9000, 9000, 9000, 80,
        );

        let small_pas = generate_batch(&small, 300);
        let big_pas = generate_batch(&big, 300);
        let small_avg = small_pas.iter().map(|&pa| pa as u32).sum::<u32>() / small_pas.len() as u32;
        let big_avg = big_pas.iter().map(|&pa| pa as u32).sum::<u32>() / big_pas.len() as u32;

        assert!(
            big_avg >= small_avg + 8,
            "reputation should shift PA: small={small_avg}, big={big_avg}"
        );
    }

    #[test]
    fn intake_state_dampens_successive_elites() {
        let mut state = AcademyIntakeState::new();
        assert!((state.elite_damping_factor() - 1.0).abs() < 1e-6);

        state.record(170);
        assert!(state.elite_damping_factor() < 0.6);

        state.record(190);
        assert!(state.elite_damping_factor() < 0.3);
    }

    #[test]
    fn academy_level_scale_normalised() {
        // tier collapsing should map facility rating 1..20 to pathway tier 1..10
        use crate::club::academy::academy::academy_tier;
        assert_eq!(academy_tier(1), 1);
        assert_eq!(academy_tier(2), 1);
        assert_eq!(academy_tier(11), 6);
        assert_eq!(academy_tier(15), 8);
        assert_eq!(academy_tier(19), 10);
        assert_eq!(academy_tier(20), 10);
    }
}

#[cfg(test)]
mod player_id_sequence_tests {
    use super::{next_player_id, seed_player_id_sequence};
    use std::sync::Mutex;

    // The id counter is process-global; serialise the tests so they don't
    // interleave and read each other's mid-flight values.
    static GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn next_id_is_strictly_above_seed() {
        let _g = GUARD.lock().unwrap();
        seed_player_id_sequence(2_000_000_000);
        let a = next_player_id();
        let b = next_player_id();
        assert!(a > 2_000_000_000, "id {a} did not advance past seed");
        assert!(b > a, "ids did not increase: {a} then {b}");
    }

    #[test]
    fn reseed_to_lower_value_does_not_regress() {
        let _g = GUARD.lock().unwrap();
        seed_player_id_sequence(2_500_000_000);
        let high = next_player_id();
        // Pretend a stale ODB scan finds a smaller max — the counter
        // must NOT walk back and start handing out colliding ids.
        seed_player_id_sequence(100_000);
        let next = next_player_id();
        assert!(
            next > high,
            "counter regressed: handed out {next} after {high}"
        );
    }
}
