pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::potential_stars::PotentialStarsView;
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

                let club_view = simulator_data
                    .player_with_team(squad_player.player_id)
                    .map(|(_, t)| t);
                let (current, potential) = match club_view {
                    Some(t) => (
                        PotentialStarsView::current(player),
                        PotentialStarsView::potential_by_staff(player, t.staffs.head_coach()),
                    ),
                    None => (
                        PotentialStarsView::current(player),
                        PotentialStarsView::potential_absolute(player),
                    ),
                };

                Some(NationalSquadPlayerDto {
                    slug: player.slug(),
                    first_name: player.full_name.display_first_name().to_string(),
                    last_name: player.full_name.display_last_name().to_string(),
                    position,
                    position_sort: player.position(),
                    club_name,
                    club_slug,
                    age: DateUtils::age(player.birth_date, now),
                    current_ability: current,
                    potential_ability: potential,
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
                    current_ability: PotentialStarsView::current(player),
                    potential_ability: PotentialStarsView::potential_absolute(player),
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

    let current_path = format!(
        "/{}/countries/{}",
        route_params.lang, route_params.country_slug
    );
    let cl: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    Ok(CountrySquadTemplate {
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
        active_tab: "squad",
        country_slug: route_params.country_slug,
        players,
    })
}

fn get_conditions(player: &core::Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}
