pub mod routes;

use crate::countries::{CountryLookup, CountryMenu};
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Datelike;
use core::{Country, NationalTeam, NationalTeamLevel};
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
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub country_slug: String,
    pub team_suffix: &'static str,
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

impl NationalStaffDto {
    fn rows(
        data: &core::SimulatorData,
        team: &NationalTeam,
        now: chrono::NaiveDate,
    ) -> Vec<NationalStaffDto> {
        team.staff
            .iter()
            .map(|s| {
                let staff_country = data.country(s.country_id);
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
            .collect()
    }
}

pub struct CountryStaffPage;

impl CountryStaffPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<CountryStaffRequest>,
    ) -> ApiResult<impl IntoResponse> {
        Self::render(state, route_params, NationalTeamLevel::Senior).await
    }

    pub async fn u21(
        State(state): State<GameAppData>,
        Path(route_params): Path<CountryStaffRequest>,
    ) -> ApiResult<impl IntoResponse> {
        Self::render(state, route_params, NationalTeamLevel::Under21).await
    }

    async fn render(
        state: GameAppData,
        route_params: CountryStaffRequest,
        level: NationalTeamLevel,
    ) -> ApiResult<impl IntoResponse> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (country, continent) =
            CountryLookup::by_slug(simulator_data, &route_params.country_slug)?;

        Ok(Self::template(
            simulator_data,
            country,
            continent,
            i18n,
            &route_params,
            level,
        ))
    }

    fn template(
        data: &core::SimulatorData,
        country: &Country,
        continent: &core::continent::Continent,
        i18n: I18n,
        route_params: &CountryStaffRequest,
        level: NationalTeamLevel,
    ) -> CountryStaffTemplate {
        let now = data.date.date();

        let (team, team_suffix) = match level {
            NationalTeamLevel::Senior => (&country.national_team, ""),
            NationalTeamLevel::Under21 => (&country.u21_national_team, "/u21"),
        };
        let staff = NationalStaffDto::rows(data, team, now);

        let current_path = format!(
            "/{}/countries/{}{}/staff",
            route_params.lang, route_params.country_slug, team_suffix
        );

        CountryStaffTemplate {
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
            active_tab: "staff",
            country_slug: route_params.country_slug.clone(),
            team_suffix,
            staff,
        }
    }
}
