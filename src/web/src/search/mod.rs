pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
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
    pub is_free_agent: bool,
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

    let mut countries: Vec<SearchCountryDto> = Vec::with_capacity(MAX_RESULTS_PER_KIND);
    let mut clubs: Vec<(u16, SearchClubDto)> = Vec::with_capacity(MAX_RESULTS_PER_KIND);
    let mut players: Vec<(u8, SearchPlayerDto)> = Vec::with_capacity(MAX_RESULTS_PER_KIND);

    let now = simulator_data.date.date();

    for continent in &simulator_data.continents {
        for country in &continent.countries {
            if country.name.to_lowercase().contains(&needle) {
                countries.push(SearchCountryDto {
                    name: country.name.clone(),
                    slug: country.slug.clone(),
                    code: country.code.clone(),
                });
            }

            for club in &country.clubs {
                if club.name.to_lowercase().contains(&needle) {
                    if let Some(main) = club.teams.main() {
                        clubs.push((
                            main.reputation.world,
                            SearchClubDto {
                                name: club.name.clone(),
                                team_slug: main.slug.clone(),
                            },
                        ));
                    }
                }

                for team in &club.teams.teams {
                    for player in team.players.players() {
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
                            players.push((
                                player.player_attributes.current_ability,
                                SearchPlayerDto {
                                    id: player.id,
                                    slug: player.slug(),
                                    name: full.trim().to_string(),
                                    country_code,
                                    team_name: team.name.clone(),
                                    age: core::utils::DateUtils::age(player.birth_date, now),
                                    generated: player.is_generated(),
                                    is_free_agent: false,
                                },
                            ));
                        }
                    }
                }
            }
        }
    }

    for player in &simulator_data.free_agents {
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
            players.push((
                player.player_attributes.current_ability,
                SearchPlayerDto {
                    id: player.id,
                    slug: player.slug(),
                    name: full.trim().to_string(),
                    country_code,
                    team_name: String::new(),
                    age: core::utils::DateUtils::age(player.birth_date, now),
                    generated: player.is_generated(),
                    is_free_agent: true,
                },
            ));
        }
    }

    countries.truncate(MAX_RESULTS_PER_KIND);

    clubs.sort_by(|a, b| b.0.cmp(&a.0));
    players.sort_by(|a, b| b.0.cmp(&a.0));

    let clubs = clubs
        .into_iter()
        .take(MAX_RESULTS_PER_KIND)
        .map(|(_, dto)| dto)
        .collect();
    let players = players
        .into_iter()
        .take(MAX_RESULTS_PER_KIND)
        .map(|(_, dto)| dto)
        .collect();

    Ok(Json(SearchResultsDto {
        countries,
        clubs,
        players,
    }))
}
