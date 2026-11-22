pub mod routes;

use crate::countries::{CountryLookup, CountryMenu};
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::{Country, NationalTeam, NationalTeamLevel};
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
    pub match_id: String,
    pub home_goals: u8,
    pub away_goals: u8,
}

pub struct CountrySchedulePage;

impl CountrySchedulePage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<CountryScheduleRequest>,
    ) -> ApiResult<impl IntoResponse> {
        Self::render(state, route_params, NationalTeamLevel::Senior).await
    }

    pub async fn u21(
        State(state): State<GameAppData>,
        Path(route_params): Path<CountryScheduleRequest>,
    ) -> ApiResult<impl IntoResponse> {
        Self::render(state, route_params, NationalTeamLevel::Under21).await
    }

    async fn render(
        state: GameAppData,
        route_params: CountryScheduleRequest,
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
        route_params: &CountryScheduleRequest,
        level: NationalTeamLevel,
    ) -> CountryScheduleTemplate {
        let (team, team_suffix) = match level {
            NationalTeamLevel::Senior => (&country.national_team, ""),
            NationalTeamLevel::Under21 => (&country.u21_national_team, "/u21"),
        };
        let items = ScheduleItems::of(data, team);

        let current_path = format!(
            "/{}/countries/{}{}/schedule",
            route_params.lang, route_params.country_slug, team_suffix
        );

        CountryScheduleTemplate {
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
            active_tab: "schedule",
            country_slug: route_params.country_slug.clone(),
            team_suffix,
            items,
        }
    }
}

struct ScheduleItems;

impl ScheduleItems {
    fn of(data: &core::SimulatorData, team: &NationalTeam) -> Vec<CountryScheduleItem> {
        team.schedule
            .iter()
            .map(|fixture| {
                let opponent_name = if !fixture.opponent_country_name.is_empty() {
                    fixture.opponent_country_name.clone()
                } else {
                    data.country(fixture.opponent_country_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default()
                };

                let opponent_slug = data
                    .country(fixture.opponent_country_id)
                    .map(|c| c.slug.clone())
                    .unwrap_or_default();

                CountryScheduleItem {
                    date: fixture.date.format("%d.%m.%Y").to_string(),
                    opponent_name,
                    opponent_slug,
                    is_home: fixture.is_home,
                    competition_name: fixture.competition_name.clone(),
                    result: fixture.result.as_ref().map(|res| CountryScheduleResult {
                        match_id: fixture.match_id.clone(),
                        home_goals: res.home_score,
                        away_goals: res.away_score,
                    }),
                }
            })
            .collect()
    }
}
