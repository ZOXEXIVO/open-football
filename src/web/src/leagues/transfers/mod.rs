pub mod routes;

use crate::leagues::address::{LeagueAddress, LeaguePage};
use crate::leagues::newspaper::LeagueNewspaperCounter;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::slug::PlayerSlug;
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::Datelike;
use core::PlayerPositionType;
use core::league::League;
use core::transfers::{CompletedTransfer, TransferNegotiation, TransferType};
use core::utils::{DateUtils, FormattingUtils};
use core::{Country, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LeagueTransfersRequest {
    lang: String,
    country_slug: String,
    league_slug: String,
}

#[derive(Deserialize)]
pub struct SeasonQuery {
    pub season: Option<u16>,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/transfers/index.html")]
pub struct LeagueTransfersTemplate {
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
    pub league_url: String,
    /// Monthly editions on the division's shelf, for the tabbar badge.
    pub newspaper_count: usize,
    pub completed_transfers: Vec<CompletedTransferItem>,
    pub has_permanent_transfers: bool,
    pub has_loan_transfers: bool,

    pub active_negotiations: Vec<NegotiationItem>,
    pub seasons: Vec<SeasonOption>,
}

pub struct SeasonOption {
    pub year: u16,
    pub display: String,
    pub selected: bool,
}

pub struct CompletedTransferItem {
    pub player_slug: String,
    pub player_name: String,
    pub position: String,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub age: String,
    pub from_team: String,
    pub from_team_slug: String,
    pub to_team: String,
    pub to_team_slug: String,
    pub fee: String,
    pub is_loan: bool,
    pub date: String,
}

pub struct NegotiationItem {
    pub player_slug: String,
    pub player_name: String,
    pub selling_team: String,
    pub selling_team_slug: String,
    pub buying_team: String,
    pub buying_team_slug: String,
    pub offer_amount: String,
    pub status: String,
}
pub struct LeagueTransfersPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    league: &'a League,
    country: &'a Country,
}

impl<'a> LeagueTransfersPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<LeagueTransfersRequest>,
        Query(query): Query<SeasonQuery>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (league, country) = match LeagueAddress::resolve(
            simulator_data,
            &route_params.lang,
            &route_params.country_slug,
            &route_params.league_slug,
            "/transfers",
        )? {
            LeaguePage::Found(league, country) => (league, country),
            LeaguePage::Moved(response) => return Ok(response),
        };

        let page = LeagueTransfersPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            league,
            country,
        };
        Ok(page.render(query.season).into_response())
    }

    fn render(self, requested_season: Option<u16>) -> LeagueTransfersTemplate {
        let league_url = LeagueAddress::new(&self.country.slug, &self.league.slug).url(&self.lang);

        // Get team IDs directly from the league table (precise per-league filter)
        let league_team_ids: Vec<u32> = self
            .league
            .table
            .get()
            .iter()
            .map(|row| row.team_id)
            .collect();

        // Get club IDs for fallback (when from_team_id is 0, e.g. foreign transfers)
        let league_club_ids: Vec<u32> = self
            .league
            .table
            .get()
            .iter()
            .filter_map(|row| self.data.team(row.team_id).map(|t| t.club_id))
            .collect();

        let (selected_season, seasons) = self.seasons(requested_season);
        let completed_transfers =
            self.completed_transfers(selected_season, &league_team_ids, &league_club_ids);
        let active_negotiations = self.negotiations(&league_club_ids);

        let league_title = views::Menus::league_display_name(self.league, &self.i18n, self.data);

        LeagueTransfersTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: format!("{} - {}", league_title, self.i18n.t("transfers")),
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: self.country.name.clone(),
            sub_title_link: format!("/{}/countries/{}", self.lang, self.country.slug),
            sub_title_flag: Some(SubTitleFlag::of(self.country, &self.lang)),
            header_color: self.country.background_color.clone(),
            foreground_color: self.country.foreground_color.clone(),
            menu_sections: self.menu_sections(&league_url),
            league_url,
            newspaper_count: LeagueNewspaperCounter::count(self.league),
            has_permanent_transfers: completed_transfers.iter().any(|t| !t.is_loan),
            has_loan_transfers: completed_transfers.iter().any(|t| t.is_loan),
            completed_transfers,
            active_negotiations,
            seasons,
            lang: self.lang,
            i18n: self.i18n,
        }
    }

    fn seasons(&self, requested_season: Option<u16>) -> (u16, Vec<SeasonOption>) {
        // Compute current season year and available seasons
        let sim_date = self.data.date.date();
        let current_season_year = if sim_date.month() >= 8 {
            sim_date.year() as u16
        } else {
            (sim_date.year() - 1) as u16
        };

        let selected_season = requested_season.unwrap_or(current_season_year);

        let min_season_year = self
            .country
            .transfer_market
            .transfer_history
            .iter()
            .map(|t| t.season_year)
            .min()
            .unwrap_or(current_season_year);

        let seasons: Vec<SeasonOption> = (min_season_year..=current_season_year)
            .rev()
            .map(|y| SeasonOption {
                year: y,
                display: format!("{}/{}", y, (y + 1) % 100),
                selected: y == selected_season,
            })
            .collect();
        (selected_season, seasons)
    }

    /// Completed transfers involving league teams, filtered by season.
    /// Use from_team_id (specific team) when available; fall back to club_id for
    /// foreign transfers (where from_team_id is 0) and for the buying side.
    ///
    /// Position / nationality / age are read from the live player record (the
    /// transfer log only stores the player's name); the whole list is then
    /// ordered by playing position — GK, defence, midfield, attack — exactly
    /// like the squad page. Players we can no longer resolve sort to the bottom.
    fn completed_transfers(
        &self,
        selected_season: u16,
        league_team_ids: &[u32],
        league_club_ids: &[u32],
    ) -> Vec<CompletedTransferItem> {
        let mut transfer_rows: Vec<(Option<PlayerPositionType>, CompletedTransferItem)> = self
            .country
            .transfer_market
            .transfer_history
            .iter()
            .filter(|t| {
                t.season_year == selected_season
                    && t.from_club_id != 0
                    && (league_team_ids.contains(&t.from_team_id)
                        || league_club_ids.contains(&t.from_club_id)
                        || league_club_ids.contains(&t.to_club_id))
            })
            .map(|t| CompletedTransferItem::from_transfer(self, t))
            .collect();

        // Squad-style ordering: GK -> Def -> Mid -> Fwd, with unresolved players last.
        transfer_rows.sort_by(|a, b| match (a.0, b.0) {
            (Some(pa), Some(pb)) => pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        transfer_rows.into_iter().map(|(_, item)| item).collect()
    }

    fn negotiations(&self, league_club_ids: &[u32]) -> Vec<NegotiationItem> {
        self.country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| {
                league_club_ids.contains(&n.selling_club_id)
                    || league_club_ids.contains(&n.buying_club_id)
            })
            .filter(|n| {
                n.status == core::transfers::NegotiationStatus::Pending
                    || n.status == core::transfers::NegotiationStatus::Countered
            })
            .filter_map(|n| NegotiationItem::from_negotiation(self, n))
            .collect()
    }

    fn first_team_slug(&self, club_id: u32) -> String {
        // Try local country first (common case)
        self.country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.slug.clone())
            .or_else(|| {
                // Fall back to global lookup for cross-country transfers
                self.data
                    .club(club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.slug.clone())
            })
            .unwrap_or_default()
    }

    fn menu_sections(&self, league_url: &str) -> Vec<MenuSection> {
        let mut cl: Vec<(u32, &str, &str)> = self
            .country
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.id, l.name.as_str(), l.slug.as_str()))
            .collect();
        cl.sort_by_key(|(id, _, _)| *id);
        let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
        let current_path = format!("{}/transfers", league_url);
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path: &current_path,
            country_name: &self.country.name,
            country_slug: &self.country.slug,
        };
        views::Menus::league_menu(
            &mp,
            &cl_refs,
            self.country
                .domestic_cup
                .as_ref()
                .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
            &self
                .country
                .playoffs
                .iter()
                .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                .collect::<Vec<_>>(),
        )
    }
}

