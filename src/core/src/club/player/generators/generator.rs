use crate::shared::FullName;
use crate::utils::IntegerUtils;
use crate::{
    Mental, PersonAttributes, PersonBehaviour, PersonBehaviourState, Physical, Player,
    PlayerAttributes, PlayerHappiness, PlayerMailbox, PlayerPosition, PlayerPositionType,
    PlayerPositions, PlayerPreferredFoot, PlayerSkills, PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatus, PlayerTraining, PlayerTrainingHistory, Relations, Technical,
};
use chrono::{Datelike, NaiveDate};

pub struct PlayerGenerator;

impl PlayerGenerator {
    pub fn generate(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        level: u8,
    ) -> Player {
        let year = IntegerUtils::random(now.year() - 14, now.year() - 16) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 29) as u32;

        let positions = PlayerPositions {
            positions: vec![PlayerPosition { position, level }],
        };

        Player {
            id: IntegerUtils::random(0, 100000) as u32,
            full_name: FullName::with_full("".to_string(), "".to_string(), "".to_string()),
            birth_date: NaiveDate::from_ymd_opt(year as i32, month, day).unwrap(),
            country_id,
            behaviour: PersonBehaviour {
                state: PersonBehaviourState::Poor,
            },
            attributes: PersonAttributes {
                adaptability: 10.0,
                ambition: 10.0,
                controversy: 10.0,
                loyalty: 10.0,
                pressure: 10.0,
                professionalism: 10.0,
                sportsmanship: 10.0,
                temperament: 10.0,
            },
            happiness: PlayerHappiness::new(),
            statuses: PlayerStatus { statuses: vec![] },
            skills: PlayerSkills {
                technical: Technical {
                    corners: 10.0,
                    crossing: 10.0,
                    dribbling: 10.0,
                    finishing: 10.0,
                    first_touch: 10.0,
                    free_kicks: 10.0,
                    heading: 10.0,
                    long_shots: 10.0,
                    long_throws: 10.0,
                    marking: 10.0,
                    passing: 10.0,
                    penalty_taking: 10.0,
                    tackling: 10.0,
                    technique: 10.0,
                },
                mental: Mental {
                    aggression: 10.0,
                    anticipation: 10.0,
                    bravery: 10.0,
                    composure: 10.0,
                    concentration: 10.0,
                    decisions: 10.0,
                    determination: 10.0,
                    flair: 10.0,
                    leadership: 10.0,
                    off_the_ball: 10.0,
                    positioning: 10.0,
                    teamwork: 10.0,
                    vision: 10.0,
                    work_rate: 10.0,
                },
                physical: Physical {
                    acceleration: 10.0,
                    agility: 10.0,
                    balance: 10.0,
                    jumping: 10.0,
                    natural_fitness: 10.0,
                    pace: 10.0,
                    stamina: 10.0,
                    strength: 10.0,
                    match_readiness: 10.0,
                },
            },
            contract: Option::None,
            positions,
            preferred_foot: PlayerPreferredFoot::Left,
            player_attributes: PlayerAttributes {
                is_banned: false,
                is_injured: false,
                condition: 10000,
                fitness: 0,
                jadedness: 0,
                weight: 0,
                height: 0,
                value: 0,
                current_reputation: 0,
                home_reputation: 1000,
                world_reputation: 1000,
                current_ability: 0,
                potential_ability: 0,
                international_apps: 0,
                international_goals: 0,
                under_21_international_apps: 0,
                under_21_international_goals: 0,
                injury_days_remaining: 0,
                injury_type: None,
                injury_proneness: 10,
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
            statistics_history: PlayerStatisticsHistory::new(),
        }
    }
}
