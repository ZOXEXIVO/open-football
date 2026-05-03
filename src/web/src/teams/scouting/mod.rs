pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use core::shared::indexes::accessors::{
    ActiveMonitoringRow, KnownPlayerRow, MatchAssignmentRow, RecruitmentMeetingRow,
    ScoutWorkloadRow, ScoutingAssignmentRow, ScoutingReportRow, ScoutingSummary, ShadowReportRow,
    TransferRequestRow,
};
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamScoutingRequest {
    lang: String,
    team_slug: String,
}

/// Wraps the dashboard rows with template-side helpers (formatted money,
/// formatted dates) so the Askama template stays presentational.
pub struct ActiveMonitoringRowView {
    pub row: ActiveMonitoringRow,
    pub started_on_display: String,
    pub last_observed_display: String,
    pub estimated_value_display: String,
}

pub struct ScoutWorkloadRowView {
    pub row: ScoutWorkloadRow,
    pub last_observed_display: String,
}

pub struct ScoutingReportRowView {
    pub row: ScoutingReportRow,
    pub estimated_value_display: String,
}

pub struct ScoutingAssignmentRowView {
    pub row: ScoutingAssignmentRow,
    pub max_budget_display: String,
    pub min_technical_display: String,
    pub min_mental_display: String,
    pub min_physical_display: String,
}

pub struct MatchAssignmentRowView {
    pub row: MatchAssignmentRow,
    pub last_attended_display: String,
}

pub struct RecruitmentMeetingRowView {
    pub row: RecruitmentMeetingRow,
    pub date_display: String,
    pub decisions: Vec<MeetingDecisionRowView>,
    pub votes: Vec<MeetingVoteRowView>,
}

pub struct MeetingDecisionRowView {
    pub row: core::shared::indexes::accessors::MeetingDecisionRow,
    pub consensus_display: String,
    pub board_risk_display: String,
    pub budget_fit_display: String,
}

pub struct MeetingVoteRowView {
    pub row: core::shared::indexes::accessors::MeetingVoteRow,
    pub score_display: String,
}

pub struct KnownPlayerRowView {
    pub row: KnownPlayerRow,
    pub last_seen_display: String,
    pub estimated_fee_display: String,
}

pub struct ShadowReportRowView {
    pub row: ShadowReportRow,
    pub recorded_on_display: String,
}

pub struct TransferRequestRowView {
    pub row: TransferRequestRow,
    pub budget_allocation_display: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/scouting/index.html")]
pub struct TeamScoutingTemplate {
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
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,

    pub summary: ScoutingSummary,
    pub scout_workload: Vec<ScoutWorkloadRowView>,
    pub active_monitoring: Vec<ActiveMonitoringRowView>,
    pub scouting_reports: Vec<ScoutingReportRowView>,
    pub scouting_assignments: Vec<ScoutingAssignmentRowView>,
    pub match_assignments: Vec<MatchAssignmentRowView>,
    pub recruitment_meetings: Vec<RecruitmentMeetingRowView>,
    pub known_players: Vec<KnownPlayerRowView>,
    pub shadow_reports: Vec<ShadowReportRowView>,
    pub transfer_requests: Vec<TransferRequestRowView>,
}

pub async fn team_scouting_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamScoutingRequest>,
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

    let dashboard = simulator_data.club_scouting_dashboard(team.club_id);

    let scout_workload = dashboard
        .scout_workload
        .into_iter()
        .map(|r| ScoutWorkloadRowView {
            last_observed_display: r
                .last_observed
                .map(|d| d.format("%d.%m.%Y").to_string())
                .unwrap_or_default(),
            row: r,
        })
        .collect();

    let active_monitoring = dashboard
        .active_monitoring
        .into_iter()
        .map(|r| ActiveMonitoringRowView {
            started_on_display: r.started_on.format("%d.%m.%Y").to_string(),
            last_observed_display: r.last_observed.format("%d.%m.%Y").to_string(),
            estimated_value_display: format_money_or_dash(r.estimated_value),
            row: r,
        })
        .collect();

