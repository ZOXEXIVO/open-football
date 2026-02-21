pub mod routes;

use crate::views::MenuSection;
use crate::{ApiError, ApiResult, GameAppData};
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
    pub leagues: Vec<LeagueDto>,
}

pub struct LeagueDto {
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
        .ok_or_else(|| ApiError::NotFound(format!("Country '{}' not found", route_params.country_slug)))?;

    let country: &Country = simulator_data
        .continents
        .iter()
        .flat_map(|c| &c.countries)
        .find(|country| country.id == country_id)
        .ok_or_else(|| ApiError::NotFound(format!("Country with ID {} not found in continents", country_id)))?;

    let continent = simulator_data
        .continent(country.continent_id)
        .ok_or_else(|| ApiError::NotFound(format!("Continent with ID {} not found", country.continent_id)))?;

    let leagues: Vec<LeagueDto> = country
        .leagues
        .leagues
        .iter()
        .map(|l| LeagueDto {
            slug: l.slug.clone(),
            name: l.name.clone(),
        })
        .collect();

    Ok(CountryGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: country.name.clone(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: continent.name.clone(),
        sub_title_link: format!("/{}/countries", route_params.lang),
        sub_title_country_code: String::new(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: vec![],
        lang: route_params.lang,
        i18n,
        leagues,
    })
}
