use crate::DatabaseEntity;
use crate::generators::{PlayerGenerator, StaffGenerator};
use crate::loaders::OdbPlayer;
use chrono::{Datelike, Utc};
use core::club::academy::ClubAcademy;
use core::context::NaiveTime;
use core::shared::Location;
use core::transfers::pipeline::ClubTransferPlan;
use core::{
    Club, ClubBoard, ClubColors, ClubFacilities, ClubFinances, ClubPhilosophy, ClubStatus,
    FacilityLevel, Player, PlayerCollection, ReputationLevel, StaffCollection, Team,
    TeamCollection, TeamReputation, TeamType, TrainingSchedule,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::str::FromStr;

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_clubs(
        country_id: u32,
        continent_id: u32,
        country_code: &str,
        country_reputation: u16,
        data: &DatabaseEntity,
        player_generator: &PlayerGenerator,
        staff_generator: &StaffGenerator,
    ) -> Vec<Club> {
        let odb = data.players_odb.as_ref();
        let now_year = Utc::now().date_naive().year();

        // Parallelise club construction: each club hydrates or generates 25-200
        // players across 1-5 teams plus 10-15 staff. Work is dominated by
        // player skill generation (CPU-bound, no I/O) and is fully independent
        // per club, so par_iter scales near-linearly with cores. The RNG is
        // thread-local (see core::utils::random::engine), and both generators
        // now take &self, so no further synchronisation is needed.
        data.clubs
            .par_iter()
            .filter(|c| c.country_id == country_id)
            .map(|club| {
                // Pre-distribute ODB players for this club into TeamType buckets.
                // If the club has any ODB players, fake generation is skipped for
                // every senior team (Main/Reserve/B/U20/U21/U23). Academy teams
                // (U18/U19) always go through the existing generator regardless,
                // because youth intake is owned by the academy system.
                let odb_for_club: Option<HashMap<TeamType, Vec<OdbPlayer>>> = odb
                    .and_then(|o| o.for_club(club.id))
                    .filter(|players| !players.is_empty())
                    .map(|players| {
                        let available_team_types: Vec<TeamType> = club
                            .teams
                            .iter()
                            .filter_map(|t| TeamType::from_str(&t.team_type).ok())
                            .collect();
                        distribute_odb_players_by_age(players, &available_team_types, now_year)
                    });

                // Determine philosophy from main team reputation
                let philosophy = if let Some(ref p) = club.philosophy {
                    match p.as_str() {
                        "SignToCompete" => ClubPhilosophy::SignToCompete,
                        "DevelopAndSell" => ClubPhilosophy::DevelopAndSell,
                        "LoanFocused" => ClubPhilosophy::LoanFocused,
                        _ => ClubPhilosophy::Balanced,
                    }
                } else {
                    let main_rep = club
                        .teams
                        .iter()
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

                                // Main and the senior reserves (B, Second) carry
                                // their full canonical name in the data
                                // ("Spartak Moscow", "Spartak Moscow 2", "Real
                                // Sociedad B"). Other sub-types (Reserve, U18..U23)
                                // get their short type label appended at runtime.
                                let team_name = match &team_type {
                                    TeamType::Main | TeamType::Second | TeamType::B => {
                                        t.name.clone()
                                    }
                                    _ => format!("{} {}", t.name, team_type),
                                };

                                let players = PlayerCollection::new(build_team_players(
                                    player_generator,
                                    country_id,
                                    continent_id,
                                    country_code,
                                    team_rep,
                                    country_reputation,
                                    &team_type,
                                    t.league_id,
                                    data,
                                    academy_rating,
                                    youth_quality,
                                    academy_quality,
                                    recruitment_quality,
                                    odb_for_club.as_ref(),
                                ));

                                let staffs = StaffCollection::new(Self::generate_staffs(
                                    staff_generator,
                                    country_id,
                                    continent_id,
                                    country_code,
                                    team_rep,
                                    &team_type,
                                ));

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
                }
            })
            .collect()
    }
}

