pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::Country;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountryScheduleRequest {
    lang: String,
    country_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/schedule/index.html")]
pub struct CountryScheduleTemplate {
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
    pub country_slug: String,
    pub items: Vec<CountryScheduleItem>,
}

pub struct CountryScheduleItem {
    pub date: String,
    pub opponent_name: String,
    pub opponent_slug: String,
    pub is_home: bool,
    pub competition_name: String,
    pub result: Option<CountryScheduleResult>,
}

pub struct CountryScheduleResult {
    pub home_goals: u8,
    pub away_goals: u8,
}

pub async fn country_schedule_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountryScheduleRequest>,
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
            ApiError::NotFound(format!(
                "Country '{}' not found",
                route_params.country_slug
            ))
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

    let friendly_label = i18n.t("friendly").to_string();

    let items: Vec<CountryScheduleItem> = country
        .national_team
        .schedule
        .iter()
        .map(|fixture| {
            let opponent_name = simulator_data
                .country(fixture.opponent_country_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("Country {}", fixture.opponent_country_id));

            let opponent_slug = simulator_data
                .country(fixture.opponent_country_id)
                .map(|c| c.slug.clone())
                .unwrap_or_default();

            CountryScheduleItem {
                date: fixture.date.format("%d.%m.%Y").to_string(),
                opponent_name,
                opponent_slug,
                is_home: fixture.is_home,
                competition_name: friendly_label.clone(),
                result: fixture.result.as_ref().map(|res| CountryScheduleResult {
                    home_goals: res.home_score,
                    away_goals: res.away_score,
                }),
            }
        })
        .collect();

    let current_path = format!(
        "/{}/countries/{}/schedule",
        route_params.lang, route_params.country_slug
    );
    let cl: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    Ok(CountryScheduleTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: country.name.clone(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: continent.name.clone(),
        sub_title_link: format!("/{}/countries", route_params.lang),
        sub_title_country_code: String::new(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: views::country_menu(
            &i18n,
            &route_params.lang,
            &route_params.country_slug,
            &current_path,
            &cl,
        ),
        lang: route_params.lang,
        i18n,
        country_slug: route_params.country_slug,
        items,
    })
}
