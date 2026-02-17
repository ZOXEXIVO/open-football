pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamTransfersRequest {
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/transfers/index.html")]
pub struct TeamTransfersTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub items: Vec<TransferListItem>,
    pub incoming_transfers: Vec<TransferHistoryItem>,
    pub outgoing_transfers: Vec<TransferHistoryItem>,
}

pub struct TransferListItem {
    pub player_id: u32,
    pub player_name: String,
    pub position: String,
    pub value: String,
}

pub struct TransferHistoryItem {
    pub player_id: u32,
    pub player_team_slug: String,
    pub player_name: String,
    pub other_team: String,
    pub other_team_slug: String,
    pub fee: String,
    pub date: String,
}

pub async fn team_transfers_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamTransfersRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let team_id = indexes
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = simulator_data
        .league(team.league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", team.league_id)))?;

    let country = simulator_data
        .country(league.country_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Country with ID {} not found", league.country_id))
        })?;

    let now = simulator_data.date.date();
    let neighbor_teams: Vec<(&str, &str)> = get_neighbor_teams(team.club_id, simulator_data)?;

    let club_id = team.club_id;

    // Current transfer list items
    let items: Vec<TransferListItem> = team
        .transfer_list
        .items()
        .iter()
        .filter_map(|ti| {
            let player = team.players().into_iter().find(|p| p.id == ti.player_id)?;
            Some(TransferListItem {
                player_id: player.id,
                player_name: format!(
                    "{} {}",
                    player.full_name.first_name, player.full_name.last_name
                ),
                position: player.position().get_short_name().to_string(),
                value: FormattingUtils::format_money(player.value(now)),
            })
        })
        .collect();

    // Incoming transfers (players bought by this club)
    let incoming_transfers: Vec<TransferHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.to_club_id == club_id)
        .map(|t| {
            let other_team_slug = get_first_team_slug(country, t.from_club_id);
            let player_team_slug = get_first_team_slug(country, t.to_club_id);
            TransferHistoryItem {
                player_id: t.player_id,
                player_team_slug,
                player_name: t.player_name.clone(),
                other_team: t.from_team_name.clone(),
                other_team_slug,
                fee: FormattingUtils::format_money(t.fee.amount),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    // Outgoing transfers (players sold by this club)
    let outgoing_transfers: Vec<TransferHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.from_club_id == club_id)
        .map(|t| {
            let other_team_slug = get_first_team_slug(country, t.to_club_id);
            let player_team_slug = get_first_team_slug(country, t.to_club_id);
            TransferHistoryItem {
                player_id: t.player_id,
                player_team_slug,
                player_name: t.player_name.clone(),
                other_team: t.to_team_name.clone(),
                other_team_slug,
                fee: FormattingUtils::format_money(t.fee.amount),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    Ok(TeamTransfersTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: team.name.clone(),
        sub_title: league.name.clone(),
        sub_title_link: format!("/leagues/{}", &league.slug),
        menu_sections: views::team_menu(&neighbor_teams, &team.slug),
        team_slug: team.slug.clone(),
        items,
        incoming_transfers,
        outgoing_transfers,
    })
}

fn get_first_team_slug(country: &core::Country, club_id: u32) -> String {
    country
        .clubs
        .iter()
        .find(|c| c.id == club_id)
        .and_then(|c| c.teams.teams.first())
        .map(|t| t.slug.clone())
        .unwrap_or_default()
}

fn get_neighbor_teams<'a>(
    club_id: u32,
    data: &'a SimulatorData,
) -> Result<Vec<(&'a str, &'a str)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(&str, &str, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (team.name.as_str(), team.slug.as_str(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
