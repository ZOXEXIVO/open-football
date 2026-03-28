pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
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
    pub active_tab: &'static str,
    pub country_slug: String,
    pub players: Vec<FreeAgentPlayerDto>,
}

pub struct FreeAgentPlayerDto {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub club_name: String,
    pub position: String,
    pub age: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub contract_days_left: i64,
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
            ApiError::NotFound(format!(
                "Country '{}' not found",
                route_params.country_slug
            ))
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

    // Collect free agents: players with no contract OR contracts expiring within 180 days
    // This shows both
    // out-of-contract players and those available on a pre-contract/free transfer
    let mut players: Vec<FreeAgentPlayerDto> = country
        .clubs
        .iter()
        .flat_map(|club| {
            let club_name = club.name.clone();
            club.teams.teams.iter().flat_map(move |team| {
                let club_name = club_name.clone();
                team.players
                    .players
                    .iter()
                    .filter_map(move |player| {
                        // Skip loan players
                        if player.is_on_loan() {
                            return None;
                        }

                        let days_left = match &player.contract {
                            None => 0, // no contract — true free agent
                            Some(c) => {
                                let days = (c.expiration - now).num_days();
                                if days <= 180 {
                                    days // contract expiring soon
                                } else {
                                    return None; // still under contract
                                }
                            }
                        };

                        let position = player.positions.display_positions_compact();
                        Some(FreeAgentPlayerDto {
                            id: player.id,
                            first_name: player.full_name.display_first_name().to_string(),
                            last_name: player.full_name.display_last_name().to_string(),
                            club_name: club_name.clone(),
                            position,
                            age: DateUtils::age(player.birth_date, now),
                            current_ability: get_ability_stars(
                                player.player_attributes.current_ability,
                            ),
                            potential_ability: get_ability_stars(
                                player.player_attributes.potential_ability,
                            ),
                            contract_days_left: days_left,
                        })
                    })
            })
        })
        .collect();

    // Sort: true free agents first (days_left=0), then by expiry, then by ability
    players.sort_by(|a, b| {
        a.contract_days_left.cmp(&b.contract_days_left)
            .then(b.current_ability.cmp(&a.current_ability))
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
        css_version: crate::common::default_handler::CSS_VERSION,
        computer_name: &crate::common::default_handler::COMPUTER_NAME,
        title: country.name.clone(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: continent.name.clone(),
        sub_title_link: format!("/{}/countries", route_params.lang),
        sub_title_country_code: String::new(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: &country.name, country_slug: &route_params.country_slug };
            views::country_menu(&mp, &cl)
        },
        lang: route_params.lang,
        i18n,
        active_tab: "free_agents",
        country_slug: route_params.country_slug,
        players,
    })
}

fn get_ability_stars(ability: u8) -> u8 {
    (5.0f32 * (ability as f32 / 200.0)).round() as u8
}
