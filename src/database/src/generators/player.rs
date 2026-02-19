use chrono::{Datelike, NaiveDate, Utc};
use core::shared::FullName;
use core::utils::{FloatUtils, IntegerUtils, StringUtils};
use core::{
    Mental, PeopleNameGeneratorData, PersonAttributes, Physical, Player,
    PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, Technical,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock};

static PLAYER_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(1));

pub struct PlayerGenerator {
    people_names_data: PeopleNameGeneratorData,
}

impl PlayerGenerator {
    pub fn with_people_names(people_names: &PeopleNameGeneratorData) -> Self {
        PlayerGenerator {
            people_names_data: PeopleNameGeneratorData {
                first_names: people_names.first_names.clone(),
                last_names: people_names.last_names.clone(),
            },
        }
    }
}

pub enum PositionType {
    Goalkeeper,
    Defender,
    Midfielder,
    Striker,
}

impl PlayerGenerator {
    pub fn generate(&mut self, country_id: u32, position: PositionType, team_reputation: u16, min_age: i32, max_age: i32, is_youth: bool) -> Player {
        let now = Utc::now();

        let rep_factor = (team_reputation as f32 / 10000.0).clamp(0.0, 1.0);

        let year = IntegerUtils::random(now.year() - max_age, now.year() - min_age) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 29) as u32;

        let salary_min = (2000.0 + rep_factor * 30000.0) as i32;
        let salary_max = (10000.0 + rep_factor * 190000.0) as i32;

        let base_salary = IntegerUtils::random(salary_min, salary_max) as u32;
        let salary = if is_youth {
            base_salary / IntegerUtils::random(10, 100) as u32
        } else {
            base_salary
        };
        let expiration = NaiveDate::from_ymd_opt(now.year() + IntegerUtils::random(1, 5), 3, 14).unwrap();

        let contract = if is_youth {
            PlayerClubContract::new_youth(salary, expiration)
        } else {
            PlayerClubContract::new(salary, expiration)
        };

        Player::builder()
            .id(PLAYER_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst))
            .full_name(FullName::new(
                self.generate_first_name(),
                self.generate_last_name(),
            ))
            .birth_date(NaiveDate::from_ymd_opt(year as i32, month, day).unwrap())
            .country_id(country_id)
            .skills(Self::generate_skills(rep_factor))
            .attributes(Self::generate_person_attributes())
            .player_attributes(Self::generate_player_attributes(rep_factor))
            .contract(Some(contract))
            .positions(Self::generate_positions(position))
            .build()
            .expect("Failed to build Player")
    }

    fn generate_skills(rep_factor: f32) -> PlayerSkills {
        let skill_min = 1.0 + rep_factor * 8.0;
        let skill_max = (6.0 + rep_factor * 15.0).min(20.0);

        PlayerSkills {
            technical: Technical {
                corners: FloatUtils::random(skill_min, skill_max),
                crossing: FloatUtils::random(skill_min, skill_max),
                dribbling: FloatUtils::random(skill_min, skill_max),
                finishing: FloatUtils::random(skill_min, skill_max),
                first_touch: FloatUtils::random(skill_min, skill_max),
                free_kicks: FloatUtils::random(skill_min, skill_max),
                heading: FloatUtils::random(skill_min, skill_max),
                long_shots: FloatUtils::random(skill_min, skill_max),
                long_throws: FloatUtils::random(skill_min, skill_max),
                marking: FloatUtils::random(skill_min, skill_max),
                passing: FloatUtils::random(skill_min, skill_max),
                penalty_taking: FloatUtils::random(skill_min, skill_max),
                tackling: FloatUtils::random(skill_min, skill_max),
                technique: FloatUtils::random(skill_min, skill_max),
            },
            mental: Mental {
                aggression: FloatUtils::random(skill_min, skill_max),
                anticipation: FloatUtils::random(skill_min, skill_max),
                bravery: FloatUtils::random(skill_min, skill_max),
                composure: FloatUtils::random(skill_min, skill_max),
                concentration: FloatUtils::random(skill_min, skill_max),
                decisions: FloatUtils::random(skill_min, skill_max),
                determination: FloatUtils::random(skill_min, skill_max),
                flair: FloatUtils::random(skill_min, skill_max),
                leadership: FloatUtils::random(skill_min, skill_max),
                off_the_ball: FloatUtils::random(skill_min, skill_max),
                positioning: FloatUtils::random(skill_min, skill_max),
                teamwork: FloatUtils::random(skill_min, skill_max),
                vision: FloatUtils::random(skill_min, skill_max),
                work_rate: FloatUtils::random(skill_min, skill_max),
            },
            physical: Physical {
                acceleration: FloatUtils::random(skill_min, skill_max),
                agility: FloatUtils::random(skill_min, skill_max),
                balance: FloatUtils::random(skill_min, skill_max),
                jumping: FloatUtils::random(skill_min, skill_max),
                natural_fitness: FloatUtils::random(skill_min, skill_max),
                pace: FloatUtils::random(skill_min, skill_max),
                stamina: FloatUtils::random(skill_min, skill_max),
                strength: FloatUtils::random(skill_min, skill_max),
                match_readiness: FloatUtils::random(skill_min, skill_max),
            },
        }
    }

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

    fn generate_player_attributes(rep_factor: f32) -> PlayerAttributes {
        let ca_min = (rep_factor * 80.0) as i32;
        let ca_max = (40.0 + rep_factor * 130.0).min(200.0) as i32;
        let current_ability = IntegerUtils::random(ca_min, ca_max).min(200) as u8;

        let pa_min = current_ability as i32;
        let pa_max = (current_ability as i32 + 50).min(200);
        let potential_ability = IntegerUtils::random(pa_min, pa_max) as u8;

        let rep_base = (rep_factor * 3000.0) as i32;

        PlayerAttributes {
            is_banned: false,
            is_injured: false,
            condition: IntegerUtils::random(3000, 10000) as i16,
            fitness: IntegerUtils::random(3000, 10000) as i16,
            jadedness: IntegerUtils::random(0, 5000) as i16,
            weight: IntegerUtils::random(60, 100) as u8,
            height: IntegerUtils::random(150, 220) as u8,
            value: 0,
            current_reputation: IntegerUtils::random((rep_base as f32 * 0.3) as i32, rep_base) as i16,
            home_reputation: IntegerUtils::random((rep_base as f32 * 0.5) as i32, rep_base) as i16,
            world_reputation: IntegerUtils::random((rep_base as f32 * 0.1) as i32, (rep_base as f32 * 0.4) as i32) as i16,
            current_ability,
            potential_ability,
            international_apps: IntegerUtils::random(0, (rep_factor * 100.0) as i32) as u16,
            international_goals: IntegerUtils::random(0, (rep_factor * 40.0) as i32) as u16,
            under_21_international_apps: IntegerUtils::random(0, 30) as u16,
            under_21_international_goals: IntegerUtils::random(0, 10) as u16,
            injury_days_remaining: 0,
            injury_type: None,
        }
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
