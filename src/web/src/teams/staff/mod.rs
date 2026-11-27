pub mod goalkeeping;
pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::teams::newspaper::NewspaperCounter;
use crate::teams::staff::goalkeeping::{GoalkeepingDepartmentView, GoalkeepingPanel};
use crate::teams::{TeamLookup, TeamNeighborhood};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
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
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
    pub staff_groups: Vec<StaffGroup>,
    /// The goalkeeping department, when the club has reviewed its keeper
    /// room. `None` collapses the panel entirely.
    pub goalkeeping: Option<GoalkeepingDepartmentView>,
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

pub struct TeamStaffPage;

impl TeamStaffPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamStaffRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let i18n = state.i18n.for_lang(&route_params.lang);
        let team = TeamLookup::by_slug(simulator_data, &route_params.team_slug)?;

        Self::template(simulator_data, team, i18n, &route_params)
    }

    fn template(
        data: &SimulatorData,
        team: &core::Team,
        i18n: I18n,
        route_params: &TeamStaffRequest,
    ) -> Result<TeamStaffTemplate, ApiError> {
        let league = team.league_id.and_then(|id| data.league(id));
        let now = data.date.date();
        let staff_groups = StaffGroup::collect(data, team, now);

        let current_path = format!("/{}/teams/{}/staff", route_params.lang, team.slug);
        let neighborhood = TeamNeighborhood::for_club(team.club_id, data, &i18n)?;
        let menu_sections = neighborhood.menu_sections(
            data,
            team.club_id,
            &i18n,
            &route_params.lang,
            &current_path,
        );

        let title = team.name.clone();
        let league_title = league
            .map(|l| views::Menus::league_display_name(l, &i18n, data))
            .unwrap_or_default();

        Ok(TeamStaffTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            i18n,
            lang: route_params.lang.clone(),
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: league_title,
            sub_title_link: league
                .map(|l| LeagueAddress::of(data, l).url(&route_params.lang))
                .unwrap_or_default(),
            sub_title_flag: None,
            header_color: data
                .club(team.club_id)
                .map(|c| c.colors.background.clone())
                .unwrap_or_default(),
            foreground_color: data
                .club(team.club_id)
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_default(),
            menu_sections,
            team_slug: team.slug.clone(),
            active_tab: "staff",
            show_finances_tab: team.team_type.is_own_team(),
            show_academy_tab: team.team_type == core::TeamType::Main
                || team.team_type == core::TeamType::U18,
            newspaper_count: NewspaperCounter::count(data, team),
            staff_groups,
            goalkeeping: GoalkeepingPanel::for_team(data, team),
        })
    }
}

impl StaffGroup {
    fn collect(data: &SimulatorData, team: &core::Team, now: chrono::NaiveDate) -> Vec<StaffGroup> {
        let mut groups_map: std::collections::HashMap<&str, Vec<StaffMember>> =
            std::collections::HashMap::new();

        for staff in team.staffs.iter() {
            let Some(contract) = &staff.contract else {
                continue;
            };

            if contract.position == StaffPosition::Free {
                continue;
            }

            let group_key = StaffLabels::group_key(&contract.position);
            let member = StaffMember::of(data, staff, contract, now);
            groups_map.entry(group_key).or_default().push(member);
        }

        let mut groups: Vec<StaffGroup> = groups_map
            .into_iter()
            .map(|(key, members)| StaffGroup {
                group_key: key.to_string(),
                members,
            })
            .collect();

        groups.sort_by_key(|g| StaffLabels::sort_order(&g.group_key));
        groups
    }
}

impl StaffMember {
    fn of(
        data: &SimulatorData,
        staff: &core::Staff,
        contract: &core::StaffClubContract,
        now: chrono::NaiveDate,
    ) -> Self {
        let country = data.country(staff.country_id);

        StaffMember {
            id: staff.id,
            first_name: staff.full_name.first_name.clone(),
            last_name: staff.full_name.last_name.clone(),
            role_key: StaffLabels::role_key(&contract.position).to_string(),
            country_slug: country.map(|c| c.slug.clone()).unwrap_or_default(),
            country_code: country.map(|c| c.code.clone()).unwrap_or_default(),
            country_name: country.map(|c| c.name.clone()).unwrap_or_default(),
            age: DateUtils::age(staff.birth_date, now),
            contract_end: contract.expired.format("%d.%m.%Y").to_string(),
            wage: FormattingUtils::format_money(contract.salary as f64),
        }
    }
}

struct StaffLabels;

impl StaffLabels {
    fn role_key(position: &StaffPosition) -> &'static str {
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

    fn group_key(position: &StaffPosition) -> &'static str {
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
            | StaffPosition::ChiefScout
            | StaffPosition::HeadOfRecruitment
            | StaffPosition::DataAnalyst => "staff_group_scouting",

            StaffPosition::Physio | StaffPosition::HeadOfPhysio => "staff_group_medical",

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

    fn sort_order(group_key: &str) -> u8 {
        match group_key {
            "staff_group_management" => 0,
            "staff_group_directors" => 1,
            "staff_group_coaching" => 2,
            "staff_group_scouting" => 3,
            "staff_group_medical" => 4,
            _ => 5,
        }
    }
}
