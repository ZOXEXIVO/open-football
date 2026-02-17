use chrono::{Datelike, NaiveDate, Utc};
use core::shared::FullName;
use core::utils::FloatUtils;
use core::utils::{IntegerUtils, StringUtils};
use core::{
    CoachFocus, MentalFocusType, PeopleNameGeneratorData, PersonAttributes,
    PhysicalFocusType, Staff, StaffAttributes, StaffClubContract, StaffCoaching, StaffDataAnalysis,
    StaffGoalkeeperCoaching, StaffKnowledge, StaffLicenseType, StaffMedical, StaffMental,
    StaffPosition, StaffStatus, TechnicalFocusType,
};
use rand::RngExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock};

static STAFF_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(1));

pub struct StaffGenerator {
    people_names_data: PeopleNameGeneratorData,
}

impl StaffGenerator {
    pub fn with_people_names(people_names: &PeopleNameGeneratorData) -> Self {
        StaffGenerator {
            people_names_data: PeopleNameGeneratorData {
                first_names: people_names.first_names.clone(),
                last_names: people_names.last_names.clone(),
            },
        }
    }
}

impl StaffGenerator {
    pub fn generate(&mut self, country_id: u32, position: StaffPosition, team_reputation: u16) -> Staff {
        let now = Utc::now();

        let rep_factor = (team_reputation as f32 / 10000.0).clamp(0.0, 1.0);

        let year = IntegerUtils::random(now.year() - 35, now.year() - 15) as u32;
        let month = IntegerUtils::random(1, 12) as u32;
        let day = IntegerUtils::random(1, 29) as u32;

        let salary_min = (1000.0 + rep_factor * 20000.0) as i32;
        let salary_max = (5000.0 + rep_factor * 150000.0) as i32;

        Staff::new(
            STAFF_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst),
            FullName::with_full(
                self.generate_first_name(),
                self.generate_last_name(),
                StringUtils::random_string(17),
            ),
            country_id,
            NaiveDate::from_ymd_opt(year as i32, month, day).unwrap(),
            Self::generate_staff_attributes(rep_factor),
            Some(StaffClubContract::new(
                IntegerUtils::random(salary_min, salary_max) as u32,
                NaiveDate::from_ymd_opt(now.year() + IntegerUtils::random(1, 5), 3, 14).unwrap(),
                position,
                StaffStatus::Active,
            )),
            Self::generate_person_attributes(),
            Self::generate_staff_license_type(),
            Some(Self::generate_staff_focus()),
        )
    }

    fn generate_person_attributes() -> PersonAttributes {
        PersonAttributes {
            adaptability: FloatUtils::random(0.0, 20.0),
            ambition: FloatUtils::random(0.0, 20.0),
            controversy: FloatUtils::random(0.0, 20.0),
            loyalty: FloatUtils::random(0.0, 20.0),
            pressure: FloatUtils::random(0.0, 20.0),
            professionalism: FloatUtils::random(0.0, 20.0),
            sportsmanship: FloatUtils::random(0.0, 20.0),
            temperament: FloatUtils::random(0.0, 20.0),
        }
    }

    fn generate_staff_license_type() -> StaffLicenseType {
        match IntegerUtils::random(0, 6) {
            0 => StaffLicenseType::ContinentalPro,
            1 => StaffLicenseType::ContinentalA,
            2 => StaffLicenseType::ContinentalB,
            3 => StaffLicenseType::ContinentalC,
            4 => StaffLicenseType::NationalA,
            5 => StaffLicenseType::NationalB,
            6 => StaffLicenseType::NationalC,
            _ => StaffLicenseType::NationalC,
        }
    }

    fn generate_staff_focus() -> CoachFocus {
        CoachFocus {
            technical_focus: get_random_technical(3),
            mental_focus: get_random_mental(5),
            physical_focus: get_random_physical(4),
        }
    }

    fn generate_staff_attributes(rep_factor: f32) -> StaffAttributes {
        let attr_min = (rep_factor * 8.0) as i32;
        let attr_max = (6 + (rep_factor * 14.0) as i32).min(20);

        StaffAttributes {
            coaching: StaffCoaching {
                attacking: IntegerUtils::random(attr_min, attr_max) as u8,
                defending: IntegerUtils::random(attr_min, attr_max) as u8,
                fitness: IntegerUtils::random(attr_min, attr_max) as u8,
                mental: IntegerUtils::random(attr_min, attr_max) as u8,
                tactical: IntegerUtils::random(attr_min, attr_max) as u8,
                technical: IntegerUtils::random(attr_min, attr_max) as u8,
                working_with_youngsters: IntegerUtils::random(attr_min, attr_max) as u8,
            },
            goalkeeping: StaffGoalkeeperCoaching {
                distribution: IntegerUtils::random(attr_min, attr_max) as u8,
                handling: IntegerUtils::random(attr_min, attr_max) as u8,
                shot_stopping: IntegerUtils::random(attr_min, attr_max) as u8,
            },
            mental: StaffMental {
                adaptability: IntegerUtils::random(attr_min, attr_max) as u8,
                determination: IntegerUtils::random(attr_min, attr_max) as u8,
                discipline: IntegerUtils::random(attr_min, attr_max) as u8,
                man_management: IntegerUtils::random(attr_min, attr_max) as u8,
                motivating: IntegerUtils::random(attr_min, attr_max) as u8,
            },
            knowledge: StaffKnowledge {
                judging_player_ability: IntegerUtils::random(attr_min, attr_max) as u8,
                judging_player_potential: IntegerUtils::random(attr_min, attr_max) as u8,
                tactical_knowledge: IntegerUtils::random(attr_min, attr_max) as u8,
            },
            data_analysis: StaffDataAnalysis {
                judging_player_data: IntegerUtils::random(attr_min, attr_max) as u8,
                judging_team_data: IntegerUtils::random(attr_min, attr_max) as u8,
                presenting_data: IntegerUtils::random(attr_min, attr_max) as u8,
            },
            medical: StaffMedical {
                physiotherapy: IntegerUtils::random(attr_min, attr_max) as u8,
                sports_science: IntegerUtils::random(attr_min, attr_max) as u8,
                non_player_tendencies: IntegerUtils::random(attr_min, attr_max) as u8,
            },
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

const TECHNICAL_FOCUSES: &[TechnicalFocusType] = &[
    TechnicalFocusType::Corners,
    TechnicalFocusType::Crossing,
    TechnicalFocusType::Dribbling,
    TechnicalFocusType::Finishing,
    TechnicalFocusType::FirstTouch,
    TechnicalFocusType::FreeKicks,
    TechnicalFocusType::Heading,
    TechnicalFocusType::LongShots,
    TechnicalFocusType::LongThrows,
    TechnicalFocusType::Marking,
    TechnicalFocusType::Passing,
    TechnicalFocusType::PenaltyTaking,
    TechnicalFocusType::Tackling,
    TechnicalFocusType::Technique,
];

const MENTAL_FOCUSES: &[MentalFocusType] = &[
    MentalFocusType::Aggression,
    MentalFocusType::Anticipation,
    MentalFocusType::Bravery,
    MentalFocusType::Composure,
    MentalFocusType::Concentration,
    MentalFocusType::Decisions,
    MentalFocusType::Determination,
    MentalFocusType::Flair,
    MentalFocusType::Leadership,
    MentalFocusType::OffTheBall,
    MentalFocusType::Positioning,
    MentalFocusType::Teamwork,
    MentalFocusType::Vision,
    MentalFocusType::WorkRate,
];

const PHYSICAL_FOCUSES: &[PhysicalFocusType] = &[
    PhysicalFocusType::Acceleration,
    PhysicalFocusType::Agility,
    PhysicalFocusType::Balance,
    PhysicalFocusType::Jumping,
    PhysicalFocusType::NaturalFitness,
    PhysicalFocusType::Pace,
    PhysicalFocusType::Stamina,
    PhysicalFocusType::Strength,
    PhysicalFocusType::MatchReadiness,
];

fn get_random_technical(count: usize) -> Vec<TechnicalFocusType> {
    let mut rng = rand::rng();

    let mut random_values = Vec::with_capacity(count);

    while random_values.len() < count {
        let random_index = rng.random_range(0..TECHNICAL_FOCUSES.len() - 1);
        let random_value = TECHNICAL_FOCUSES[random_index];

        if !random_values.contains(&random_value) {
            random_values.push(random_value);
        }
    }

    random_values
}

fn get_random_mental(count: usize) -> Vec<MentalFocusType> {
    let mut rng = rand::rng();

    let mut random_values = Vec::with_capacity(count);

    while random_values.len() < count {
        let random_index = rng.random_range(0..MENTAL_FOCUSES.len() - 1);
        let random_value = MENTAL_FOCUSES[random_index];

        if !random_values.contains(&random_value) {
            random_values.push(random_value);
        }
    }

    random_values
}

fn get_random_physical(count: usize) -> Vec<PhysicalFocusType> {
    let mut rng = rand::rng();

    let mut random_values = Vec::with_capacity(count);

    while random_values.len() < count {
        let random_index = rng.random_range(0..PHYSICAL_FOCUSES.len() - 1);
        let random_value = PHYSICAL_FOCUSES[random_index];

        if !random_values.contains(&random_value) {
            random_values.push(random_value);
        }
    }

    random_values
}
