pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::{Country, PlayerPositionType};
use core::utils::DateUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CountrySquadRequest {
    lang: String,
    country_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "countries/squad/index.html")]
pub struct CountrySquadTemplate {
    pub css_version: &'static str,
    pub hostname: &'static str,
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
    pub players: Vec<NationalSquadPlayerDto>,
}

pub struct NationalSquadPlayerDto {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub club_name: String,
    pub club_slug: String,
    pub age: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub conditions: u8,
    pub international_apps: u16,
    pub international_goals: u16,
}

pub async fn country_squad_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountrySquadRequest>,
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

    let now = simulator_data.date.date();

    // Build national squad player DTOs
    let mut players: Vec<NationalSquadPlayerDto> = country
        .national_team
        .squad
        .iter()
        .filter_map(|squad_player| {
            // Find the player globally — they may play for a club in another country
            let player = simulator_data.player(squad_player.player_id)?;
            let club = simulator_data.club(squad_player.club_id);
            let club_name = club.map(|c| c.name.clone()).unwrap_or_default();
            let club_slug = club
                .and_then(|c| c.teams.teams.first())
                .map(|t| t.slug.clone())
                .unwrap_or_default();

            let position = player.positions.display_positions_compact();

            // Use the player's team's head coach for ability assessment
            let (judging, coach_id) = simulator_data
                .player_with_team(squad_player.player_id)
                .map(|(_, t)| {
                    let hc = t.staffs.head_coach();
                    (hc.staff_attributes.knowledge.judging_player_potential, hc.id)
                })
                .unwrap_or((10, 0));

            Some(NationalSquadPlayerDto {
                id: player.id,
                first_name: player.full_name.display_first_name().to_string(),
                last_name: player.full_name.display_last_name().to_string(),
                position,
                position_sort: player.position(),
                club_name,
                club_slug,
                age: DateUtils::age(player.birth_date, now),
                current_ability: get_current_ability_stars(player),
                potential_ability: get_potential_ability_stars_by_staff(
                    player,
                    judging,
                    coach_id,
                ),
                conditions: get_conditions(player),
                international_apps: player.player_attributes.international_apps,
                international_goals: player.player_attributes.international_goals,
            })
        })
        .collect();

    players.sort_by(|a, b| {
        a.position_sort
            .partial_cmp(&b.position_sort)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let current_path = format!("/{}/countries/{}", route_params.lang, route_params.country_slug);
    let cl: Vec<(&str, &str)> = country.leagues.leagues.iter().filter(|l| !l.friendly).map(|l| (l.name.as_str(), l.slug.as_str())).collect();

    Ok(CountrySquadTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        hostname: &crate::common::default_handler::HOSTNAME,
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
        active_tab: "squad",
        country_slug: route_params.country_slug,
        players,
    })
}

fn get_conditions(player: &core::Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

fn get_current_ability_stars(player: &core::Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)).round() as u8
}

fn get_potential_ability_stars_by_staff(player: &core::Player, staff_judging: u8, staff_id: u32) -> u8 {
    let raw_stars = 5.0 * (player.player_attributes.potential_ability as f32 / 200.0);
    let accuracy = (staff_judging as f32 / 20.0).clamp(0.0, 1.0);
    let noise_scale = (1.0 - accuracy) * 1.5;

    let hash = staff_id
        .wrapping_mul(2654435761)
        .wrapping_add(player.id.wrapping_mul(2246822519));
    let hash = hash ^ (hash >> 16);
    let hash = hash.wrapping_mul(0x45d9f3b);
    let hash = hash ^ (hash >> 16);
    let noise = (hash & 0xFFFF) as f32 / 32768.0 - 1.0;

    (raw_stars + noise * noise_scale).round().clamp(0.0, 5.0) as u8
}
