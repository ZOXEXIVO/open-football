pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use core::league::{DomesticCup, LeagueSettings};
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
    pub sub_title_country_code: String,
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

/// Season label for a cup edition anchored at `start_year`. Autumn-spring
/// campaigns (the end month falls on or before the start month) wrap into
/// the next year and render as `2025/26`; calendar-year competitions show
/// the single year.
fn season_label(settings: &LeagueSettings, start_year: i32) -> String {
    let wraps = settings.season_ending_half.to_month <= settings.season_starting_half.from_month;
    if wraps {
        format!("{}/{:02}", start_year, (start_year + 1).rem_euclid(100))
    } else {
        start_year.to_string()
    }
}

pub async fn cup_history_action(
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
        return Ok(
            Redirect::to(&format!("/{}/leagues/{}", route_params.lang, league.slug))
                .into_response(),
        );
    }

    let country = simulator_data.country(league.country_id).unwrap();

    // Past champions live on the `DomesticCup` wrapper (outside the
    // standings collection), keyed off the same league id.
    let cup: Option<&DomesticCup> = country
        .domestic_cup
        .as_ref()
        .filter(|c| c.league.id == league_id);

    let mut rows: Vec<CupHistoryRow> = Vec::new();
    let mut winners: HashSet<u32> = HashSet::new();
    if let Some(cup) = cup {
        // Newest edition first.
        for entry in cup.past_champions.iter().rev() {
            winners.insert(entry.champion_team_id);
            let (champion_name, champion_slug) = simulator_data
                .team_data(entry.champion_team_id)
                .map(|d| (d.name.clone(), d.slug.clone()))
                .unwrap_or_default();
            let (runner_up_name, runner_up_slug) = entry
                .runner_up_team_id
                .and_then(|id| simulator_data.team_data(id))
                .map(|d| (d.name.clone(), d.slug.clone()))
                .unwrap_or_default();
            let has_runner_up = !runner_up_name.is_empty();
            rows.push(CupHistoryRow {
                season_label: season_label(&league.settings, entry.season_start_year),
                champion_name,
                champion_slug,
                runner_up_name,
                runner_up_slug,
                has_runner_up,
            });
        }
    }

    let editions = rows.len();
    let distinct_winners = winners.len();

    let title = views::league_display_name(&league, &i18n, simulator_data);
    let current_path = format!("/{}/cups/{}", &route_params.lang, &league.slug);
    let country_leagues: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    let menu_sections = {
        let mp = views::MenuParams {
            i18n: &i18n,
            lang: &route_params.lang,
            current_path: &current_path,
            country_name: &country.name,
            country_slug: &country.slug,
        };
        views::cup_menu(
            &mp,
            &league.slug,
            &country_leagues,
            &league.name,
            country.continent_id,
        )
    };

    Ok(CupHistoryTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections,
        cup_slug: league.slug.clone(),
        active_tab: "history",
        editions,
        distinct_winners,
        rows,
        lang: route_params.lang,
        i18n,
    }
    .into_response())
}
