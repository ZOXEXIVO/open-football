pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use core::league::League;
use core::league::{CupHistoryEntry, DomesticCup};
use core::{Country, SimulatorData};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
pub struct CupHistoryRequest {
    pub lang: String,
    pub cup_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "cups/history/index.html")]
pub struct CupHistoryTemplate {
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
    pub cup_slug: String,
    pub active_tab: &'static str,
    /// Total completed editions on record.
    pub editions: usize,
    /// Distinct clubs that have lifted the trophy.
    pub distinct_winners: usize,
    /// One row per past edition, most recent first.
    pub rows: Vec<CupHistoryRow>,
}

/// One past edition in the roll of honour. Teams are pre-resolved to
/// display name + slug so the template only links and renders.
pub struct CupHistoryRow {
    pub season_label: String,
    pub champion_name: String,
    pub champion_slug: String,
    pub runner_up_name: String,
    pub runner_up_slug: String,
    pub has_runner_up: bool,
}

impl CupHistoryRow {
    fn from_entry(data: &SimulatorData, league: &League, entry: &CupHistoryEntry) -> CupHistoryRow {
        let (champion_name, champion_slug) = data
            .team(entry.champion_team_id)
            .map(|d| (d.name.clone(), d.slug.clone()))
            .unwrap_or_default();
        let (runner_up_name, runner_up_slug) = entry
            .runner_up_team_id
            .and_then(|id| data.team(id))
            .map(|d| (d.name.clone(), d.slug.clone()))
            .unwrap_or_default();
        let has_runner_up = !runner_up_name.is_empty();
        CupHistoryRow {
            season_label: league
                .settings
                .season_calendar()
                .season_opening_in(entry.season_start_year)
                .label(),
            champion_name,
            champion_slug,
            runner_up_name,
            runner_up_slug,
            has_runner_up,
        }
    }
}

pub struct CupHistoryPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    league: &'a League,
    country: &'a Country,
    cup: Option<&'a DomesticCup>,
}

impl<'a> CupHistoryPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<CupHistoryRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;
        let simulator_data = guard.as_ref().unwrap();

        let league_id = simulator_data
            .indexes
            .as_ref()
            .unwrap()
            .slug_indexes
            .get_league_by_slug(&route_params.cup_slug)
            .ok_or_else(|| {
                ApiError::NotFound(format!("Cup with slug {} not found", route_params.cup_slug))
            })?;

        let league = simulator_data.league(league_id).unwrap();

        // The history route only serves domestic cups; a normal league slug
        // (or any non-cup competition) is bounced to its standings page.
        if !league.is_cup {
            return Ok(Redirect::to(
                &LeagueAddress::of(simulator_data, league).url(&route_params.lang),
            )
            .into_response());
        }

        let country = simulator_data.country(league.country_id).unwrap();

        // Grouped-competition playoffs keep their history on their own page.
        if country.playoffs.iter().any(|p| p.league.id == league_id) {
            return Ok(Redirect::to(&format!(
                "/{}/playoffs/{}/history",
                route_params.lang, league.slug
            ))
            .into_response());
        }

        // Past champions live on the `DomesticCup` wrapper (outside the
        // standings collection), keyed off the same league id.
        let cup = country
            .domestic_cup
            .as_ref()
            .filter(|c| c.league.id == league_id);

        let page = CupHistoryPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            league,
            country,
            cup,
        };
        Ok(page.render().into_response())
    }

    fn render(self) -> CupHistoryTemplate {
        let (rows, distinct_winners) = self.rows();
        let title = views::Menus::league_display_name(self.league, &self.i18n, self.data);
        let current_path = format!("/{}/cups/{}", self.lang, self.league.slug);
        let menu_sections = self.menu_sections(&current_path);

        CupHistoryTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: self.country.name.clone(),
            sub_title_link: format!("/{}/countries/{}", self.lang, self.country.slug),
            sub_title_flag: Some(SubTitleFlag::of(self.country, &self.lang)),
            header_color: self.country.background_color.clone(),
            foreground_color: self.country.foreground_color.clone(),
            menu_sections,
            cup_slug: self.league.slug.clone(),
            active_tab: "history",
            editions: rows.len(),
            distinct_winners,
            rows,
            lang: self.lang,
            i18n: self.i18n,
        }
    }

    fn rows(&self) -> (Vec<CupHistoryRow>, usize) {
        let mut rows: Vec<CupHistoryRow> = Vec::new();
        let mut winners: HashSet<u32> = HashSet::new();
        if let Some(cup) = self.cup {
            // Newest edition first.
            for entry in cup.past_champions.iter().rev() {
                winners.insert(entry.champion_team_id);
                rows.push(CupHistoryRow::from_entry(self.data, self.league, entry));
            }
        }
        (rows, winners.len())
    }

    fn menu_sections(&self, current_path: &str) -> Vec<MenuSection> {
        let country_leagues: Vec<(&str, &str)> = self
            .country
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.name.as_str(), l.slug.as_str()))
            .collect();
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path,
            country_name: &self.country.name,
            country_slug: &self.country.slug,
        };
        views::Menus::cup_menu(
            &mp,
            &self.league.slug,
            &country_leagues,
            &self.league.name,
            &self
                .country
                .playoffs
                .iter()
                .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                .collect::<Vec<_>>(),
            self.country.continent_id,
        )
    }
}
