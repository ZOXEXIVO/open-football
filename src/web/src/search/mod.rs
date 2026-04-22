pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

pub fn search_routes() -> axum::Router<GameAppData> {
    routes::routes()
}

#[derive(Deserialize)]
pub struct SearchPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "search/index.html")]
pub struct SearchPageTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
}

pub async fn search_page_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<SearchPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let current_path = format!("/{}/search", &route_params.lang);
    let menu_sections = views::search_menu(&i18n, &route_params.lang, &current_path);
    let title = i18n.t("search").to_string();

    Ok(SearchPageTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: String::new(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections,
    })
}

#[derive(Deserialize)]
pub struct SearchApiQuery {
    pub q: String,
}

#[derive(Serialize)]
pub struct SearchCountryDto {
    pub name: String,
    pub slug: String,
    pub code: String,
}

#[derive(Serialize)]
pub struct SearchClubDto {
    pub name: String,
    pub team_slug: String,
}

#[derive(Serialize)]
pub struct SearchPlayerDto {
    pub id: u32,
    pub slug: String,
    pub name: String,
    pub country_code: String,
    pub team_name: String,
    pub age: u8,
    pub generated: bool,
}

#[derive(Serialize)]
pub struct SearchResultsDto {
    pub countries: Vec<SearchCountryDto>,
    pub clubs: Vec<SearchClubDto>,
    pub players: Vec<SearchPlayerDto>,
}

const MAX_RESULTS_PER_KIND: usize = 15;

pub async fn search_api_action(
    State(state): State<GameAppData>,
    Query(query): Query<SearchApiQuery>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;
    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let needle = query.q.trim().to_lowercase();

    if needle.len() < 4 {
        return Ok(Json(SearchResultsDto {
            countries: Vec::new(),
            clubs: Vec::new(),
            players: Vec::new(),
        }));
    }

    let mut countries: Vec<SearchCountryDto> = Vec::new();
    let mut clubs: Vec<SearchClubDto> = Vec::new();
    let mut players: Vec<SearchPlayerDto> = Vec::new();

    let now = simulator_data.date.date();

    'outer: for continent in &simulator_data.continents {
        for country in &continent.countries {
            if countries.len() < MAX_RESULTS_PER_KIND
                && country.name.to_lowercase().contains(&needle)
            {
                countries.push(SearchCountryDto {
                    name: country.name.clone(),
                    slug: country.slug.clone(),
                    code: country.code.clone(),
                });
            }

            for club in &country.clubs {
                if clubs.len() < MAX_RESULTS_PER_KIND
                    && club.name.to_lowercase().contains(&needle)
                {
                    if let Some(main) = club.teams.main() {
                        clubs.push(SearchClubDto {
                            name: club.name.clone(),
                            team_slug: main.slug.clone(),
                        });
                    }
                }

                if players.len() >= MAX_RESULTS_PER_KIND
                    && clubs.len() >= MAX_RESULTS_PER_KIND
                    && countries.len() >= MAX_RESULTS_PER_KIND
                {
                    break 'outer;
                }

                for team in &club.teams.teams {
                    if players.len() >= MAX_RESULTS_PER_KIND {
                        break;
                    }
                    for player in team.players.players() {
                        if players.len() >= MAX_RESULTS_PER_KIND {
                            break;
                        }
                        let first = player.full_name.display_first_name();
                        let last = player.full_name.display_last_name();
                        let full = format!("{} {}", first, last);
                        if full.to_lowercase().contains(&needle) {
                            let country_code = simulator_data
                                .country(player.country_id)
                                .map(|c| c.code.clone())
                                .or_else(|| {
                                    simulator_data
                                        .country_info
                                        .get(&player.country_id)
                                        .map(|i| i.code.clone())
                                })
                                .unwrap_or_default();
                            players.push(SearchPlayerDto {
                                id: player.id,
                                slug: player.slug(),
                                name: full.trim().to_string(),
                                country_code,
                                team_name: team.name.clone(),
                                age: core::utils::DateUtils::age(player.birth_date, now),
                                generated: player.generated,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(Json(SearchResultsDto {
        countries,
        clubs,
        players,
    }))
}
