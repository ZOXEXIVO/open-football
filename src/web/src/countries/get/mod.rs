pub mod routes;

use crate::countries::{CountryLookup, CountryMenu};
use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{MenuSection, SubTitleFlag};
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
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub country_slug: String,
    pub team_suffix: &'static str,
    /// League sections in tier order: grouped competitions (Primera
    /// Division zones, MLS conferences) carry a heading, consecutive
    /// ungrouped divisions share an unnamed section.
    pub sections: Vec<CompetitionGroupDto>,
}

pub struct LeagueDto {
    pub url: String,
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

impl CompetitionGroupDto {
    fn sections(country: &Country, lang: &str) -> Vec<Self> {
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
                url: LeagueAddress::new(&country.slug, &l.slug).url(lang),
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

        sections
    }
}

pub struct CountryGetPage;

impl CountryGetPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<CountryGetRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (country, continent) =
            CountryLookup::by_slug(simulator_data, &route_params.country_slug)?;

        Ok(Self::template(country, continent, i18n, &route_params))
    }

    fn template(
        country: &Country,
        continent: &core::continent::Continent,
        i18n: I18n,
        route_params: &CountryGetRequest,
    ) -> CountryGetTemplate {
        let sections = CompetitionGroupDto::sections(country, &route_params.lang);
        let current_path = format!(
            "/{}/countries/{}/leagues",
            route_params.lang, route_params.country_slug
        );

        CountryGetTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: country.name.clone(),
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: continent.name.clone(),
            sub_title_link: format!("/{}/countries", route_params.lang),
            sub_title_flag: None,
            header_color: country.background_color.clone(),
            foreground_color: country.foreground_color.clone(),
            menu_sections: CountryMenu::sections(
                country,
                &i18n,
                &route_params.lang,
                &route_params.country_slug,
                &current_path,
            ),
            lang: route_params.lang.clone(),
            i18n,
            active_tab: "leagues",
            country_slug: route_params.country_slug.clone(),
            team_suffix: "",
            sections,
        }
    }
}