impl CompletedTransferItem {
    fn from_transfer(
        page: &LeagueTransfersPage<'_>,
        t: &CompletedTransfer,
    ) -> (Option<PlayerPositionType>, CompletedTransferItem) {
        let from_team_slug = page.first_team_slug(t.from_club_id);
        let to_team_slug = page.first_team_slug(t.to_club_id);

        let player = page.data.player(t.player_id);
        let position_sort = player.map(|p| p.position());
        let position = player
            .map(|p| p.positions.display_positions_compact())
            .unwrap_or_else(|| "-".to_string());
        let (country_slug, country_code, country_name) = player
            .and_then(|p| {
                page.data
                    .country(p.country_id)
                    .map(|c| (c.slug.clone(), c.code.clone(), c.name.clone()))
                    .or_else(|| {
                        page.data
                            .country_info
                            .get(&p.country_id)
                            .map(|i| (i.slug.clone(), i.code.clone(), i.name.clone()))
                    })
            })
            .unwrap_or_default();
        let age = player
            .map(|p| DateUtils::age(p.birth_date, page.data.date.date()).to_string())
            .unwrap_or_else(|| "-".to_string());

        let item = CompletedTransferItem {
            player_slug: PlayerSlug::history(page.data, t.player_id, &t.player_name),
            player_name: t.player_name.clone(),
            position,
            country_slug,
            country_code,
            country_name,
            age,
            from_team: if t.from_club_id == 0 {
                page.i18n.t("free_agent").to_string()
            } else {
                t.from_team_name.clone()
            },
            from_team_slug,
            to_team: if t.to_club_id == 0 {
                page.i18n.t("free_agent").to_string()
            } else {
                t.to_team_name.clone()
            },
            to_team_slug,
            fee: if t.fee.amount > 0.0 {
                FormattingUtils::format_money(t.fee.amount)
            } else {
                page.i18n.t("fee_free").to_string()
            },
            is_loan: matches!(&t.transfer_type, TransferType::Loan(_)),
            date: t.transfer_date.format("%d.%m.%Y").to_string(),
        };

        (position_sort, item)
    }
}

