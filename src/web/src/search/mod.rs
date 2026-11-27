pub mod routes;

use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;

pub struct SearchRoutes;

impl SearchRoutes {
    pub fn router() -> Router<GameAppData> {
        routes::SearchGetRoutes::router()
    }
}

#[allow(non_upper_case_globals)]
pub const search_routes: fn() -> axum::Router<GameAppData> = SearchRoutes::router;

#[derive(Deserialize)]
pub struct SearchPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "search/index.html")]
pub struct SearchPageTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
}

pub struct SearchPage;

impl SearchPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<SearchPageRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let current_path = format!("/{}/search", route_params.lang);
        let menu_sections = views::Menus::search_menu(&i18n, &route_params.lang, &current_path);
        let title = i18n.t("search").to_string();

        Ok(SearchPageTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            i18n,
            lang: route_params.lang.clone(),
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: String::new(),
            sub_title_link: String::new(),
            sub_title_flag: None,
            header_color: String::new(),
            foreground_color: String::new(),
            menu_sections,
        })
    }
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

pub struct SearchApi;

impl SearchApi {
    pub async fn action(
        State(state): State<GameAppData>,
        Query(query): Query<SearchApiQuery>,
    ) -> ApiResult<impl IntoResponse> {
        let world = state.world.snapshot();
        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let needle = query.q.trim().to_lowercase();
        if needle.len() < 4 {
            return Ok(Json(SearchResultsDto::empty()));
        }

        let mut collector = SearchCollector::new(simulator_data);
        collector.collect(&needle);
        Ok(Json(collector.finish()))
    }
}

impl SearchResultsDto {
    fn empty() -> SearchResultsDto {
        SearchResultsDto {
            countries: Vec::new(),
            clubs: Vec::new(),
            players: Vec::new(),
        }
    }
}

struct SearchCollector<'a> {
    data: &'a core::SimulatorData,
    now: chrono::NaiveDate,
    countries: Vec<SearchCountryDto>,
    clubs: Vec<(u16, SearchClubDto)>,
    players: Vec<(u8, SearchPlayerDto)>,
}

impl<'a> SearchCollector<'a> {
    fn new(data: &'a core::SimulatorData) -> SearchCollector<'a> {
        SearchCollector {
            data,
            now: data.date.date(),
            countries: Vec::with_capacity(MAX_RESULTS_PER_KIND),
            clubs: Vec::with_capacity(MAX_RESULTS_PER_KIND),
            players: Vec::with_capacity(MAX_RESULTS_PER_KIND),
        }
    }

    fn collect(&mut self, needle: &str) {
        for continent in &self.data.continents {
            for country in &continent.countries {
                if country.name.to_lowercase().contains(needle) {
                    self.countries.push(SearchCountryDto {
                        name: country.name.clone(),
                        slug: country.slug.clone(),
                        code: country.code.clone(),
                    });
                }
                self.collect_country_clubs(country, needle);
            }
        }
        self.collect_free_agents(needle);
    }

    fn collect_country_clubs(&mut self, country: &core::Country, needle: &str) {
        for club in &country.clubs {
            if club.name.to_lowercase().contains(needle)
                && let Some(main) = club.teams.main()
            {
                self.clubs.push((
                    main.reputation.world,
                    SearchClubDto {
                        name: club.name.clone(),
                        team_slug: main.slug.clone(),
                    },
                ));
            }

            for team in &club.teams.teams {
                for player in team.players.players() {
                    let full = Self::player_name(player);
                    if full.to_lowercase().contains(needle) {
                        self.push_player(player, full, team.name.clone(), false);
                    }
                }
            }
        }
    }

    fn collect_free_agents(&mut self, needle: &str) {
        for player in &self.data.free_agents {
            let full = Self::player_name(player);
            if full.to_lowercase().contains(needle) {
                self.push_player(player, full, String::new(), true);
            }
        }
    }

    fn player_name(player: &core::Player) -> String {
        let first = player.full_name.display_first_name();
        let last = player.full_name.display_last_name();
        format!("{} {}", first, last)
    }

    fn push_player(
        &mut self,
        player: &core::Player,
        full_name: String,
        team_name: String,
        is_free_agent: bool,
    ) {
        let country_code = self
            .data
            .country(player.country_id)
            .map(|c| c.code.clone())
            .or_else(|| {
                self.data
                    .country_info
                    .get(&player.country_id)
                    .map(|i| i.code.clone())
            })
            .unwrap_or_default();
        self.players.push((
            player.player_attributes.current_ability,
            SearchPlayerDto {
                id: player.id,
                slug: player.slug(),
                name: full_name.trim().to_string(),
                country_code,
                team_name,
                age: core::utils::DateUtils::age(player.birth_date, self.now),
                generated: player.is_generated(),
                is_free_agent,
            },
        ));
    }

    fn finish(mut self) -> SearchResultsDto {
        self.countries.truncate(MAX_RESULTS_PER_KIND);

        self.clubs.sort_by_key(|c| Reverse(c.0));
        self.players.sort_by_key(|p| Reverse(p.0));

        let clubs = self
            .clubs
            .into_iter()
            .take(MAX_RESULTS_PER_KIND)
            .map(|(_, dto)| dto)
            .collect();
        let players = self
            .players
            .into_iter()
            .take(MAX_RESULTS_PER_KIND)
            .map(|(_, dto)| dto)
            .collect();

        SearchResultsDto {
            countries: self.countries,
            clubs,
            players,
        }
    }
}
