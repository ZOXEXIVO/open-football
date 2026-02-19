pub mod routes;

use crate::player::PlayerStatusDto;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::ContractType;
use core::Player;
use core::PlayerPositionType;
use core::PlayerStatusType;
use core::transfers::TransferType;
use core::utils::{DateUtils, FormattingUtils};
use core::{SimulatorData, Team};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamGetRequest {
    pub lang: String,
    pub team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/get/index.html")]
pub struct TeamGetTemplate {
    pub css_version: &'static str,
    pub i18n: crate::I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub players: Vec<TeamPlayer>,
}

pub struct TeamPlayer {
    pub id: u32,
    pub last_name: String,
    pub first_name: String,
    pub behaviour: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub value: String,
    pub injured: bool,
    pub transfer_listed: bool,
    pub is_loan: bool,
    pub is_loaned_out: bool,
    pub is_youth: bool,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub conditions: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub age: u8,
    pub played: u16,
    pub played_subs: u16,
    #[allow(dead_code)]
    pub status: PlayerStatusDto,
}

pub async fn team_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let team_id = indexes
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug)))?;

    let team: &Team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let now = simulator_data.date.date();

    let club_id = team.club_id;

    // Build a set of player IDs loaned IN to this club from transfer history
    let loaned_in_player_ids: Vec<u32> = simulator_data
        .country_by_club(club_id)
        .map(|country| {
            country
                .transfer_market
                .transfer_history
                .iter()
                .filter(|t| {
                    t.to_club_id == club_id
                        && matches!(&t.transfer_type, TransferType::Loan(_))
                })
                .map(|t| t.player_id)
                .collect()
        })
        .unwrap_or_default();

    let mut players: Vec<TeamPlayer> = team
        .players()
        .iter()
        .filter_map(|p| {
            let country = simulator_data.country(p.country_id)?;
            let position = p.positions.display_positions().join(", ");

            let is_loan = p.contract.as_ref()
                .map(|c| c.contract_type == ContractType::Loan)
                .unwrap_or(false)
                || loaned_in_player_ids.contains(&p.id);

            let is_youth = p.contract.as_ref()
                .map(|c| c.contract_type == ContractType::Youth)
                .unwrap_or(false);

            Some(TeamPlayer {
                id: p.id,
                first_name: p.full_name.first_name.clone(),
                position_sort: p.position(),
                position,
                behaviour: p.behaviour.as_str().to_string(),
                injured: p.player_attributes.is_injured,
                transfer_listed: p.statuses.get().contains(&PlayerStatusType::Lst),
                is_loan,
                is_loaned_out: false,
                is_youth,
                country_slug: country.slug.clone(),
                country_code: country.code.clone(),
                country_name: country.name.clone(),
                last_name: p.full_name.last_name.clone(),
                conditions: get_conditions(p),
                value: FormattingUtils::format_money(p.value(now)),
                current_ability: get_current_ability_stars(p),
                potential_ability: get_potential_ability_stars(p),
                age: DateUtils::age(p.birth_date, now),
                played: p.statistics.played,
                played_subs: p.statistics.played_subs,
                status: PlayerStatusDto::new(p.statuses.get()),
            })
        })
        .collect();

    // Collect current player IDs to avoid duplicates
    let current_player_ids: Vec<u32> = players.iter().map(|p| p.id).collect();

    // Find loaned-out players from transfer history
    // Note: simulator_data.player() won't work here because indexes are stale after transfers.
    // We search directly in the country's clubs instead.
    if let Some(country) = simulator_data.country_by_club(club_id) {
        let loan_records: Vec<_> = country
            .transfer_market
            .transfer_history
            .iter()
            .filter(|t| {
                t.from_club_id == club_id
                    && matches!(&t.transfer_type, TransferType::Loan(_))
                    && !current_player_ids.contains(&t.player_id)
            })
            .collect();

        for t in loan_records {
            // Search for the player directly in destination club's teams
            let dest_club = country.clubs.iter().find(|c| c.id == t.to_club_id);

            let found = dest_club.and_then(|club| {
                for dest_team in &club.teams.teams {
                    if let Some(player) = dest_team.players.players.iter().find(|p| p.id == t.player_id) {
                        return Some((player, dest_team.slug.clone()));
                    }
                }
                None
            });

            if let Some((player, _)) = found {
                let player_country = simulator_data.country(player.country_id);
                let position = player.positions.display_positions().join(", ");

                players.push(TeamPlayer {
                    id: player.id,
                    first_name: player.full_name.first_name.clone(),
                    position_sort: player.position(),
                    position,
                    behaviour: player.behaviour.as_str().to_string(),
                    injured: player.player_attributes.is_injured,
                    transfer_listed: false,
                    is_loan: false,
                    is_loaned_out: true,
                    is_youth: false,
                    country_slug: player_country.map(|c| c.slug.clone()).unwrap_or_default(),
                    country_code: player_country.map(|c| c.code.clone()).unwrap_or_default(),
                    country_name: player_country.map(|c| c.name.clone()).unwrap_or_default(),
                    last_name: player.full_name.last_name.clone(),
                    conditions: get_conditions(player),
                    value: FormattingUtils::format_money(player.value(now)),
                    current_ability: get_current_ability_stars(player),
                    potential_ability: get_potential_ability_stars(player),
                    age: DateUtils::age(player.birth_date, now),
                    played: player.statistics.played,
                    played_subs: player.statistics.played_subs,
                    status: PlayerStatusDto::new(player.statuses.get()),
                });
            }
        }
    }

    players.sort_by(|a, b| {
        // Sort loaned-out players to the end
        a.is_loaned_out.cmp(&b.is_loaned_out).then_with(|| {
            a.position_sort
                .partial_cmp(&b.position_sort)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    let neighbor_teams: Vec<(String, String)> = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let menu_sections = views::team_menu(&i18n, &route_params.lang, &neighbor_refs, &team.slug, &format!("/{}/teams/{}", &route_params.lang, &team.slug));
    let title = if team.team_type == core::TeamType::Main { team.name.clone() } else { format!("{} - {}", team.name, i18n.t(team.team_type.as_i18n_key())) };

    Ok(TeamGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league.map(|l| l.name.clone()).unwrap_or_default(),
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        players,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<Vec<(String, String)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (i18n.t(team.team_type.as_i18n_key()).to_string(), team.slug.clone(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams.into_iter().map(|(name, slug, _)| (name, slug)).collect())
}

pub fn get_conditions(player: &Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

pub fn get_current_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)) as u8
}

pub fn get_potential_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.potential_ability as f32) / 200.0)) as u8
}
