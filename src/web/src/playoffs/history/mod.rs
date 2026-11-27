pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use core::league::{CupHistoryEntry, LeaguePlayoff};
use core::{Country, SimulatorData};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
pub struct PlayoffHistoryRequest {
    pub lang: String,
    pub playoff_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "playoffs/history/index.html")]
pub struct PlayoffHistoryTemplate {
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
    pub playoff_slug: String,
    pub active_tab: &'static str,
    /// Total completed editions on record.
    pub editions: usize,
    /// Distinct clubs that have lifted the trophy.
    pub distinct_winners: usize,
    /// One row per past edition, most recent first.
    pub rows: Vec<PlayoffHistoryRow>,
    /// Past Supporters' Shield winners, most recent first (MLS format only).
    pub shield_rows: Vec<PlayoffHistoryRow>,
}

/// One past edition in the roll of honour. Teams are pre-resolved to
/// display name + slug so the template only links and renders.
pub struct PlayoffHistoryRow {
    pub season_label: String,
    pub champion_name: String,
    pub champion_slug: String,
    pub runner_up_name: String,
    pub runner_up_slug: String,
    pub has_runner_up: bool,
}

impl PlayoffHistoryRow {
    fn from_entry(
        data: &SimulatorData,
        season_label: String,
        entry: &CupHistoryEntry,
    ) -> PlayoffHistoryRow {
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
        PlayoffHistoryRow {
            season_label,
            champion_name,
            champion_slug,
            runner_up_name,
            runner_up_slug,
            has_runner_up,
        }
    }
}

pub struct PlayoffHistoryPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    conference: &'a Country,
    playoff: &'a LeaguePlayoff,
}

impl<'a> PlayoffHistoryPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayoffHistoryRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let world = state.world.snapshot();
        let simulator_data = world.as_ref().unwrap();

        let league_id = simulator_data
            .indexes
            .as_ref()
            .unwrap()
            .slug_indexes
            .get_league_by_slug(&route_params.playoff_slug)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Playoff with slug {} not found",
                    route_params.playoff_slug
                ))
            })?;

        let league = simulator_data.league(league_id).unwrap();

        // The history route only serves playoffs; a normal league slug is
        // bounced to its standings page.
        if !league.is_cup {
            return Ok(Redirect::to(
                &LeagueAddress::of(simulator_data, league).url(&route_params.lang),
            )
            .into_response());
        }

        let country = simulator_data.country(league.country_id).unwrap();

        // A cup slug (domestic cup, not a playoff) belongs to the cup pages.
        let Some(playoff) = country.playoffs.iter().find(|p| p.league.id == league_id) else {
            return Ok(Redirect::to(&format!(
                "/{}/cups/{}/history",
                route_params.lang, league.slug
            ))
            .into_response());
        };

        let page = PlayoffHistoryPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            conference: country,
            playoff,
        };
        Ok(page.render().into_response())
    }

    fn render(self) -> PlayoffHistoryTemplate {
        let (rows, shield_rows, distinct_winners) = self.rows();
        let title = views::Menus::league_display_name(&self.playoff.league, &self.i18n, self.data);
        let current_path = format!("/{}/playoffs/{}", self.lang, self.playoff.league.slug);
        let menu_sections = self.menu_sections(&current_path);

        PlayoffHistoryTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: self.conference.name.clone(),
            sub_title_link: format!("/{}/countries/{}", self.lang, self.conference.slug),
            sub_title_flag: Some(SubTitleFlag::of(self.conference, &self.lang)),
            header_color: self.conference.background_color.clone(),
            foreground_color: self.conference.foreground_color.clone(),
            menu_sections,
            playoff_slug: self.playoff.league.slug.clone(),
            active_tab: "history",
            editions: rows.len(),
            distinct_winners,
            rows,
            shield_rows,
            lang: self.lang,
            i18n: self.i18n,
        }
    }

    fn rows(&self) -> (Vec<PlayoffHistoryRow>, Vec<PlayoffHistoryRow>, usize) {
        let calendar = self.playoff.league.settings.season_calendar();

        // Newest edition first.
        let mut rows: Vec<PlayoffHistoryRow> = Vec::new();
        let mut winners: HashSet<u32> = HashSet::new();
        for entry in self.playoff.past_champions.iter().rev() {
            winners.insert(entry.champion_team_id);
            rows.push(PlayoffHistoryRow::from_entry(
                self.data,
                calendar.season_opening_in(entry.season_start_year).label(),
                entry,
            ));
        }

        let shield_rows: Vec<PlayoffHistoryRow> = self
            .playoff
            .shield_history
            .iter()
            .rev()
            .map(|entry| {
                let (champion_name, champion_slug) = self
                    .data
                    .team(entry.champion_team_id)
                    .map(|d| (d.name.clone(), d.slug.clone()))
                    .unwrap_or_default();
                PlayoffHistoryRow {
                    season_label: calendar.season_opening_in(entry.season_start_year).label(),
                    champion_name,
                    champion_slug,
                    runner_up_name: String::new(),
                    runner_up_slug: String::new(),
                    has_runner_up: false,
                }
            })
            .collect();

        (rows, shield_rows, winners.len())
    }

    fn menu_sections(&self, current_path: &str) -> Vec<MenuSection> {
        let country_leagues: Vec<(&str, &str)> = self
            .conference
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.name.as_str(), l.slug.as_str()))
            .collect();
        let country_playoffs: Vec<(&str, &str)> = self
            .conference
            .playoffs
            .iter()
            .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
            .collect();
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path,
            country_name: &self.conference.name,
            country_slug: &self.conference.slug,
        };
        views::Menus::playoff_menu(
            &mp,
            &country_leagues,
            self.conference
                .domestic_cup
                .as_ref()
                .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
            &country_playoffs,
            self.conference.continent_id,
        )
    }
}
