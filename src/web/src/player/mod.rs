pub mod actions;
pub mod awards;
pub mod contract;
pub mod events;
pub mod get;
pub mod history;
pub mod matches;
pub mod newspaper;
pub mod personal;
pub mod relations;
pub mod transfers;

pub use get::PlayerStatusDto;

use crate::player::events::PlayerEventsCounter;
use crate::player::newspaper::PlayerNewsCounter;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection, NeighborMenus, SubTitleFlag};
use crate::watchlist::key::WatchKey;
use crate::{ApiError, GameAppData, I18n};
use axum::Router;
use core::{Player, SimulatorData, Team};

pub struct PlayerRoutes;

impl PlayerRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new()
            .merge(get::routes::PlayerGetRoutes::create())
            .merge(contract::routes::PlayerContractRoutes::create())
            .merge(personal::routes::PlayerPersonalRoutes::create())
            .merge(events::routes::PlayerEventsRoutes::create())
            .merge(awards::routes::PlayerAwardsRoutes::create())
            .merge(matches::routes::PlayerMatchesRoutes::create())
            .merge(transfers::routes::PlayerTransfersRoutes::create())
            .merge(relations::routes::PlayerRelationsRoutes::create())
            .merge(history::routes::PlayerHistoryRoutes::create())
            .merge(newspaper::routes::PlayerNewspaperRoutes::create())
            .merge(actions::routes::PlayerActionsRoutes::create())
    }
}

/// The routes entry point keeps its historical callable name for the server
/// router while the implementation lives on the owning type.
#[allow(non_upper_case_globals)]
pub const player_routes: fn() -> Router<GameAppData> = PlayerRoutes::create;

/// The neighbour menus every player page shows: the club's teams and the
/// leagues of the country it plays in.
pub struct PlayerPageMenus;

impl PlayerPageMenus {
    pub fn neighbor_teams(
        club_id: u32,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> Result<NeighborMenus, ApiError> {
        let club = data.club(club_id).ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", club_id))
        })?;

        let teams = views::Menus::neighbor_teams(club, i18n);

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
}

/// Colour fallbacks differ on the overview page, which leaves the header
/// blank when the club colours cannot be resolved.
#[derive(Clone, Copy)]
pub enum PlayerPageColors {
    Grey,
    Empty,
}

/// The chrome shared by every player page: title block, club colours,
/// neighbour menu, tab identity and the counts the tabbar reads.
pub struct PlayerPageChrome {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub player_slug: String,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub watch_key: String,
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
}

impl PlayerPageChrome {
    pub fn build(
        data: &SimulatorData,
        i18n: I18n,
        player: &Player,
        team: Option<&Team>,
        lang: &str,
        player_slug: &str,
        active_tab: &'static str,
        colors: PlayerPageColors,
    ) -> Result<Self, ApiError> {
        let (neighbor_teams, country_leagues) = match team {
            Some(team) => PlayerPageMenus::neighbor_teams(team.club_id, data, &i18n)?,
            None => (Vec::new(), Vec::new()),
        };
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
            player.full_name.display_first_name(),
            player.full_name.display_last_name()
        );
        let (header_fallback, foreground_fallback) = match colors {
            PlayerPageColors::Grey => ("#808080", "#ffffff"),
            PlayerPageColors::Empty => ("", ""),
        };
        let header_color = team
            .and_then(|t| data.club(t.club_id).map(|c| c.colors.background.clone()))
            .unwrap_or_else(|| header_fallback.to_string());
        let foreground_color = team
            .and_then(|t| data.club(t.club_id).map(|c| c.colors.foreground.clone()))
            .unwrap_or_else(|| foreground_fallback.to_string());
        let menu_sections = if let Some(team) = team {
            let (country_name, country_slug) = views::Menus::club_country_info(data, team.club_id);
            let current_path = format!("/{}/teams/{}", lang, team.slug);
            let params = views::MenuParams {
                i18n: &i18n,
                lang,
                current_path: &current_path,
                country_name,
                country_slug,
            };
            views::Menus::team_menu(&params, &neighbor_refs, &league_refs)
        } else {
            Vec::new()
        };

        Ok(Self {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
            sub_title_suffix: String::new(),
            sub_title: team.map(|t| t.name.clone()).unwrap_or_else(|| {
                if player.is_retired() {
                    i18n.t("retired").to_string()
                } else {
                    i18n.t("free_agent").to_string()
                }
            }),
            sub_title_link: team
                .map(|t| format!("/{}/teams/{}", lang, t.slug))
                .unwrap_or_default(),
            sub_title_flag: None,
            header_color,
            foreground_color,
            menu_sections,
            i18n,
            lang: lang.to_string(),
            active_tab,
            player_id: player.id,
            player_slug: player_slug.to_string(),
            club_id: team.map(|t| t.club_id).unwrap_or(0),
            is_on_loan: player.is_on_loan(),
            is_injured: player.player_attributes.is_injured,
            is_unhappy: player.statuses.get().contains(&core::PlayerStatusType::Unh),
            is_force_match_selection: player.is_force_match_selection,
            watch_key: WatchKey::of(player).to_string(),
            events_count: PlayerEventsCounter::count(player),
            interested_clubs_count: data.clubs_interested_in_player(player.id).len(),
            awards_count: player.awards_count.total(),
            news_count: PlayerNewsCounter::count(data, player),
        })
    }

    /// A retired or free-agent page reports no club activity even if the
    /// player record still carries transient flags.
    pub fn without_club_activity(mut self) -> Self {
        self.is_on_loan = false;
        self.is_injured = false;
        self.is_unhappy = false;
        self
    }
}
