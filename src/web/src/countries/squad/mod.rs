pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::DateUtils;
use core::{CallUpReason, Country, PlayerPositionType, SquadPick};
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
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub country_slug: String,
    pub players: Vec<NationalSquadPlayerDto>,
}

pub struct NationalSquadPlayerDto {
    pub slug: String,
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
    /// i18n key of the player's primary call-up reason
    pub reason_key: &'static str,
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

    // Build national squad player DTOs. `squad_picks` returns real
    // call-ups + synthetic depth players in one pass — both kinds are
    // shown in the squad table so weak nations don't appear empty.
    let mut players: Vec<NationalSquadPlayerDto> = country
        .national_team
        .squad_picks()
        .into_iter()
        .filter_map(|pick| match pick {
            SquadPick::Real(squad_player) => {
                let player = simulator_data.player(squad_player.player_id)?;
                let club = simulator_data.club(squad_player.club_id);
                let club_name = club.map(|c| c.name.clone()).unwrap_or_default();
                let club_slug = club
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();

                let position = player.positions.display_positions_compact();

                let (judging, coach_id) = simulator_data
                    .player_with_team(squad_player.player_id)
                    .map(|(_, t)| {
                        let hc = t.staffs.head_coach();
                        (hc.staff_attributes.knowledge.judging_player_potential, hc.id)
                    })
                    .unwrap_or((10, 0));

                Some(NationalSquadPlayerDto {
                    slug: player.slug(),
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
                    reason_key: squad_player.primary_reason.as_i18n_key(),
                })
            }
            SquadPick::Synthetic(player) => {
                // Synthetic players don't belong to any club; render
                // with no club link and a SyntheticDepth reason so the
                // UI is honest about which slots are stand-ins.
                let position = player.positions.display_positions_compact();
                Some(NationalSquadPlayerDto {
                    slug: player.slug(),
                    first_name: player.full_name.display_first_name().to_string(),
                    last_name: player.full_name.display_last_name().to_string(),
                    position,
                    position_sort: player.position(),
                    club_name: String::new(),
                    club_slug: String::new(),
                    age: DateUtils::age(player.birth_date, now),
                    current_ability: get_current_ability_stars(player),
                    potential_ability: get_potential_ability_stars_by_staff(player, 10, 0),
                    conditions: get_conditions(player),
                    international_apps: player.player_attributes.international_apps,
                    international_goals: player.player_attributes.international_goals,
                    reason_key: CallUpReason::SyntheticDepth.as_i18n_key(),
                })
            }
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
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
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

    let stars = (raw_stars + noise * noise_scale).round().clamp(0.0, 5.0) as u8;
    // Real potential is always ≥ current ability, so the display must
    // be too. Without this, scout noise can push potential below the
    // un-noised current rating (e.g. CA 65 → 2 ★, PA 75 → 1 ★).
    stars.max(get_current_ability_stars(player))
}
