use core::shared::FullName;
use core::utils::{FloatUtils, IntegerUtils, StringUtils};
use core::{
    Datelike, Mental, NaiveDate, PeopleNameGeneratorData, PersonAttributes, Physical, Player,
    PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, Technical, Utc,
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
    pub fn generate(&mut self, country_id: u32, position: PositionType) -> Player {
        let now = Utc::now();

        let year = IntegerUtils::random(now.year() - 35, now.year() - 15) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 29) as u32;

        Player::builder()
            .id(PLAYER_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst))
            .full_name(FullName::with_full(
                self.generate_first_name(),
                self.generate_last_name(),
                StringUtils::random_string(17),
            ))
            .birth_date(NaiveDate::from_ymd_opt(year as i32, month, day).unwrap())
            .country_id(country_id)
            .skills(Self::generate_skills())
            .attributes(Self::generate_person_attributes())
            .player_attributes(Self::generate_player_attributes())
            .contract(Some(PlayerClubContract::new(
                IntegerUtils::random(1000, 200000) as u32,
                NaiveDate::from_ymd_opt(now.year() + IntegerUtils::random(1, 5), 3, 14).unwrap(),
            )))
            .positions(Self::generate_positions(position))
            .build()
            .expect("Failed to build Player")
    }

    fn generate_skills() -> PlayerSkills {
        PlayerSkills {
            technical: Technical {
                corners: FloatUtils::random(1.0, 20.0),
                crossing: FloatUtils::random(1.0, 20.0),
                dribbling: FloatUtils::random(1.0, 20.0),
                finishing: FloatUtils::random(1.0, 20.0),
                first_touch: FloatUtils::random(1.0, 20.0),
                free_kicks: FloatUtils::random(1.0, 20.0),
                heading: FloatUtils::random(1.0, 20.0),
                long_shots: FloatUtils::random(1.0, 20.0),
                long_throws: FloatUtils::random(1.0, 20.0),
                marking: FloatUtils::random(1.0, 20.0),
                passing: FloatUtils::random(1.0, 20.0),
                penalty_taking: FloatUtils::random(1.0, 20.0),
                tackling: FloatUtils::random(1.0, 20.0),
                technique: FloatUtils::random(1.0, 20.0),
            },
            mental: Mental {
                aggression: FloatUtils::random(1.0, 20.0),
                anticipation: FloatUtils::random(1.0, 20.0),
                bravery: FloatUtils::random(1.0, 20.0),
                composure: FloatUtils::random(1.0, 20.0),
                concentration: FloatUtils::random(1.0, 20.0),
                decisions: FloatUtils::random(1.0, 20.0),
                determination: FloatUtils::random(1.0, 20.0),
                flair: FloatUtils::random(1.0, 20.0),
                leadership: FloatUtils::random(1.0, 20.0),
                off_the_ball: FloatUtils::random(1.0, 20.0),
                positioning: FloatUtils::random(1.0, 20.0),
                teamwork: FloatUtils::random(1.0, 20.0),
                vision: FloatUtils::random(1.0, 20.0),
                work_rate: FloatUtils::random(1.0, 20.0),
            },
            physical: Physical {
                acceleration: FloatUtils::random(1.0, 20.0),
                agility: FloatUtils::random(1.0, 20.0),
                balance: FloatUtils::random(1.0, 20.0),
                jumping: FloatUtils::random(1.0, 20.0),
                natural_fitness: FloatUtils::random(1.0, 20.0),
                pace: FloatUtils::random(1.0, 20.0),
                stamina: FloatUtils::random(1.0, 20.0),
                strength: FloatUtils::random(1.0, 20.0),
                match_readiness: FloatUtils::random(1.0, 20.0),
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

    fn generate_player_attributes() -> PlayerAttributes {
        PlayerAttributes {
            is_banned: false,
            is_injured: false,
            condition: IntegerUtils::random(0, 10000) as i16,
            fitness: IntegerUtils::random(0, 10000) as i16,
            jadedness: IntegerUtils::random(0, 10000) as i16,
            weight: IntegerUtils::random(60, 100) as u8,
            height: IntegerUtils::random(150, 220) as u8,
            value: 0,
            current_reputation: IntegerUtils::random(0, 3000) as i16,
            home_reputation: IntegerUtils::random(0, 3000) as i16,
            world_reputation: IntegerUtils::random(0, 1000) as i16,
            current_ability: IntegerUtils::random(0, 100) as u8,
            potential_ability: IntegerUtils::random(80, 200) as u8,
            international_apps: IntegerUtils::random(0, 100) as u16,
            international_goals: IntegerUtils::random(0, 40) as u16,
            under_21_international_apps: IntegerUtils::random(0, 30) as u16,
            under_21_international_goals: IntegerUtils::random(0, 10) as u16,
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
