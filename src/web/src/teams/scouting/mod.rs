pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::teams::newspaper::NewspaperCounter;
use crate::teams::{TeamLookup, TeamNeighborhood};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::shared::indexes::accessors::{
    ActiveMonitoringRow, ClubScoutingDashboard, KnownPlayerRow, MatchAssignmentRow,
    MeetingDecisionRow, MeetingVoteRow, RecruitmentMeetingRow, ScoutWorkloadRow,
    ScoutingAssignmentRow, ScoutingReportRow, ScoutingSummary, ShadowReportRow, TransferRequestRow,
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

impl ScoutWorkloadRowView {
    fn of(row: ScoutWorkloadRow) -> Self {
        Self {
            last_observed_display: row
                .last_observed
                .map(|d| d.format("%d.%m.%Y").to_string())
                .unwrap_or_default(),
            row,
        }
    }
}

impl ActiveMonitoringRowView {
    fn of(row: ActiveMonitoringRow) -> Self {
        Self {
            started_on_display: row.started_on.format("%d.%m.%Y").to_string(),
            last_observed_display: row.last_observed.format("%d.%m.%Y").to_string(),
            estimated_value_display: ScoutingMoney::or_dash(row.estimated_value),
            row,
        }
    }
}

impl ScoutingReportRowView {
    fn of(row: ScoutingReportRow) -> Self {
        Self {
            estimated_value_display: ScoutingMoney::or_dash(row.estimated_value),
            row,
        }
    }
}

impl ScoutingAssignmentRowView {
    fn of(row: ScoutingAssignmentRow) -> Self {
        Self {
            max_budget_display: ScoutingMoney::or_dash(row.max_budget),
            min_technical_display: format!("{:.1}", row.min_technical_avg),
            min_mental_display: format!("{:.1}", row.min_mental_avg),
            min_physical_display: format!("{:.1}", row.min_physical_avg),
            row,
        }
    }
}

impl MatchAssignmentRowView {
    fn of(row: MatchAssignmentRow) -> Self {
        Self {
            last_attended_display: row
                .last_attended
                .map(|d| d.format("%d.%m.%Y").to_string())
                .unwrap_or_default(),
            row,
        }
    }
}

impl RecruitmentMeetingRowView {
    fn of(row: RecruitmentMeetingRow) -> Self {
        Self {
            date_display: row.date.format("%d.%m.%Y").to_string(),
            decisions: row
                .decisions
                .iter()
                .map(|d| MeetingDecisionRowView::of(d.clone()))
                .collect(),
            votes: row
                .votes
                .iter()
                .map(|v| MeetingVoteRowView::of(v.clone()))
                .collect(),
            row,
        }
    }
}

impl MeetingDecisionRowView {
    fn of(row: MeetingDecisionRow) -> Self {
        Self {
            consensus_display: format!("{:.2}", row.consensus_score),
            board_risk_display: format!("{:.2}", row.board_risk_score),
            budget_fit_display: format!("{:.2}", row.budget_fit),
            row,
        }
    }
}

impl MeetingVoteRowView {
    fn of(row: MeetingVoteRow) -> Self {
        Self {
            score_display: format!("{:.2}", row.score),
            row,
        }
    }
}

impl KnownPlayerRowView {
    fn of(row: KnownPlayerRow) -> Self {
        Self {
            last_seen_display: row.last_seen.format("%d.%m.%Y").to_string(),
            estimated_fee_display: ScoutingMoney::or_dash(row.estimated_fee),
            row,
        }
    }
}

impl ShadowReportRowView {
    fn of(row: ShadowReportRow) -> Self {
        Self {
            recorded_on_display: row.recorded_on.format("%d.%m.%Y").to_string(),
            row,
        }
    }
}

impl TransferRequestRowView {
    fn of(row: TransferRequestRow) -> Self {
        Self {
            budget_allocation_display: ScoutingMoney::or_dash(row.budget_allocation),
            row,
        }
    }
}

struct ScoutingDashboardView {
    summary: ScoutingSummary,
    scout_workload: Vec<ScoutWorkloadRowView>,
    active_monitoring: Vec<ActiveMonitoringRowView>,
    scouting_reports: Vec<ScoutingReportRowView>,
    scouting_assignments: Vec<ScoutingAssignmentRowView>,
    match_assignments: Vec<MatchAssignmentRowView>,
    recruitment_meetings: Vec<RecruitmentMeetingRowView>,
    known_players: Vec<KnownPlayerRowView>,
    shadow_reports: Vec<ShadowReportRowView>,
    transfer_requests: Vec<TransferRequestRowView>,
}

impl ScoutingDashboardView {
    fn of(dashboard: ClubScoutingDashboard) -> Self {
        Self {
            summary: dashboard.summary,
            scout_workload: dashboard
                .scout_workload
                .into_iter()
                .map(ScoutWorkloadRowView::of)
                .collect(),
            active_monitoring: dashboard
                .active_monitoring
                .into_iter()
                .map(ActiveMonitoringRowView::of)
                .collect(),
            scouting_reports: dashboard
                .scouting_reports
                .into_iter()
                .map(ScoutingReportRowView::of)
                .collect(),
            scouting_assignments: dashboard
                .scouting_assignments
                .into_iter()
                .map(ScoutingAssignmentRowView::of)
                .collect(),
            match_assignments: dashboard
                .match_assignments
                .into_iter()
                .map(MatchAssignmentRowView::of)
                .collect(),
            recruitment_meetings: dashboard
                .recruitment_meetings
                .into_iter()
                .map(RecruitmentMeetingRowView::of)
                .collect(),
            known_players: dashboard
                .known_players
                .into_iter()
                .map(KnownPlayerRowView::of)
                .collect(),
            shadow_reports: dashboard
                .shadow_reports
                .into_iter()
                .map(ShadowReportRowView::of)
                .collect(),
            transfer_requests: dashboard
                .transfer_requests
                .into_iter()
                .map(TransferRequestRowView::of)
                .collect(),
        }
    }
}

pub struct TeamScoutingPage;

impl TeamScoutingPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamScoutingRequest>,
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
        data: &core::SimulatorData,
        team: &core::Team,
        i18n: I18n,
        route_params: &TeamScoutingRequest,
    ) -> Result<TeamScoutingTemplate, ApiError> {
        let league = team.league_id.and_then(|id| data.league(id));
        let dashboard = ScoutingDashboardView::of(data.club_scouting_dashboard(team.club_id));

        let current_path = format!("/{}/teams/{}/scouting", route_params.lang, team.slug);
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
            active_tab: "scouting",
            show_finances_tab: team.team_type.is_own_team(),
            show_academy_tab: team.team_type == core::TeamType::Main
                || team.team_type == core::TeamType::U18,
            newspaper_count: NewspaperCounter::count(data, team),
            summary: dashboard.summary,
            scout_workload: dashboard.scout_workload,
            active_monitoring: dashboard.active_monitoring,
            scouting_reports: dashboard.scouting_reports,
            scouting_assignments: dashboard.scouting_assignments,
            match_assignments: dashboard.match_assignments,
            recruitment_meetings: dashboard.recruitment_meetings,
            known_players: dashboard.known_players,
            shadow_reports: dashboard.shadow_reports,
            transfer_requests: dashboard.transfer_requests,
        })
    }
}

/// Format money but render `0` as a dash so empty/unset values don't
/// muddy a dense table.
struct ScoutingMoney;

impl ScoutingMoney {
    fn or_dash(value: f64) -> String {
        if value <= 0.0 {
            "—".to_string()
        } else {
            FormattingUtils::format_money(value)
        }
    }
}
