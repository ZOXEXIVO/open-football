pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Datelike;
use core::Country;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountryStaffRequest {
    lang: String,
    country_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/staff/index.html")]
pub struct CountryStaffTemplate {
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
    pub staff: Vec<NationalStaffDto>,
}

pub struct NationalStaffDto {
    pub first_name: String,
    pub last_name: String,
    pub role_key: String,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub age: u8,
}

pub async fn country_staff_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountryStaffRequest>,
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

    let now = simulator_data.date.date();

    let staff: Vec<NationalStaffDto> = country
        .national_team
        .staff
        .iter()
        .map(|s| {
            let staff_country = simulator_data.country(s.country_id);
            let age = (now.year() - s.birth_year) as u8;

            NationalStaffDto {
                first_name: s.first_name.clone(),
                last_name: s.last_name.clone(),
                role_key: s.role.as_i18n_key().to_string(),
                country_slug: staff_country.map(|c| c.slug.clone()).unwrap_or_default(),
                country_code: staff_country.map(|c| c.code.clone()).unwrap_or_default(),
                country_name: staff_country.map(|c| c.name.clone()).unwrap_or_default(),
                age,
            }
        })
        .collect();

    let current_path = format!(
        "/{}/countries/{}/staff",
        route_params.lang, route_params.country_slug
    );
    let cl: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    Ok(CountryStaffTemplate {
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
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: &country.name,
                country_slug: &route_params.country_slug,
            };
            views::country_menu(&mp, &cl)
        },
        lang: route_params.lang,
        i18n,
        active_tab: "staff",
        country_slug: route_params.country_slug,
        staff,
    })
}
