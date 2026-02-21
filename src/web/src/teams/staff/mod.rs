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
pub struct TeamStaffRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/staff/index.html")]
pub struct TeamStaffTemplate {
    pub css_version: &'static str,
    pub i18n: crate::I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub staff_groups: Vec<StaffGroup>,
}

pub struct StaffGroup {
    pub group_key: String,
    pub members: Vec<StaffMember>,
}

pub struct StaffMember {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub role_key: String,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub age: u8,
    pub contract_end: String,
    pub wage: String,
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

fn position_to_group_key(position: &StaffPosition) -> &'static str {
    match position {
        StaffPosition::Manager
        | StaffPosition::AssistantManager
        | StaffPosition::CaretakerManager => "staff_group_management",

        StaffPosition::Coach
        | StaffPosition::FirstTeamCoach
        | StaffPosition::FitnessCoach
        | StaffPosition::GoalkeeperCoach
        | StaffPosition::YouthCoach
        | StaffPosition::U21Manager
        | StaffPosition::U19Manager => "staff_group_coaching",

        StaffPosition::Scout
        | StaffPosition::ChiefScout => "staff_group_scouting",

        StaffPosition::Physio
        | StaffPosition::HeadOfPhysio => "staff_group_medical",

        StaffPosition::Chairman
        | StaffPosition::Director
        | StaffPosition::ManagingDirector
        | StaffPosition::DirectorOfFootball
        | StaffPosition::GeneralManager
        | StaffPosition::HeadOfYouthDevelopment
        | StaffPosition::MediaPundit => "staff_group_directors",

        StaffPosition::Free => "staff_group_other",
    }
}

fn group_sort_order(group_key: &str) -> u8 {
    match group_key {
        "staff_group_management" => 0,
        "staff_group_directors" => 1,
        "staff_group_coaching" => 2,
        "staff_group_scouting" => 3,
        "staff_group_medical" => 4,
        _ => 5,
    }
}

pub async fn team_staff_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamStaffRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let team_id = indexes
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));
    let now = simulator_data.date.date();

    // Build staff member DTOs grouped by position type
    let mut groups_map: std::collections::HashMap<&str, Vec<StaffMember>> =
        std::collections::HashMap::new();

    for staff in &team.staffs.staffs {
        let contract = match &staff.contract {
            Some(c) => c,
            None => continue,
        };

        if contract.position == StaffPosition::Free {
            continue;
        }

        let country = simulator_data.country(staff.country_id);
        let group_key = position_to_group_key(&contract.position);

        let member = StaffMember {
            id: staff.id,
            first_name: staff.full_name.first_name.clone(),
            last_name: staff.full_name.last_name.clone(),
            role_key: position_to_i18n_key(&contract.position).to_string(),
            country_slug: country.map(|c| c.slug.clone()).unwrap_or_default(),
            country_code: country.map(|c| c.code.clone()).unwrap_or_default(),
            country_name: country.map(|c| c.name.clone()).unwrap_or_default(),
            age: DateUtils::age(staff.birth_date, now),
            contract_end: contract.expired.format("%d.%m.%Y").to_string(),
            wage: FormattingUtils::format_money(contract.salary as f64),
        };

        groups_map.entry(group_key).or_default().push(member);
    }

    let mut staff_groups: Vec<StaffGroup> = groups_map
        .into_iter()
        .map(|(key, members)| StaffGroup {
            group_key: key.to_string(),
            members,
        })
        .collect();

    staff_groups.sort_by_key(|g| group_sort_order(&g.group_key));

    let (neighbor_teams, league_info) =
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Option<(&str, &str)> = league_info.as_ref().map(|(n, s)| (n.as_str(), s.as_str()));

    let menu_sections = views::team_menu(
        &i18n,
        &route_params.lang,
        &neighbor_refs,
        &team.slug,
        &format!("/{}/teams/{}/staff", &route_params.lang, &team.slug),
        league_refs,
    );

    let title = if team.team_type == core::TeamType::Main {
        team.name.clone()
    } else {
        format!("{} - {}", team.name, i18n.t(team.team_type.as_i18n_key()))
    };

    Ok(TeamStaffTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league
            .map(|l| l.name.clone())
            .unwrap_or_default(),
        sub_title_link: league
            .map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug))
            .unwrap_or_default(),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        staff_groups,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<(Vec<(String, String)>, Option<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut league_info: Option<(String, String)> = None;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            if team.team_type == core::TeamType::Main {
                if let Some(league_id) = team.league_id {
                    if let Some(league) = data.league(league_id) {
                        league_info = Some((league.name.clone(), league.slug.clone()));
                    }
                }
            }
            (
                format!("{} {}", club_name, i18n.t(team.team_type.as_i18n_key())),
                team.slug.clone(),
                team.reputation.world,
            )
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok((teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect(), league_info))
}
