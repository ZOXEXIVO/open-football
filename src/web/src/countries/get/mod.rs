pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::Country;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountryGetRequest {
    lang: String,
    country_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/get/index.html")]
pub struct CountryGetTemplate {
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
    pub active_tab: &'static str,
    pub country_slug: String,
    /// League sections in tier order: grouped competitions (Primera
    /// Division zones, MLS conferences) carry a heading, consecutive
    /// ungrouped divisions share an unnamed section.
    pub sections: Vec<CompetitionGroupDto>,
}

pub struct LeagueDto {
    pub slug: String,
    pub name: String,
}

/// One section on the country page: a grouped competition (heading +
/// zone/conference leagues + playoff links such as Torneo
/// Apertura/Clausura or MLS Cup Playoffs), or — with an empty name —
/// a run of ungrouped divisions.
pub struct CompetitionGroupDto {
    pub name: String,
    pub leagues: Vec<LeagueDto>,
    pub playoffs: Vec<PlayoffLinkDto>,
}

pub struct PlayoffLinkDto {
    pub slug: String,
    pub name: String,
}

pub async fn country_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountryGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let country_id = indexes
        .slug_indexes
        .get_country_by_slug(&route_params.country_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Country '{}' not found", route_params.country_slug))
        })?;

    let country: &Country = simulator_data
        .continents
        .iter()
        .flat_map(|c| &c.countries)
        .find(|country| country.id == country_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Country with ID {} not found in continents",
                country_id
            ))
        })?;

    let continent = simulator_data
        .continent(country.continent_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Continent with ID {} not found",
                country.continent_id
            ))
        })?;

    // Divisions render in tier order top to bottom: a grouped competition
    // (Primera Division zones, MLS conferences) becomes a headed section
    // at its tier's position, consecutive ungrouped divisions share an
    // unnamed one.
    let mut ordered: Vec<_> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .collect();
    ordered.sort_by_key(|l| l.settings.tier);
    let mut sections: Vec<CompetitionGroupDto> = Vec::new();
    for l in ordered {
        let dto = LeagueDto {
            slug: l.slug.clone(),
            name: l.name.clone(),
        };
        match &l.settings.league_group {
            Some(group) => match sections.iter_mut().find(|s| s.name == group.competition) {
                Some(section) => section.leagues.push(dto),
                None => sections.push(CompetitionGroupDto {
                    name: group.competition.clone(),
                    leagues: vec![dto],
                    playoffs: Vec::new(),
                }),
            },
            None => match sections.last_mut().filter(|s| s.name.is_empty()) {
                Some(section) => section.leagues.push(dto),
                None => sections.push(CompetitionGroupDto {
                    name: String::new(),
                    leagues: vec![dto],
                    playoffs: Vec::new(),
                }),
            },
        }
    }
    for section in sections.iter_mut().filter(|s| !s.name.is_empty()) {
        section.playoffs = country
            .playoffs
            .iter()
            .filter(|p| p.competition == section.name)
            .map(|p| PlayoffLinkDto {
                slug: p.league.slug.clone(),
                name: p.league.name.clone(),
            })
            .collect();
    }

    let current_path = format!(
        "/{}/countries/{}/leagues",
        route_params.lang, route_params.country_slug
    );
    let cl: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    Ok(CountryGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title: country.name.clone(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: continent.name.clone(),
        sub_title_link: format!("/{}/countries", route_params.lang),
        sub_title_country_code: String::new(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let country_playoffs: Vec<(&str, &str)> = country
                .playoffs
                .iter()
                .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                .collect();
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: &country.name,
                country_slug: &route_params.country_slug,
            };
            views::country_menu(
                &mp,
                &cl,
                country
                    .domestic_cup
                    .as_ref()
                    .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
                &country_playoffs,
                country.continent_id,
            )
        },
        lang: route_params.lang,
        i18n,
        active_tab: "leagues",
        country_slug: route_params.country_slug,
        sections,
    })
}
