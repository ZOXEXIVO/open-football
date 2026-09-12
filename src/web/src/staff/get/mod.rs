pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::{DateUtils, FormattingUtils};
use chrono::NaiveDate;
use core::club::mind::organs::memory::MindClock;
use core::{SimulatorData, Staff, StaffPosition};
use serde::Deserialize;
use std::cmp::Ordering;

#[derive(Deserialize)]
pub struct StaffGetRequest {
    pub lang: String,
    pub staff_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "staff/get/index.html")]
pub struct StaffGetTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub staff: StaffViewModel,
}

pub struct StaffViewModel {
    pub id: u32,
    pub role_key: String,
    pub age: u8,
    pub birth_date: String,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub team_slug: String,
    pub team_name: String,
    pub contract: Option<StaffContractDto>,
    pub coaching: StaffCoachingDto,
    pub goalkeeping: StaffGoalkeepingDto,
    pub mental: StaffMentalDto,
    pub knowledge: StaffKnowledgeDto,
    pub medical: StaffMedicalDto,
    /// The men he has worked with, warmest first.
    ///
    /// The one thing on this page that is not an attribute: everything
    /// above is what he can do, and this is who he knows.
    pub known_players: Vec<KnownPlayerDto>,
}

/// One player a manager has worked with before.
pub struct KnownPlayerDto {
    pub id: u32,
    pub name: String,
    /// Separate spells at the same or different clubs.
    pub spells: u8,
    pub matches: u16,
    /// Five-step label for how warmly he remembers him.
    pub regard_key: &'static str,
    /// Whether they are working together right now.
    pub working_together: bool,
    /// How it ended, when it has.
    pub parted_key: Option<&'static str>,
    pub medals: Vec<&'static str>,
    pub scars: Vec<&'static str>,
}

pub struct StaffContractDto {
    pub salary: String,
    pub expiration: String,
}

pub struct StaffCoachingDto {
    pub attacking: u8,
    pub defending: u8,
    pub fitness: u8,
    pub mental: u8,
    pub tactical: u8,
    pub technical: u8,
    pub working_with_youngsters: u8,
}

pub struct StaffGoalkeepingDto {
    pub distribution: u8,
    pub handling: u8,
    pub shot_stopping: u8,
}

pub struct StaffMentalDto {
    pub adaptability: u8,
    pub determination: u8,
    pub discipline: u8,
    pub man_management: u8,
    pub motivating: u8,
}

pub struct StaffKnowledgeDto {
    pub judging_player_ability: u8,
    pub judging_player_potential: u8,
    pub tactical_knowledge: u8,
}

pub struct StaffMedicalDto {
    pub physiotherapy: u8,
    pub sports_science: u8,
}

/// What a member of staff is called on his own page.
struct StaffRole;

impl StaffRole {
    fn as_i18n_key(position: &StaffPosition) -> &'static str {
        match position {
            StaffPosition::Manager => "staff_manager",
            StaffPosition::AssistantManager => "staff_assistant_manager",
            StaffPosition::CaretakerManager => "staff_caretaker_manager",
            StaffPosition::Coach => "staff_coach",
            StaffPosition::FirstTeamCoach => "staff_first_team_coach",
            StaffPosition::FitnessCoach => "staff_fitness_coach",
            StaffPosition::GoalkeeperCoach => "staff_goalkeeper_coach",
            StaffPosition::YouthCoach => "staff_youth_coach",
            StaffPosition::U21Manager => "staff_u21_manager",
            StaffPosition::U19Manager => "staff_u19_manager",
            StaffPosition::Scout => "staff_scout",
            StaffPosition::ChiefScout => "staff_chief_scout",
            StaffPosition::Physio => "staff_physio",
            StaffPosition::HeadOfPhysio => "staff_head_of_physio",
            StaffPosition::Chairman => "staff_chairman",
            StaffPosition::Director => "staff_director",
            StaffPosition::ManagingDirector => "staff_managing_director",
            StaffPosition::DirectorOfFootball => "staff_director_of_football",
            StaffPosition::GeneralManager => "staff_general_manager",
            StaffPosition::HeadOfYouthDevelopment => "staff_head_of_youth_dev",
            StaffPosition::MediaPundit => "staff_media_pundit",
            StaffPosition::DataAnalyst => "staff_data_analyst",
            StaffPosition::HeadOfRecruitment => "staff_head_of_recruitment",
            StaffPosition::Free => "staff_free",
        }
    }
}

