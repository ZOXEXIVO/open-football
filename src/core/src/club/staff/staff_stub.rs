use crate::club::PersonBehaviour;
use crate::shared::fullname::FullName;
use crate::{
    CoachFocus, MentalFocusType, PersonAttributes, PhysicalFocusType, Relations, Staff,
    StaffAttributes, StaffCoaching, StaffDataAnalysis, StaffGoalkeeperCoaching, StaffKnowledge,
    StaffLicenseType, StaffMedical, StaffMental, TechnicalFocusType,
};
use chrono::NaiveDate;

#[derive(Debug)]
pub struct StaffStub;

impl StaffStub {
    pub fn default() -> Staff {
        let staff = Staff {
            id: 0,
            full_name: FullName::with_full(
                "stub".to_string(),
                "stub".to_string(),
                "stub".to_string(),
            ),
            contract: None,
            country_id: 0,
            behaviour: PersonBehaviour::default(),
            birth_date: NaiveDate::from_ymd_opt(2019, 1, 1).unwrap(),
            relations: Relations::new(),
            license: StaffLicenseType::NationalC,
            attributes: PersonAttributes {
                adaptability: 1.0f32,
                ambition: 1.0f32,
                controversy: 1.0f32,
                loyalty: 1.0f32,
                pressure: 1.0f32,
                professionalism: 1.0f32,
                sportsmanship: 1.0f32,
                temperament: 1.0f32,
            },
            staff_attributes: StaffAttributes {
                coaching: StaffCoaching {
                    attacking: 1,
                    defending: 1,
                    fitness: 1,
                    mental: 1,
                    tactical: 1,
                    technical: 1,
                    working_with_youngsters: 1,
                },
                goalkeeping: StaffGoalkeeperCoaching {
                    distribution: 1,
                    handling: 1,
                    shot_stopping: 1,
                },
                mental: StaffMental {
                    adaptability: 1,
                    determination: 1,
                    discipline: 1,
                    man_management: 1,
                    motivating: 1,
                },
                knowledge: StaffKnowledge {
                    judging_player_ability: 1,
                    judging_player_potential: 1,
                    tactical_knowledge: 1,
                },
                data_analysis: StaffDataAnalysis {
                    judging_player_data: 1,
                    judging_team_data: 1,
                    presenting_data: 1,
                },
                medical: StaffMedical {
                    physiotherapy: 1,
                    sports_science: 1,
                    non_player_tendencies: 1,
                },
            },
            focus: Some(CoachFocus {
                technical_focus: vec![
                    TechnicalFocusType::FreeKicks,
                    TechnicalFocusType::LongThrows,
                ],
                mental_focus: vec![MentalFocusType::OffTheBall, MentalFocusType::Teamwork],
                physical_focus: vec![PhysicalFocusType::NaturalFitness],
            }),
            fatigue: 0.0,
            job_satisfaction: 0.0,
            recent_performance: Default::default(),
            coaching_style: Default::default(),
            training_schedule: vec![],
        };
        staff
    }
}
