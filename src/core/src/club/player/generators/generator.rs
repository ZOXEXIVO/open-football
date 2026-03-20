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

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

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
        lerp(0.75, 1.0, t)
    } else if age <= peak_end {
        1.0
    } else {
        let t = ((age_f - peak_end as f32) / (40.0 - peak_end as f32)).clamp(0.0, 1.0);
        lerp(1.0, 0.65, t)
    }
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

fn position_weights(position: &PositionType) -> [f32; SKILL_COUNT] {
    let mut w = [0.8f32; SKILL_COUNT];
    match position {
        PositionType::Goalkeeper => {
            // GK-critical skills — much higher
            w[SK_POSITIONING] = 1.6; w[SK_CONCENTRATION] = 1.5; w[SK_AGILITY] = 1.5;
            w[SK_ANTICIPATION] = 1.4; w[SK_COMPOSURE] = 1.4; w[SK_JUMPING] = 1.4;
            w[SK_BRAVERY] = 1.3; w[SK_DECISIONS] = 1.2; w[SK_STRENGTH] = 1.1;
            // Modern GK skills
            w[SK_FIRST_TOUCH] = 1.1; w[SK_PASSING] = 1.1; w[SK_TECHNIQUE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0; w[SK_PACE] = 0.9; w[SK_STAMINA] = 0.9;
            // Irrelevant outfield skills — much lower
            w[SK_FINISHING] = 0.2; w[SK_LONG_SHOTS] = 0.2; w[SK_CROSSING] = 0.2;
            w[SK_CORNERS] = 0.2; w[SK_FREE_KICKS] = 0.3; w[SK_HEADING] = 0.3;
            w[SK_OFF_THE_BALL] = 0.3; w[SK_DRIBBLING] = 0.4; w[SK_LONG_THROWS] = 0.5;
            w[SK_TACKLING] = 0.3; w[SK_MARKING] = 0.3; w[SK_WORK_RATE] = 0.5;
            w[SK_FLAIR] = 0.4; w[SK_ACCELERATION] = 0.7;
        }
        PositionType::Defender => {
            w[SK_TACKLING] = 1.3; w[SK_MARKING] = 1.3; w[SK_POSITIONING] = 1.3;
            w[SK_HEADING] = 1.2; w[SK_STRENGTH] = 1.2; w[SK_CONCENTRATION] = 1.2;
            w[SK_ANTICIPATION] = 1.2; w[SK_BRAVERY] = 1.2;
            w[SK_PACE] = 1.0; w[SK_JUMPING] = 1.0; w[SK_PASSING] = 1.0;
            w[SK_TEAMWORK] = 1.0; w[SK_DECISIONS] = 1.0; w[SK_COMPOSURE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0; w[SK_STAMINA] = 1.0;
            w[SK_FINISHING] = 0.5; w[SK_DRIBBLING] = 0.6; w[SK_FLAIR] = 0.5;
            w[SK_LONG_SHOTS] = 0.5; w[SK_OFF_THE_BALL] = 0.6;
        }
        PositionType::Midfielder => {
            w[SK_PASSING] = 1.3; w[SK_VISION] = 1.3; w[SK_STAMINA] = 1.2;
            w[SK_TECHNIQUE] = 1.2; w[SK_FIRST_TOUCH] = 1.2; w[SK_DECISIONS] = 1.2;
            w[SK_TEAMWORK] = 1.2; w[SK_WORK_RATE] = 1.2;
            w[SK_DRIBBLING] = 1.0; w[SK_TACKLING] = 1.0; w[SK_POSITIONING] = 1.0;
            w[SK_COMPOSURE] = 1.0; w[SK_ANTICIPATION] = 1.0; w[SK_CONCENTRATION] = 1.0;
            w[SK_PACE] = 1.0; w[SK_ACCELERATION] = 1.0; w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_BALANCE] = 1.0;
            w[SK_HEADING] = 0.7; w[SK_LONG_THROWS] = 0.6; w[SK_FINISHING] = 0.7;
        }
        PositionType::Striker => {
            w[SK_FINISHING] = 1.4; w[SK_OFF_THE_BALL] = 1.3; w[SK_DRIBBLING] = 1.2;
            w[SK_PACE] = 1.2; w[SK_COMPOSURE] = 1.2; w[SK_FIRST_TOUCH] = 1.2;
            w[SK_ANTICIPATION] = 1.2; w[SK_ACCELERATION] = 1.2;
            w[SK_HEADING] = 1.0; w[SK_TECHNIQUE] = 1.0; w[SK_STRENGTH] = 1.0;
            w[SK_AGILITY] = 1.0; w[SK_BALANCE] = 1.0; w[SK_DECISIONS] = 1.0;
            w[SK_DETERMINATION] = 1.0; w[SK_BRAVERY] = 1.0; w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_TACKLING] = 0.4; w[SK_MARKING] = 0.4; w[SK_POSITIONING] = 0.6;
            w[SK_CONCENTRATION] = 0.7; w[SK_LONG_THROWS] = 0.5;
        }
    }
    w
}