/// Choose ODB-backed players if available for a senior team, otherwise fall
/// back to the procedural generator. U18/U19 squads always go through the
/// academy path — those players are owned by the youth/intake system.
fn build_team_players(
    player_generator: &PlayerGenerator,
    country_id: u32,
    continent_id: u32,
    country_code: &str,
    team_reputation: u16,
    country_reputation: u16,
    team_type: &TeamType,
    league_id: Option<u32>,
    data: &DatabaseEntity,
    academy_level: u8,
    youth_quality: f32,
    academy_quality: f32,
    recruitment_quality: f32,
    odb_for_club: Option<&HashMap<TeamType, Vec<OdbPlayer>>>,
) -> Vec<Player> {
    // If the club has any ODB players, it is fully ODB-backed: hydrate every
    // team (including U18/U19) exclusively from the file and skip synthetic
    // generation entirely. Buckets without ODB players for this team type
    // return an empty squad — we do not mix loaded and generated players.
    if let Some(buckets) = odb_for_club {
        return buckets
            .get(team_type)
            .map(|records| {
                // ODB hydration is per-record skill generation — the same
                // CPU-bound pipeline as procedural players. Parallelise the
                // per-record mapping so large squads (main teams carry 25+
                // records) don't serialise one whole club's hydration on a
                // single thread.
                records
                    .par_iter()
                    .map(|r| {
                        PlayerGenerator::generate_from_odb(r, continent_id, country_code, data)
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    // Academy teams for clubs without ODB data fall back to the academy
    // generator — youth intake is owned by the academy system.
    if matches!(team_type, TeamType::U18 | TeamType::U19) {
        return DatabaseGenerator::generate_players(
            player_generator,
            country_id,
            team_reputation,
            country_reputation,
            team_type,
            league_id,
            data,
            academy_level,
            youth_quality,
            academy_quality,
            recruitment_quality,
        );
    }

    // No ODB data for this club — original synthetic path.
    DatabaseGenerator::generate_players(
        player_generator,
        country_id,
        team_reputation,
        country_reputation,
        team_type,
        league_id,
        data,
        academy_level,
        youth_quality,
        academy_quality,
        recruitment_quality,
    )
}

/// Bucket ODB players into the senior team types the club actually has,
/// prioritising the youngest team they fit into. Players too old or whose
/// preferred bucket doesn't exist fall through to Main.
fn distribute_odb_players_by_age(
    players: &[OdbPlayer],
    available: &[TeamType],
    now_year: i32,
) -> HashMap<TeamType, Vec<OdbPlayer>> {
    let has = |tt: TeamType| available.iter().any(|t| *t == tt);
    let mut out: HashMap<TeamType, Vec<OdbPlayer>> = HashMap::new();

    for p in players {
        // Compiler-set hint pins a player to a specific bucket (e.g. squad
        // folded in from a satellite "B-team" directory). Honour it whenever
        // the parent club actually has that bucket; otherwise fall through
        // to age-based placement.
        let hinted = p
            .team_type_hint
            .as_deref()
            .and_then(|s| TeamType::from_str(s).ok())
            .filter(|tt| has(*tt));
        let target = if let Some(tt) = hinted {
            tt
        } else {
            let age = now_year - p.birth_date.year();
            if age <= 18 && has(TeamType::U18) {
                TeamType::U18
            } else if age <= 19 && has(TeamType::U19) {
                TeamType::U19
            } else if age <= 20 && has(TeamType::U20) {
                TeamType::U20
            } else if age <= 21 && has(TeamType::U21) {
                TeamType::U21
            } else if age <= 23 && has(TeamType::U23) {
                TeamType::U23
            } else if has(TeamType::Main) {
                TeamType::Main
            } else if has(TeamType::B) {
                TeamType::B
            } else if has(TeamType::Second) {
                TeamType::Second
            } else if has(TeamType::Reserve) {
                TeamType::Reserve
            } else {
                // No senior team at all — drop into the first available bucket so
                // the player isn't silently lost.
                *available
                    .iter()
                    .find(|t| !matches!(t, TeamType::U18 | TeamType::U19))
                    .unwrap_or(&TeamType::Main)
            }
        };
        out.entry(target).or_default().push(p.clone());
    }

    out
}
