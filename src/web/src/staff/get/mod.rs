pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::{DateUtils, FormattingUtils};
use core::{SimulatorData, StaffPosition};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct StaffGetRequest {
    pub lang: String,
    pub staff_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "staff/get/index.html")]
pub struct StaffGetTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub staff: StaffViewModel,
}

pub struct StaffViewModel {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
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

fn position_to_i18n_key(position: &StaffPosition) -> &'static str {
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
        StaffPosition::Free => "staff_free",
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
        .map(|c| position_to_i18n_key(&c.position).to_string())
        .unwrap_or_else(|| "staff_free".to_string());

    let contract = staff.contract.as_ref().map(|c| StaffContractDto {
        salary: FormattingUtils::format_money(c.salary as f64),
        expiration: c.expired.format("%d.%m.%Y").to_string(),
    });

    let neighbor_teams: Vec<(String, String)> =
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let title = format!("{} {}", staff.full_name.first_name, staff.full_name.last_name);

    let staff_vm = StaffViewModel {
        id: staff.id,
        first_name: staff.full_name.first_name.clone(),
        last_name: staff.full_name.last_name.clone(),
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
        medical: StaffMedicalDto {
            physiotherapy: staff.staff_attributes.medical.physiotherapy,
            sports_science: staff.staff_attributes.medical.sports_science,
        },
    };

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    Ok(StaffGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title_prefix: i18n.t(&role_key).to_string(),
        sub_title_suffix: if team.team_type == core::TeamType::Main {
            String::new()
        } else {
            i18n.t(team.team_type.as_i18n_key()).to_string()
        },
        sub_title: team.name.clone(),
        sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections: views::staff_menu(
            &i18n,
            &route_params.lang,
            &neighbor_refs,
            &team.slug,
            &format!("/{}/teams/{}", &route_params.lang, &team.slug),
        ),
        i18n,
        lang: route_params.lang.clone(),
        staff: staff_vm,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<Vec<(String, String)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            (
                i18n.t(team.team_type.as_i18n_key()).to_string(),
                team.slug.clone(),
                team.reputation.world,
            )
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
