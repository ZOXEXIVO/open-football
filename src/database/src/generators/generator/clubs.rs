use crate::generators::{PlayerGenerator, StaffGenerator};
use crate::DatabaseEntity;
use core::club::academy::ClubAcademy;
use core::context::NaiveTime;
use core::shared::Location;
use core::{
    Club, ClubBoard, ClubColors, ClubFacilities, ClubFinances, ClubPhilosophy, ClubStatus,
    FacilityLevel, PlayerCollection, ReputationLevel, StaffCollection, Team, TeamReputation,
    TeamType, TrainingSchedule, TeamCollection,
};
use core::transfers::pipeline::ClubTransferPlan;
use std::str::FromStr;

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_clubs(
        country_id: u32,
        continent_id: u32,
        country_code: &str,
        country_reputation: u16,
        data: &DatabaseEntity,
        player_generator: &mut PlayerGenerator,
        staff_generator: &mut StaffGenerator,
    ) -> Vec<Club> {
        data
            .clubs
            .iter()
            .filter(|c| c.country_id == country_id)
            .map(|club| {
                // Determine philosophy from main team reputation
                let philosophy = if let Some(ref p) = club.philosophy {
                    match p.as_str() {
                        "SignToCompete" => ClubPhilosophy::SignToCompete,
                        "DevelopAndSell" => ClubPhilosophy::DevelopAndSell,
                        "LoanFocused" => ClubPhilosophy::LoanFocused,
                        _ => ClubPhilosophy::Balanced,
                    }
                } else {
                    let main_rep = club.teams.iter()
                        .find(|t| t.team_type.eq_ignore_ascii_case("main"))
                        .map(|t| t.reputation.world)
                        .unwrap_or(0);
                    match TeamReputation::new(0, 0, main_rep).level() {
                        ReputationLevel::Elite => ClubPhilosophy::SignToCompete,
                        ReputationLevel::Continental => ClubPhilosophy::Balanced,
                        ReputationLevel::National => ClubPhilosophy::Balanced,
                        _ => ClubPhilosophy::LoanFocused,
                    }
                };

                let facilities = match &club.facilities {
                    Some(f) => ClubFacilities {
                        training: FacilityLevel::from_str(&f.training),
                        youth: FacilityLevel::from_str(&f.youth),
                        academy: FacilityLevel::from_str(&f.academy),
                        recruitment: FacilityLevel::from_str(&f.recruitment),
                        average_attendance: club.average_attendance.unwrap_or(0),
                    },
                    None => ClubFacilities::default(),
                };

                // Extract facility values for youth generation before facilities is moved
                let academy_rating = facilities.academy.to_rating();
                let youth_quality = facilities.youth.multiplier();
                let academy_quality = facilities.academy.multiplier();
                let recruitment_quality = facilities.recruitment.multiplier();

                Club {
                id: club.id,
                name: club.name.clone(),
                location: Location {
                    city_id: club.location.city_id,
                },
                board: ClubBoard::new(),
                status: ClubStatus::Professional,
                finance: ClubFinances::new(club.finance.balance as i64, Vec::new()),
                academy: ClubAcademy::new(academy_rating),
                colors: ClubColors {
                    background: club.colors.background.clone(),
                    foreground: club.colors.foreground.clone(),
                },
                transfer_plan: ClubTransferPlan::new(),
                philosophy,
                facilities,
                rivals: club.rivals.clone(),
                teams: TeamCollection::new(
                    club.teams
                        .iter()
                        .map(|t| {
                            let team_rep = t.reputation.world;
                            let team_type = TeamType::from_str(&t.team_type).unwrap();

                            let team_name = match &team_type {
                                TeamType::Main => t.name.clone(),
                                _ => format!("{} {}", t.name, team_type),
                            };

                            let players = PlayerCollection::new(Self::generate_players(
                                player_generator,
                                country_id,
                                team_rep,
                                country_reputation,
                                &team_type,
                                t.league_id,
                                data,
                                academy_rating,
                                youth_quality,
                                academy_quality,
                                recruitment_quality,
                            ));

                            let staffs = StaffCollection::new(
                                Self::generate_staffs(staff_generator, country_id, continent_id, country_code, team_rep, &team_type)
                            );

                            Team::builder()
                                .id(t.id)
                                .league_id(t.league_id)
                                .club_id(club.id)
                                .name(team_name)
                                .slug(t.slug.clone())
                                .team_type(team_type)
                                .training_schedule(TrainingSchedule::new(
                                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                                ))
                                .reputation(TeamReputation::new(
                                    t.reputation.home,
                                    t.reputation.national,
                                    t.reputation.world,
                                ))
                                .players(players)
                                .staffs(staffs)
                                .build()
                                .expect("Failed to build Team")
                        })
                        .collect(),
                ),
            }})
            .collect()
    }
}
