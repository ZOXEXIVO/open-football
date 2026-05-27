pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::potential_stars::PotentialStarsView;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::DateUtils;
use core::{CallUpReason, Country, NationalTeam, NationalTeamLevel, PlayerPositionType, SquadPick};
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
    /// Players for the single national-team level shown on this page.
    pub players: Vec<NationalSquadPlayerDto>,
    /// i18n key for the squad panel title — "squad" (senior) or
    /// "u21_national_team" (U21).
    pub panel_title_key: &'static str,
    /// i18n keys for the caps/goals column headers (level-appropriate).
    pub caps_key: &'static str,
    pub goals_key: &'static str,
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

/// Senior national-team squad — the country's default squad page.
pub async fn country_squad_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountrySquadRequest>,
) -> ApiResult<impl IntoResponse> {
    render_country_squad(state, route_params, NationalTeamLevel::Senior).await
}

/// U21 national-team squad — reached via the left-menu switch.
pub async fn country_u21_squad_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountrySquadRequest>,
) -> ApiResult<impl IntoResponse> {
    render_country_squad(state, route_params, NationalTeamLevel::Under21).await
}

async fn render_country_squad(
    state: GameAppData,
    route_params: CountrySquadRequest,
    level: NationalTeamLevel,
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

    // Pick the team for the requested level. Each table shows real
    // call-ups + synthetic depth players (so weak nations don't render
    // empty); caps/goals columns are level-appropriate.
    let (team, panel_title_key, caps_key, goals_key) = match level {
        NationalTeamLevel::Senior => (&country.national_team, "squad", "int_apps", "int_goals"),
        NationalTeamLevel::Under21 => (
            &country.u21_national_team,
            "u21_national_team",
            "u21_caps",
            "u21_goals",
        ),
    };
    let players = build_squad_dtos(simulator_data, team, now);

    // Senior lives at the base country URL; U21 hangs off `/u21`. The
    // menu's active-state checks key off this path.
    let current_path = match level {
        NationalTeamLevel::Senior => {
            format!(
                "/{}/countries/{}",
                route_params.lang, route_params.country_slug
            )
        }
        NationalTeamLevel::Under21 => format!(
            "/{}/countries/{}/u21",
            route_params.lang, route_params.country_slug
        ),
    };
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
            views::country_menu(
                &mp,
                &cl,
                country
                    .domestic_cup
                    .as_ref()
                    .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
                country.continent_id,
            )
        },
        lang: route_params.lang,
        i18n,
        active_tab: "squad",
        country_slug: route_params.country_slug,
        players,
        panel_title_key,
        caps_key,
        goals_key,
    })
}

/// Build the squad-table DTOs for one national-team level. `squad_picks`
/// returns real call-ups followed by synthetic depth players in a single
/// pass. The caps/goals columns are filled from the senior or U21 ledger
/// depending on `team.level`.
fn build_squad_dtos(
    simulator_data: &core::SimulatorData,
    team: &NationalTeam,
    now: chrono::NaiveDate,
) -> Vec<NationalSquadPlayerDto> {
    let is_u21 = team.level == NationalTeamLevel::Under21;
    let mut players: Vec<NationalSquadPlayerDto> = team
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

                let (apps, goals) = if is_u21 {
                    (
                        player.player_attributes.under_21_international_apps,
                        player.player_attributes.under_21_international_goals,
                    )
                } else {
                    (
                        player.player_attributes.international_apps,
                        player.player_attributes.international_goals,
                    )
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
                    international_apps: apps,
                    international_goals: goals,
                    reason_key: squad_player.primary_reason.as_i18n_key(),
                })
            }
            SquadPick::Synthetic(player) => {
                // Synthetic players don't belong to any club; render
                // with no club link and a SyntheticDepth reason so the
                // UI is honest about which slots are stand-ins.
                let position = player.positions.display_positions_compact();
                let (apps, goals) = if is_u21 {
                    (
                        player.player_attributes.under_21_international_apps,
                        player.player_attributes.under_21_international_goals,
                    )
                } else {
                    (
                        player.player_attributes.international_apps,
                        player.player_attributes.international_goals,
                    )
                };
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
                    international_apps: apps,
                    international_goals: goals,
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

    players
}

fn get_conditions(player: &core::Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}
