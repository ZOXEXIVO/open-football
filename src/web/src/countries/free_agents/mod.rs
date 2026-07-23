pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::potential_stars::{PotentialStarsView, StarRating};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PlayerPositionType;
use core::utils::DateUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountryFreeAgentsRequest {
    lang: String,
    country_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/free_agents/index.html")]
pub struct CountryFreeAgentsTemplate {
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
    pub players: Vec<FreeAgentPlayerDto>,
}

pub struct FreeAgentPlayerDto {
    pub slug: String,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
    pub age: u8,
    pub current_ability: StarRating,
    pub potential_ability: StarRating,
    /// Primary position used to bucket players in the squad-style sort
    /// (GK → DF → MF → FW). Not rendered — purely a sort key.
    pub position_sort: PlayerPositionType,
    /// Plain-language reason he is still unsigned, from the player's own
    /// durable market state (no world scan). Empty only if state is
    /// somehow missing.
    pub status_message: String,
}

pub async fn country_free_agents_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountryFreeAgentsRequest>,
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

    let country: &core::Country = simulator_data
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

    let mut players: Vec<FreeAgentPlayerDto> = simulator_data
        .free_agents
        .iter()
        .filter(|player| player.country_id == country.id)
        .map(|player| {
            let position = player.positions.display_positions_compact();
            // Cheap, state-only "why is he still free?" explanation —
            // no world scan, so it's safe to compute per row in a list.
            let status_message = player
                .market_explanation(now)
                .map(|e| e.message)
                .unwrap_or_default();
            FreeAgentPlayerDto {
                slug: player.slug(),
                first_name: player.full_name.display_first_name().to_string(),
                last_name: player.full_name.display_last_name().to_string(),
                position,
                age: DateUtils::age(player.birth_date, now),
                current_ability: PotentialStarsView::current(player),
                potential_ability: PotentialStarsView::potential_absolute(player, now),
                position_sort: player.position(),
                status_message,
            }
        })
        .collect();

    players.sort_by(|a, b| {
        a.position_sort
            .partial_cmp(&b.position_sort)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.current_ability.cmp(&a.current_ability))
    });

    let current_path = format!(
        "/{}/countries/{}/free-agents",
        route_params.lang, route_params.country_slug
    );
    let cl: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    Ok(CountryFreeAgentsTemplate {
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
            views::country_menu(
                &mp,
                &cl,
                country
                    .domestic_cup
                    .as_ref()
                    .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
                &country
                    .playoffs
                    .iter()
                    .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                    .collect::<Vec<_>>(),
                country.continent_id,
            )
        },
        lang: route_params.lang,
        i18n,
        active_tab: "free_agents",
        country_slug: route_params.country_slug,
        players,
    })
}
