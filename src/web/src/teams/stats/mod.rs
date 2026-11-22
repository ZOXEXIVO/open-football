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
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamStatsRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/stats/index.html")]
pub struct TeamStatsTemplate {
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
    pub players: Vec<TeamPlayerStats>,
}

pub struct TeamPlayerStats {
    pub slug: String,
    pub last_name: String,
    pub first_name: String,
    pub position: String,
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub yellow_cards: u8,
    pub red_cards: u8,
    pub shots_on_target: f32,
    pub passes: u8,
    pub tackling: f32,
    pub average_rating: String,
}

pub struct TeamStatsPage;

impl TeamStatsPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamStatsRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let guard = state.data.read().await;

        let simulator_data = guard
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
        route_params: &TeamStatsRequest,
    ) -> Result<TeamStatsTemplate, ApiError> {
        let league = team.league_id.and_then(|id| data.league(id));

        let current_path = format!("/{}/teams/{}/stats", route_params.lang, team.slug);
        let neighborhood = TeamNeighborhood::for_club(team.club_id, data, &i18n)?;
        let menu_sections = neighborhood.menu_sections(
            data,
            team.club_id,
            &i18n,
            &route_params.lang,
            &current_path,
        );
        let players = Self::players(team);
        let title = team.name.clone();
        let league_title = league
            .map(|l| views::Menus::league_display_name(l, &i18n, data))
            .unwrap_or_default();

        Ok(TeamStatsTemplate {
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
            active_tab: "stats",
            show_finances_tab: team.team_type.is_own_team(),
            show_academy_tab: team.team_type == core::TeamType::Main
                || team.team_type == core::TeamType::U18,
            newspaper_count: NewspaperCounter::count(data, team),
            players,
        })
    }

    fn players(team: &core::Team) -> Vec<TeamPlayerStats> {
        // Sort by the sample-size regressed value so a one-cap teen with
        // 8.2 raw doesn't push proven starters down the squad-stats table.
        let mut raw_players: Vec<(&core::Player, f32)> = team
            .players()
            .iter()
            .map(|p| {
                let pos = p.position().position_group();
                (*p, p.statistics.average_rating_realistic(pos))
            })
            .collect();

        raw_players.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        raw_players
            .iter()
            .map(|(p, _)| TeamPlayerStats {
                slug: p.slug(),
                last_name: p.full_name.display_last_name().to_string(),
                first_name: p.full_name.display_first_name().to_string(),
                position: p.position().get_short_name().to_string(),
                played: p.statistics.played,
                played_subs: p.statistics.played_subs,
                goals: p.statistics.goals,
                assists: p.statistics.assists,
                yellow_cards: p.statistics.yellow_cards,
                red_cards: p.statistics.red_cards,
                shots_on_target: p.statistics.shots_on_target,
                passes: p.statistics.passes,
                tackling: p.statistics.tackling,
                // Printed value is the plain mean; the ordering above is
                // what keeps a one-cap 8.2 off the top of the table.
                average_rating: p.statistics.display_average_rating(),
            })
            .collect()
    }
}
