pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use core::transfers::TransferType;
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamTransfersRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/transfers/index.html")]
pub struct TeamTransfersTemplate {
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
    pub items: Vec<TransferListItem>,
    pub incoming_transfers: Vec<TransferHistoryItem>,
    pub outgoing_transfers: Vec<TransferHistoryItem>,
    pub incoming_loans: Vec<LoanHistoryItem>,
    pub outgoing_loans: Vec<LoanHistoryItem>,
}

pub struct TransferListItem {
    pub player_id: u32,
    pub player_name: String,
    pub position: String,
    pub value: String,
}

pub struct TransferHistoryItem {
    pub player_id: u32,
    pub player_name: String,
    pub other_team: String,
    pub other_team_slug: String,
    pub fee: String,
    pub date: String,
}

pub struct LoanHistoryItem {
    pub player_id: u32,
    pub player_name: String,
    pub other_team: String,
    pub other_team_slug: String,
    pub date: String,
    pub end_date: String,
}

pub async fn team_transfers_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamTransfersRequest>,
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
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let country = simulator_data
        .country_by_club(team.club_id)
        .ok_or_else(|| {
            ApiError::NotFound("Country not found for team".to_string())
        })?;

    let now = simulator_data.date.date();
    let (neighbor_teams, league_info) = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Option<(&str, &str)> = league_info.as_ref().map(|(n, s)| (n.as_str(), s.as_str()));

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

    // Incoming transfers (players bought by this club, excluding loans)
    let incoming_transfers: Vec<TransferHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.to_club_id == club_id && !matches!(t.transfer_type, TransferType::Loan(_)))
        .map(|t| {
            let other_team_slug = get_first_team_slug(country, t.from_club_id);
            TransferHistoryItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                other_team: t.from_team_name.clone(),
                other_team_slug,
                fee: FormattingUtils::format_money(t.fee.amount),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    // Outgoing transfers (players sold by this club, excluding loans)
    let outgoing_transfers: Vec<TransferHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.from_club_id == club_id && !matches!(t.transfer_type, TransferType::Loan(_)))
        .map(|t| {
            let other_team_slug = get_first_team_slug(country, t.to_club_id);
            TransferHistoryItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                other_team: t.to_team_name.clone(),
                other_team_slug,
                fee: FormattingUtils::format_money(t.fee.amount),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    // Incoming loans (players loaned in)
    let incoming_loans: Vec<LoanHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.to_club_id == club_id && matches!(t.transfer_type, TransferType::Loan(_)))
        .map(|t| {
            let end_date = match &t.transfer_type {
                TransferType::Loan(d) => d.format("%d.%m.%Y").to_string(),
                _ => String::new(),
            };
            LoanHistoryItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                other_team: t.from_team_name.clone(),
                other_team_slug: get_first_team_slug(country, t.from_club_id),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
                end_date,
            }
        })
        .collect();

    // Outgoing loans (players loaned out)
    let outgoing_loans: Vec<LoanHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.from_club_id == club_id && matches!(t.transfer_type, TransferType::Loan(_)))
        .map(|t| {
            let end_date = match &t.transfer_type {
                TransferType::Loan(d) => d.format("%d.%m.%Y").to_string(),
                _ => String::new(),
            };
            LoanHistoryItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                other_team: t.to_team_name.clone(),
                other_team_slug: get_first_team_slug(country, t.to_club_id),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
                end_date,
            }
        })
        .collect();

    let menu_sections = views::team_menu(&i18n, &route_params.lang, &neighbor_refs, &team.slug, &format!("/{}/teams/{}/transfers", &route_params.lang, &team.slug), league_refs);
    let title = if team.team_type == core::TeamType::Main { team.name.clone() } else { format!("{} - {}", team.name, i18n.t(team.team_type.as_i18n_key())) };

    Ok(TeamTransfersTemplate {
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
        items,
        incoming_transfers,
        outgoing_transfers,
        incoming_loans,
        outgoing_loans,
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

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<(Vec<(String, String)>, Option<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut league_info: Option<(String, String)> = None;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            if team.team_type == core::TeamType::Main {
                if let Some(league_id) = team.league_id {
                    if let Some(league) = data.league(league_id) {
                        league_info = Some((league.name.clone(), league.slug.clone()));
                    }
                }
            }
            (format!("{} {}", club_name, i18n.t(team.team_type.as_i18n_key())), team.slug.clone(), team.reputation.world)
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok((teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect(), league_info))
}
