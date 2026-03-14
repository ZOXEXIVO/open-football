use chrono::{Datelike, NaiveDate, Utc};
use core::shared::FullName;
use core::utils::{FloatUtils, IntegerUtils, StringUtils};
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

fn position_weights(position: &PositionType) -> [f32; SKILL_COUNT] {
    // Default 0.8: non-key skills are naturally ~20% below mean
    // Key skills override to 1.0-1.4, irrelevant skills drop to 0.3-0.6
    let mut w = [0.8f32; SKILL_COUNT];
    match position {
        PositionType::Goalkeeper => {
            // Key GK skills — high weights for clear differentiation
            w[SK_POSITIONING] = 1.6;
            w[SK_CONCENTRATION] = 1.5;
            w[SK_AGILITY] = 1.5;
            w[SK_ANTICIPATION] = 1.4;
            w[SK_COMPOSURE] = 1.4;
            w[SK_JUMPING] = 1.4;
            w[SK_BRAVERY] = 1.3;
            w[SK_DECISIONS] = 1.2;
            // Relevant
            w[SK_STRENGTH] = 1.1;
            w[SK_FIRST_TOUCH] = 1.1;
            w[SK_PASSING] = 1.1;
            w[SK_TECHNIQUE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_PACE] = 0.9;
            w[SK_STAMINA] = 0.9;
            w[SK_LEADERSHIP] = 1.0;
            w[SK_BALANCE] = 1.0;
            w[SK_DETERMINATION] = 1.0;
            w[SK_TEAMWORK] = 1.0;
            w[SK_PENALTY_TAKING] = 0.5;
            // Irrelevant outfield skills — low weights
            w[SK_FINISHING] = 0.2;
            w[SK_LONG_SHOTS] = 0.2;
            w[SK_CROSSING] = 0.2;
            w[SK_CORNERS] = 0.2;
            w[SK_FREE_KICKS] = 0.3;
            w[SK_HEADING] = 0.3;
            w[SK_OFF_THE_BALL] = 0.3;
            w[SK_DRIBBLING] = 0.4;
            w[SK_LONG_THROWS] = 0.5;
            w[SK_TACKLING] = 0.3;
            w[SK_MARKING] = 0.3;
            w[SK_WORK_RATE] = 0.5;
            w[SK_FLAIR] = 0.4;
            w[SK_ACCELERATION] = 0.7;
        }
        PositionType::Defender => {
            // Key
            w[SK_TACKLING] = 1.3;
            w[SK_MARKING] = 1.3;
            w[SK_POSITIONING] = 1.3;
            w[SK_HEADING] = 1.2;
            w[SK_STRENGTH] = 1.2;
            w[SK_CONCENTRATION] = 1.2;
            w[SK_ANTICIPATION] = 1.2;
            w[SK_BRAVERY] = 1.2;
            // Relevant
            w[SK_PACE] = 1.0;
            w[SK_JUMPING] = 1.0;
            w[SK_PASSING] = 1.0;
            w[SK_TEAMWORK] = 1.0;
            w[SK_DECISIONS] = 1.0;
            w[SK_COMPOSURE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_STAMINA] = 1.0;
            // Irrelevant
            w[SK_FINISHING] = 0.5;
            w[SK_DRIBBLING] = 0.6;
            w[SK_FLAIR] = 0.5;
            w[SK_LONG_SHOTS] = 0.5;
            w[SK_OFF_THE_BALL] = 0.6;
        }
        PositionType::Midfielder => {
            // Key
            w[SK_PASSING] = 1.3;
            w[SK_VISION] = 1.3;
            w[SK_STAMINA] = 1.2;
            w[SK_TECHNIQUE] = 1.2;
            w[SK_FIRST_TOUCH] = 1.2;
            w[SK_DECISIONS] = 1.2;
            w[SK_TEAMWORK] = 1.2;
            w[SK_WORK_RATE] = 1.2;
            // Relevant
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
            // Irrelevant
            w[SK_HEADING] = 0.7;
            w[SK_LONG_THROWS] = 0.6;
            w[SK_FINISHING] = 0.7;
        }
        PositionType::Striker => {
            // Key
            w[SK_FINISHING] = 1.4;
            w[SK_OFF_THE_BALL] = 1.3;
            w[SK_DRIBBLING] = 1.2;
            w[SK_PACE] = 1.2;
            w[SK_COMPOSURE] = 1.2;
            w[SK_FIRST_TOUCH] = 1.2;
            w[SK_ANTICIPATION] = 1.2;
            w[SK_ACCELERATION] = 1.2;
            // Relevant
            w[SK_HEADING] = 1.0;
            w[SK_TECHNIQUE] = 1.0;
            w[SK_STRENGTH] = 1.0;
            w[SK_AGILITY] = 1.0;
            w[SK_BALANCE] = 1.0;
            w[SK_DECISIONS] = 1.0;
            w[SK_DETERMINATION] = 1.0;
            w[SK_BRAVERY] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            // Irrelevant
            w[SK_TACKLING] = 0.4;
            w[SK_MARKING] = 0.4;
            w[SK_POSITIONING] = 0.6;
            w[SK_CONCENTRATION] = 0.7;
            w[SK_LONG_THROWS] = 0.5;
        }
    }
    w
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
        min_age: i32,
        max_age: i32,
        is_youth: bool,
    ) -> Player {
        let now = Utc::now();

        let rep_factor = (team_reputation as f32 / 10000.0).clamp(0.0, 1.0);

        let year = IntegerUtils::random(now.year() - max_age, now.year() - min_age) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 29) as u32;
        let age = (now.year() as u32).saturating_sub(year);

        let salary_min = (2000.0 + rep_factor * 30000.0) as i32;
        let salary_max = (10000.0 + rep_factor * 190000.0) as i32;

        let base_salary = IntegerUtils::random(salary_min, salary_max) as u32;
        let salary = if is_youth {
            base_salary / IntegerUtils::random(10, 100) as u32
        } else {
            base_salary
        };
        let expiration =
            NaiveDate::from_ymd_opt(now.year() + IntegerUtils::random(1, 5), 3, 14).unwrap();

        let contract = if is_youth {
            PlayerClubContract::new_youth(salary, expiration)
        } else {
            PlayerClubContract::new(salary, expiration)
        };

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
            .build()
            .expect("Failed to build Player")
    }

    // ── Skill generation pipeline ───────────────────────────────────────

    fn generate_skills(position: &PositionType, age: u32, rep_factor: f32, potential_ability: u8) -> PlayerSkills {
        // ── PA is the anchor ───────────────────────────────────────────────
        // PA maps to a "fully developed" skill level: what this player's average
        // skill would be at peak. Each skill group develops differently with age:
        //   Mental:    barely changes over career → starts near final level
        //   Physical:  changes ~10-20% over career → starts close to final level
        //   Technical: grows the most → young players start much lower

        let pa = potential_ability as f32;
        // Final skill level this PA implies (PA 1→1, PA 100→10.5, PA 200→20)
        let pa_final = (pa - 1.0) / 199.0 * 19.0 + 1.0;

        // Age-dependent development ratio per skill group (how close to final level)
        let tech_age_ratio = match age {
            0..=17 =>  0.40,
            18..=19 => 0.50,
            20..=22 => 0.65,
            23..=26 => 0.80,
            27..=29 => 0.92,
            30..=32 => 0.95,
            _ =>       0.90,
        };
        let mental_age_ratio = match age {
            0..=17 =>  0.82,
            18..=19 => 0.85,
            20..=22 => 0.90,
            23..=26 => 0.95,
            27..=29 => 0.98,
            30..=32 => 1.0,
            _ =>       1.0, // mental doesn't decline
        };
        let physical_age_ratio = match age {
            0..=17 =>  0.70,
            18..=19 => 0.78,
            20..=22 => 0.88,
            23..=26 => 0.95,
            27..=29 => 1.0,  // physical peak
            30..=32 => 0.93,
            _ =>       0.82, // decline after 32
        };

        // Group means: PA-driven (85%) with small team context bonus (15%)
        let rep_bonus = rep_factor * 1.5; // 0.0 to 1.5 extra skill points
        let tech_mean   = pa_final * tech_age_ratio + rep_bonus;
        let mental_mean = pa_final * mental_age_ratio + rep_bonus;
        let phys_mean   = pa_final * physical_age_ratio + rep_bonus;

        let max_possible = 20.0_f32;
        let pos_w = position_weights(position);

        // Noise per group:
        //   Technical — widest spread (distinct skill profiles), extra for youth
        //   Mental — narrow (personality is consistent)
        //   Physical — moderate (cohesion applied later)
        let base_noise = 1.5 + rep_factor * 1.0;
        let tech_noise = if age <= 18 { base_noise + 2.0 } else { base_noise + 0.5 };
        let mental_noise = base_noise * 0.5;
        let phys_noise = base_noise * 0.7;

        let mut skills = [0.0f32; SKILL_COUNT];

        for i in 0..SKILL_COUNT {
            // 1. Pick the correct group mean and noise
            let (group_mean, noise) = match skill_group(i) {
                0 => (tech_mean, tech_noise),
                1 => (mental_mean, mental_noise),
                _ => (phys_mean, phys_noise),
            };

            // 2. base = group_mean + noise
            let base = group_mean + random_normal() * noise;

            // 3. Apply position weight (floored at 0.4 to prevent crushing)
            let effective_pos_w = pos_w[i].max(0.4);

            // 4. Apply per-skill age curve for individual peak timing
            let raw = base * age_curve(i, age) * effective_pos_w;

            // 5. Clamp
            skills[i] = raw.min(max_possible).clamp(1.0, 20.0);
        }

        // 6. Mental cohesion: pull toward group mean (mentality is unified)
        let m_start = 14; // SK_AGGRESSION
        let m_end = 28;   // through SK_WORK_RATE
        let m_count = (m_end - m_start) as f32;
        let m_avg: f32 = skills[m_start..m_end].iter().sum::<f32>() / m_count;
        for i in m_start..m_end {
            skills[i] = skills[i] * 0.70 + m_avg * 0.30;
        }

        // 7. Physical cohesion: pull toward group mean (body is one unit)
        let p_start = 28; // SK_ACCELERATION
        let p_end = SKILL_COUNT;
        let p_count = (p_end - p_start) as f32;
        let p_avg: f32 = skills[p_start..p_end].iter().sum::<f32>() / p_count;
        for i in p_start..p_end {
            skills[i] = skills[i] * 0.65 + p_avg * 0.35;
        }

        // 8. PA-based floor: high-PA players can't have garbage skills
        //    PA 180 → floor ~8, PA 100 → floor ~4, PA 40 → floor 3
        let pa_floor = ((pa - 40.0) / 160.0 * 6.0 + 3.0).clamp(3.0, 10.0);
        let key_floor = (pa_final * 0.55).clamp(pa_floor, 12.0);
        for i in 0..SKILL_COUNT {
            if pos_w[i] >= 1.0 {
                skills[i] = skills[i].max(key_floor);
            }
            skills[i] = skills[i].max(pa_floor);
        }

        // 9. Apply affinities
        apply_affinities(&mut skills);

        // 10. Final clamp
        for v in skills.iter_mut() {
            *v = v.clamp(3.0, 20.0);
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
        let gem_roll = rand::random::<f32>();
        let gem_chance = 0.05 + rep_factor * 0.10; // 5-15% chance per player
        let is_gem = gem_roll < gem_chance;

        if is_gem {
            // High-potential player
            let gem_min = (95.0 + rep_factor * 35.0) as i32;
            let gem_max = (150 + (rep_factor * 40.0) as i32).min(195);
            IntegerUtils::random(gem_min, gem_max).min(200) as u8
        } else {
            // Normal player: PA based on rep with age-dependent variance
            let base = 35.0 + rep_factor * 110.0; // rep 0.0 → 35, rep 1.0 → 145
            let youth_bonus = if age <= 21 { 12.0 } else if age <= 25 { 6.0 } else { 0.0 };
            let pa = base + youth_bonus + random_normal() * 14.0;
            pa.clamp(30.0, 195.0) as u8
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
        if !self.people_names_data.first_names.is_empty() {
            let idx =
                IntegerUtils::random(0, self.people_names_data.first_names.len() as i32) as usize;
            self.people_names_data.first_names[idx].to_owned()
        } else {
            StringUtils::random_string(5)
        }
    }

    fn generate_last_name(&self) -> String {
        if !self.people_names_data.first_names.is_empty() {
            let idx =
                IntegerUtils::random(0, self.people_names_data.last_names.len() as i32) as usize;
            self.people_names_data.last_names[idx].to_owned()
        } else {
            StringUtils::random_string(12)
        }
    }
}