pub async fn staff_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<StaffGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (staff, team) = simulator_data
        .staff_with_team(route_params.staff_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Staff with ID {} not found", route_params.staff_id))
        })?;

    let country = simulator_data.country(staff.country_id);
    let now = simulator_data.date.date();

    let role_key = staff
        .contract
        .as_ref()
        .map(|c| StaffRole::as_i18n_key(&c.position).to_string())
        .unwrap_or_else(|| "staff_free".to_string());

    let contract = staff.contract.as_ref().map(|c| StaffContractDto {
        salary: FormattingUtils::format_money(c.salary as f64),
        expiration: c.expired.format("%d.%m.%Y").to_string(),
    });

    let (neighbor_teams, country_leagues) =
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let title = format!(
        "{} {}",
        staff.full_name.first_name, staff.full_name.last_name
    );

    let known_players = KnownPlayers::of(staff, simulator_data, now);

    let staff_vm = StaffViewModel {
        id: staff.id,
        role_key: role_key.clone(),
        age: DateUtils::age(staff.birth_date, now),
        birth_date: staff.birth_date.format("%d.%m.%Y").to_string(),
        country_slug: country.map(|c| c.slug.clone()).unwrap_or_default(),
        country_code: country.map(|c| c.code.clone()).unwrap_or_default(),
        country_name: country.map(|c| c.name.clone()).unwrap_or_default(),
        team_slug: team.slug.clone(),
        team_name: team.name.clone(),
        contract,
        coaching: StaffCoachingDto {
            attacking: staff.staff_attributes.coaching.attacking,
            defending: staff.staff_attributes.coaching.defending,
            fitness: staff.staff_attributes.coaching.fitness,
            mental: staff.staff_attributes.coaching.mental,
            tactical: staff.staff_attributes.coaching.tactical,
            technical: staff.staff_attributes.coaching.technical,
            working_with_youngsters: staff.staff_attributes.coaching.working_with_youngsters,
        },
        goalkeeping: StaffGoalkeepingDto {
            distribution: staff.staff_attributes.goalkeeping.distribution,
            handling: staff.staff_attributes.goalkeeping.handling,
            shot_stopping: staff.staff_attributes.goalkeeping.shot_stopping,
        },
        mental: StaffMentalDto {
            adaptability: staff.staff_attributes.mental.adaptability,
            determination: staff.staff_attributes.mental.determination,
            discipline: staff.staff_attributes.mental.discipline,
            man_management: staff.staff_attributes.mental.man_management,
            motivating: staff.staff_attributes.mental.motivating,
        },
        knowledge: StaffKnowledgeDto {
            judging_player_ability: staff.staff_attributes.knowledge.judging_player_ability,
            judging_player_potential: staff.staff_attributes.knowledge.judging_player_potential,
            tactical_knowledge: staff.staff_attributes.knowledge.tactical_knowledge,
        },
        known_players,
        medical: StaffMedicalDto {
            physiotherapy: staff.staff_attributes.medical.physiotherapy,
            sports_science: staff.staff_attributes.medical.sports_science,
        },
    };

    let _league = team.league_id.and_then(|id| simulator_data.league(id));

    Ok(StaffGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: i18n.t(&role_key).to_string(),
        sub_title_suffix: if team.team_type == core::TeamType::Main {
            String::new()
        } else {
            i18n.t(team.team_type.as_i18n_key()).to_string()
        },
        sub_title: team.name.clone(),
        sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
        sub_title_country_code: String::new(),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections: {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: cn,
                country_slug: cs,
            };
            views::team_menu(&mp, &neighbor_refs, &league_refs)
        },
        i18n,
        lang: route_params.lang.clone(),
        staff: staff_vm,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let teams = views::neighbor_teams(club, i18n);

    let mut country_leagues: Vec<(u32, String, String)> = data
        .country_by_club(club_id)
        .map(|country| {
            country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);

    Ok((
        teams,
        country_leagues
            .into_iter()
            .map(|(_, name, slug)| (name, slug))
            .collect(),
    ))
}

/// The block on a manager's page that is not an attribute.
///
/// Everything else there is what he can do. This is who he knows — the
/// men he has worked with, how it went, and whether he would have them
/// again.
struct KnownPlayers;

impl KnownPlayers {
    /// Rows shown. A thirty-year career fills the store, and a page that
    /// lists a hundred and ninety names is a database dump rather than a
    /// profile. The ones cut are by construction the ones he cares least
    /// about.
    const SHOWN: usize = 24;

    /// The men he has worked with, warmest first.
    fn of(staff: &Staff, data: &SimulatorData, today: NaiveDate) -> Vec<KnownPlayerDto> {
        let day = MindClock::day(today);
        let mut rows: Vec<(f32, KnownPlayerDto)> = staff
            .dossiers
            .iter()
            .filter_map(|record| {
                // A name the page cannot resolve is a name it should not
                // print.
                let player = data
                    .player(record.player_id)
                    .or_else(|| data.retired_player(record.player_id))?;
                let warmth = record.warmth_now(day);
                Some((
                    warmth,
                    KnownPlayerDto {
                        id: record.player_id,
                        name: format!(
                            "{} {}",
                            player.full_name.first_name, player.full_name.last_name
                        ),
                        spells: record.spells,
                        matches: record.matches_together,
                        regard_key: Self::regard(warmth),
                        working_together: record.open,
                        parted_key: (!record.open).then(|| record.parted.as_i18n_key()),
                        medals: record.medals.held().map(|(_, key)| key).collect(),
                        scars: record.scars.held().map(|(_, key)| key).collect(),
                    },
                ))
            })
            .collect();

        // Warmest first, then the ones he knows best — so the top of the
        // list is the men he would ring, and the bottom is the ones he
        // would not.
        rows.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.1.matches.cmp(&a.1.matches))
        });
        rows.truncate(Self::SHOWN);
        rows.into_iter().map(|(_, row)| row).collect()
    }

    /// How warmly he remembers a man, as a word rather than a number.
    ///
    /// Five steps, because the underlying figure is a blend nobody outside
    /// the simulation should be asked to read, and because the honest
    /// answer for most players a manager has worked with is "he has no
    /// strong feelings either way".
    fn regard(warmth: f32) -> &'static str {
        if warmth >= 0.5 {
            "regard_would_sign_again"
        } else if warmth >= 0.15 {
            "regard_fondly"
        } else if warmth > -0.15 {
            "regard_no_strong_view"
        } else if warmth > -0.5 {
            "regard_reservations"
        } else {
            "regard_would_not_work_with"
        }
    }
}
