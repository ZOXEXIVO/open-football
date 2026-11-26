pub mod key;
pub mod routes;
#[cfg(test)]
mod tests;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::potential_stars::{PotentialStarsView, StarRating};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::watchlist::key::WatchKey;
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::NaiveDate;
use core::Player;
use core::PlayerPositionType;
use core::PlayerStatusType;
use core::SimulatorData;
use core::Team;
use core::TeamType;
use core::utils::{DateUtils, FormattingUtils};
use serde::Deserialize;
use std::collections::HashSet;

pub struct WatchlistPlayerDto {
    pub watch_key: String,
    pub slug: String,
    pub last_name: String,
    pub first_name: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub age: u8,
    pub current_ability: StarRating,
    pub potential_ability: StarRating,
    pub conditions: u8,
    pub team_name: String,
    pub team_slug: String,
    pub league_name: String,
    pub league_url: String,
    pub played: u16,
    pub played_subs: u16,
    pub value: String,
    pub injured: bool,
    pub unhappy: bool,
    pub transfer_listed: bool,
    pub retired: bool,
}

#[derive(Deserialize)]
pub struct WatchlistPageRequest {
    pub lang: String,
}

#[derive(Deserialize)]
pub struct WatchlistRowsRequest {
    pub keys: Vec<String>,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "watchlist/index.html")]
pub struct WatchlistPageTemplate {
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
}

impl WatchlistPageTemplate {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<WatchlistPageRequest>,
    ) -> ApiResult<impl IntoResponse> {
        if state.data.read().await.is_none() {
            return Err(ApiError::InternalError(
                "Simulator data not loaded".to_string(),
            ));
        }

        let i18n = state.i18n.for_lang(&route_params.lang);
        let current_path = format!("/{}/watchlist", route_params.lang);
        let menu_sections = views::Menus::watchlist_menu(&i18n, &route_params.lang, &current_path);

        Ok(Self {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: i18n.t("watchlist").to_string(),
            i18n,
            lang: route_params.lang,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: String::new(),
            sub_title_link: String::new(),
            sub_title_flag: None,
            header_color: String::new(),
            foreground_color: String::new(),
            menu_sections,
        })
    }
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "watchlist/rows.html")]
pub struct WatchlistRowsTemplate {
    pub i18n: I18n,
    pub lang: String,
    pub players: Vec<WatchlistPlayerDto>,
}

impl WatchlistRowsTemplate {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<WatchlistPageRequest>,
        Json(body): Json<WatchlistRowsRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let guard = state.data.read().await;
        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let i18n = state.i18n.for_lang(&route_params.lang);
        let players = WatchlistRows::resolve(simulator_data, &body.keys, &i18n, &route_params.lang);

        Ok(Self {
            i18n,
            lang: route_params.lang,
            players,
        })
    }
}

/// Resolves browser-held watch keys against the current world. A key yields
/// a row only when its id still belongs to the same person.
pub struct WatchlistRows<'a> {
    data: &'a SimulatorData,
    i18n: &'a I18n,
    lang: &'a str,
    now: NaiveDate,
}

impl<'a> WatchlistRows<'a> {
    pub fn resolve(
        data: &'a SimulatorData,
        keys: &[String],
        i18n: &'a I18n,
        lang: &'a str,
    ) -> Vec<WatchlistPlayerDto> {
        let rows = WatchlistRows {
            data,
            i18n,
            lang,
            now: data.date.date(),
        };
        let mut seen = HashSet::new();
        let mut players: Vec<WatchlistPlayerDto> = keys
            .iter()
            .filter_map(|raw| raw.parse::<WatchKey>().ok())
            .filter(|key| seen.insert(*key))
            .filter_map(|key| rows.row(key))
            .collect();
        players.sort_by(|a, b| {
            a.position_sort
                .partial_cmp(&b.position_sort)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        players
    }

    fn row(&self, key: WatchKey) -> Option<WatchlistPlayerDto> {
        if let Some((player, team)) = self.data.player_with_team(key.id) {
            return key.matches(player).then(|| self.on_team(player, team));
        }
        let player = self
            .data
            .player(key.id)
            .filter(|player| key.matches(player))?;
        Some(if player.is_retired() {
            self.retired(player)
        } else {
            self.teamless(player)
        })
    }

    fn on_team(&self, player: &Player, team: &Team) -> WatchlistPlayerDto {
        let league = team.league_id.and_then(|id| self.data.league(id));
        WatchlistPlayerDto {
            potential_ability: PotentialStarsView::potential_by_staff(
                player,
                team.staffs.head_coach(),
                team.team_type == TeamType::Main,
                self.now,
            ),
            team_name: team.name.clone(),
            team_slug: team.slug.clone(),
            league_name: league.map(|l| l.name.clone()).unwrap_or_default(),
            league_url: league
                .map(|l| LeagueAddress::of(self.data, l).url(self.lang))
                .unwrap_or_default(),
            value: FormattingUtils::format_money(player.value(
                self.now,
                league.map(|l| l.reputation).unwrap_or(0),
                team.reputation.market_value_score(),
            )),
            unhappy: !player.happiness.is_happy(),
            transfer_listed: player.statuses.get().contains(&PlayerStatusType::Lst),
            ..WatchlistPlayerDto::from_player(player, self.data, self.now)
        }
    }

    fn retired(&self, player: &Player) -> WatchlistPlayerDto {
        WatchlistPlayerDto {
            conditions: 0,
            team_name: self.i18n.t("retired").to_string(),
            retired: true,
            ..WatchlistPlayerDto::from_player(player, self.data, self.now)
        }
    }

    fn teamless(&self, player: &Player) -> WatchlistPlayerDto {
        WatchlistPlayerDto {
            team_name: self.i18n.t("free_agent").to_string(),
            ..WatchlistPlayerDto::from_player(player, self.data, self.now)
        }
    }
}

impl WatchlistPlayerDto {
    /// DTO filled from fields that don't depend on the player's current team.
    /// Caller overrides team/league/value/potential/etc via struct-update syntax.
    fn from_player(player: &Player, simulator_data: &SimulatorData, now: NaiveDate) -> Self {
        let (country_code, country_name, country_slug) = simulator_data
            .country(player.country_id)
            .map(|c| (c.code.clone(), c.name.clone(), c.slug.clone()))
            .or_else(|| {
                simulator_data
                    .country_info
                    .get(&player.country_id)
                    .map(|i| (i.code.clone(), i.name.clone(), i.slug.clone()))
            })
            .unwrap_or_default();

        Self {
            watch_key: WatchKey::of(player).to_string(),
            slug: player.slug(),
            first_name: player.full_name.display_first_name().to_string(),
            last_name: player.full_name.display_last_name().to_string(),
            position: player.positions.display_positions_compact(),
            position_sort: player.position(),
            country_code,
            country_name,
            country_slug,
            age: DateUtils::age(player.birth_date, now),
            current_ability: PotentialStarsView::current(player),
            potential_ability: PotentialStarsView::potential_absolute(player, now),
            conditions: Self::conditions(player),
            team_name: String::new(),
            team_slug: String::new(),
            league_name: String::new(),
            league_url: String::new(),
            played: player.statistics.played,
            played_subs: player.statistics.played_subs,
            value: "-".to_string(),
            injured: player.player_attributes.is_injured,
            unhappy: false,
            transfer_listed: false,
            retired: false,
        }
    }

    fn conditions(player: &Player) -> u8 {
        (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
    }
}