fn age_group_weights(age: u32) -> (f32, f32, f32) {
    // (technical, mental, physical)
    if age <= 18 {
        // Young academy players: raw athleticism, developing game sense, low technique
        (0.80, 1.05, 1.30)
    } else if age <= 21 {
        (0.88, 1.02, 1.18)
    } else if age <= 29 {
        (1.0, 1.0, 1.0)
    } else {
        (1.0, 1.1, 0.8)
    }
}

fn skill_group(idx: usize) -> usize {
    if idx < 14 { 0 } else if idx < 28 { 1 } else { 2 }
}

fn apply_affinities(skills: &mut [f32; SKILL_COUNT]) {
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
        let year = IntegerUtils::random(now.year() - 14, now.year() - 12) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 28) as u32;
        let age = (now.year() as u32).saturating_sub(year);

        // Academy level → reputation factor (level 1-10 maps to 0.05-0.50)
        let rep_factor = (level as f32 / 20.0).clamp(0.05, 0.50);

        let pos_type = position_type_from(position);
        let skills = Self::generate_skills(&pos_type, age, rep_factor);

        let positions = PlayerPositions {
            positions: vec![PlayerPosition {
                position,
                level: 20,
            }],
        };

        let current_ability = skills.calculate_ability_for_position(position);

        // Potential ability with gem mechanic
        let gem_roll = rand::random::<f32>();
        let gem_chance = 0.05 + rep_factor * 0.20;
        let is_gem = gem_roll < gem_chance;

        let potential_ability = if is_gem {
            let gem_min = 140i32.min(current_ability as i32 + 30);
            let gem_max = (150 + (rep_factor * 50.0) as i32).min(200);
            IntegerUtils::random(gem_min, gem_max).min(200) as u8
        } else {
            let headroom = (30.0 * (0.5 + rep_factor)) as i32;
            (current_ability as i32 + IntegerUtils::random(5, headroom.max(6))).min(200) as u8
        };

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
            statistics_history: PlayerStatisticsHistory::new(),
            decision_history: PlayerDecisionHistory::new(),
            languages: Vec::new(), // Academy youth — languages set at graduation
            last_transfer_date: None,
        }
    }

    fn generate_skills(position: &PositionType, age: u32, rep_factor: f32) -> PlayerSkills {
        let mean_skill = 5.0 + rep_factor * 13.0;
        let max_possible = 20.0 * lerp(0.7, 1.0, rep_factor);
        let noise_std = 2.0 + rep_factor * 1.0;
        let youth_tech_noise = if age <= 18 { noise_std + 2.5 } else { noise_std };
        let pos_w = position_weights(position);
        let (tech_gw, mental_gw, phys_gw) = age_group_weights(age);

        let mut skills = [0.0f32; SKILL_COUNT];

        for i in 0..SKILL_COUNT {
            let effective_noise = if skill_group(i) == 0 {
                youth_tech_noise
            } else {
                noise_std
            };
            let base = mean_skill + random_normal() * effective_noise;

            let gw = match skill_group(i) {
                0 => tech_gw,
                1 => mental_gw,
                _ => phys_gw,
            };

            // Position weight adjusts as bonus/penalty around the base, not as a multiplier.
            // w=1.0 → no change, w=1.5 → +50% bonus, w=0.3 → base * 0.55 (floor at ~55%)
            let pos_adjust = 0.55 + 0.45 * pos_w[i];
            let raw = base * age_curve(i, age) * pos_adjust * gw;
            skills[i] = raw.min(max_possible).clamp(1.0, 20.0);
        }

        apply_affinities(&mut skills);

        // Ensure mental and physical don't collapse to same level as technical
        // Even weak players have basic athleticism and game awareness
        let tech_avg: f32 = skills[..14].iter().sum::<f32>() / 14.0;
        let mental_floor = tech_avg + 1.0;
        let physical_floor = tech_avg + 2.0;

        for skill in &mut skills[14..28] {
            if *skill < mental_floor {
                *skill = mental_floor + random_normal().abs() * 0.5;
            }
        }
        for skill in &mut skills[28..SKILL_COUNT] {
            if *skill < physical_floor {
                *skill = physical_floor + random_normal().abs() * 0.5;
            }
        }

        // Inject intra-group talent spikes so skills within a group don't all
        // round to the same integer. Every player has natural strengths/weaknesses.
        apply_talent_spikes(&mut skills, mean_skill);

        for v in skills.iter_mut() {
            *v = v.clamp(1.0, 20.0);
        }

        skills_from_array(&skills)
    }
}