    let scouting_reports = dashboard
        .scouting_reports
        .into_iter()
        .map(|r| ScoutingReportRowView {
            estimated_value_display: format_money_or_dash(r.estimated_value),
            row: r,
        })
        .collect();

    let scouting_assignments = dashboard
        .scouting_assignments
        .into_iter()
        .map(|r| ScoutingAssignmentRowView {
            max_budget_display: format_money_or_dash(r.max_budget),
            min_technical_display: format!("{:.1}", r.min_technical_avg),
            min_mental_display: format!("{:.1}", r.min_mental_avg),
            min_physical_display: format!("{:.1}", r.min_physical_avg),
            row: r,
        })
        .collect();

    let match_assignments = dashboard
        .match_assignments
        .into_iter()
        .map(|r| MatchAssignmentRowView {
            last_attended_display: r
                .last_attended
                .map(|d| d.format("%d.%m.%Y").to_string())
                .unwrap_or_default(),
            row: r,
        })
        .collect();

    let recruitment_meetings = dashboard
        .recruitment_meetings
        .into_iter()
        .map(|r| RecruitmentMeetingRowView {
            date_display: r.date.format("%d.%m.%Y").to_string(),
            decisions: r
                .decisions
                .iter()
                .map(|d| MeetingDecisionRowView {
                    consensus_display: format!("{:.2}", d.consensus_score),
                    board_risk_display: format!("{:.2}", d.board_risk_score),
                    budget_fit_display: format!("{:.2}", d.budget_fit),
                    row: d.clone(),
                })
                .collect(),
            votes: r
                .votes
                .iter()
                .map(|v| MeetingVoteRowView {
                    score_display: format!("{:.2}", v.score),
                    row: v.clone(),
                })
                .collect(),
            row: r,
        })
        .collect();

    let known_players = dashboard
        .known_players
        .into_iter()
        .map(|r| KnownPlayerRowView {
            last_seen_display: r.last_seen.format("%d.%m.%Y").to_string(),
            estimated_fee_display: format_money_or_dash(r.estimated_fee),
            row: r,
        })
        .collect();

    let shadow_reports = dashboard
        .shadow_reports
        .into_iter()
        .map(|r| ShadowReportRowView {
            recorded_on_display: r.recorded_on.format("%d.%m.%Y").to_string(),
            row: r,
        })
        .collect();

    let transfer_requests = dashboard
        .transfer_requests
        .into_iter()
        .map(|r| TransferRequestRowView {
            budget_allocation_display: format_money_or_dash(r.budget_allocation),
            row: r,
        })
        .collect();

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

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/scouting", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams {
        i18n: &i18n,
        lang: &route_params.lang,
        current_path: &current_path,
        country_name: cn,
        country_slug: cs,
    };
    let menu_sections = views::team_menu(
        &menu_params,
        &neighbor_refs,
        &team.slug,
        &league_refs,
        team.team_type == core::TeamType::Main,
    );
    let title = team.name.clone();
    let league_title = league
        .map(|l| views::league_display_name(l, &i18n, simulator_data))
        .unwrap_or_default();

    Ok(TeamScoutingTemplate {
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
            .map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
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
        active_tab: "scouting",
        show_finances_tab: team.team_type.is_own_team(),
        show_academy_tab: team.team_type == core::TeamType::Main
            || team.team_type == core::TeamType::U18,
        summary: dashboard.summary,
        scout_workload,
        active_monitoring,
        scouting_reports,
        scouting_assignments,
        match_assignments,
        recruitment_meetings,
        known_players,
        shadow_reports,
        transfer_requests,
    })
}

/// Format money but render `0` as a dash so empty/unset values don't
/// muddy a dense table.
fn format_money_or_dash(value: f64) -> String {
    if value <= 0.0 {
        "—".to_string()
    } else {
        FormattingUtils::format_money(value)
    }
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
