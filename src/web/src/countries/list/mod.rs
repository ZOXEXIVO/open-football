pub mod routes;

use crate::views::MenuSection;
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountryListRequest {
    lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/list/index.html")]
pub struct CountryListTemplate {
    pub css_version: &'static str,
    pub hostname: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub continents: Vec<ContinentDto>,
    pub total_countries: usize,
    pub total_clubs: usize,
    pub total_players: usize,
}

pub struct ContinentDto {
    pub name: String,
    pub countries: Vec<CountryDto>,
}

pub struct CountryDto {
    pub slug: String,
    pub code: String,
    pub name: String,
}

pub async fn country_list_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountryListRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let continents: Vec<ContinentDto> = simulator_data
        .continents
        .iter()
        .map(|continent| {
            let mut countries: Vec<CountryDto> = continent
                .countries
                .iter()
                .filter(|c| !c.leagues.leagues.is_empty())
                .map(|country| CountryDto {
                    slug: country.slug.clone(),
                    code: country.code.clone(),
                    name: country.name.clone(),
                })
                .collect();
            countries.sort_by(|a, b| a.slug.cmp(&b.slug));
            ContinentDto {
                name: continent.name.clone(),
                countries,
            }
        })
        .collect();

    let total_countries = continents.iter().map(|c| c.countries.len()).sum();
    let mut total_clubs = 0usize;
    let mut total_players = 0usize;
    for continent in &simulator_data.continents {
        for country in &continent.countries {
            total_clubs += country.clubs.len();
            for club in &country.clubs {
                for team in &club.teams.teams {
                    total_players += team.players.players.len();
                }
            }
        }
    }

    Ok(CountryListTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        hostname: &crate::common::default_handler::HOSTNAME,
        title: i18n.t("select_country").to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: i18n.t("select_country_sub").to_string(),
        sub_title_link: format!("/{}", route_params.lang),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections: vec![],
        lang: route_params.lang,
        i18n,
        continents,
        total_countries,
        total_clubs,
        total_players,
    })
}