impl NegotiationItem {
    fn from_negotiation(
        page: &LeagueTransfersPage<'_>,
        n: &TransferNegotiation,
    ) -> Option<NegotiationItem> {
        let selling_club = page
            .country
            .clubs
            .iter()
            .find(|c| c.id == n.selling_club_id)?;
        let buying_club = page
            .country
            .clubs
            .iter()
            .find(|c| c.id == n.buying_club_id)?;

        // Find the player name globally — the player may have moved to another country/continent
        let player_name = page
            .data
            .player(n.player_id)
            .map(|p| p.full_name.to_string())
            .unwrap_or_else(|| page.i18n.t("unknown").to_string());

        let selling_team_slug = selling_club
            .teams
            .teams
            .first()
            .map(|t| t.slug.clone())
            .unwrap_or_default();
        let buying_team_slug = buying_club
            .teams
            .teams
            .first()
            .map(|t| t.slug.clone())
            .unwrap_or_default();
        Some(NegotiationItem {
            player_slug: PlayerSlug::history(page.data, n.player_id, &player_name),
            player_name,
            selling_team: selling_club.name.clone(),
            selling_team_slug,
            buying_team: buying_club.name.clone(),
            buying_team_slug,
            offer_amount: FormattingUtils::format_money(n.current_offer.base_fee.amount),
            status: page.i18n.t(n.status.as_i18n_key()).to_string(),
        })
    }
}
