pub mod routes;

use crate::GameAppData;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Datelike;
use core::ContractType;
use core::PlayerClubContract;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct PlayerPathParam {
    pub player_id: u32,
}

// ── Clear injury ────────────────────────────────────────────────

pub async fn clear_injury_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        if let Some(player) = sim.player_mut(params.player_id) {
            player.player_attributes.is_injured = false;
            player.player_attributes.injury_days_remaining = 0;
            player.player_attributes.injury_type = None;
            player.player_attributes.recovery_days_remaining = 0;
            return StatusCode::OK;
        }
    }

    StatusCode::NOT_FOUND
}

// ── Cancel loan ─────────────────────────────────────────────────

pub async fn cancel_loan_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        if let Some(player) = sim.player_mut(params.player_id) {
            if let Some(ref mut contract) = player.contract {
                if contract.contract_type == ContractType::Loan {
                    contract.contract_type = ContractType::FullTime;
                    contract.loan_from_club_id = None;
                    contract.loan_to_club_id = None;
                    return StatusCode::OK;
                }
            }
        }
    }

    StatusCode::NOT_FOUND
}

// ── Transfer ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TransferRequest {
    pub to_club_id: u32,
    #[allow(dead_code)]
    pub fee: Option<u32>,
}

pub async fn transfer_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
    Json(body): Json<TransferRequest>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        let date = sim.date.date();

        // Find which team the player is currently on
        let player_pos = sim.find_player_position(params.player_id);
        let (ci, coi, cli, ti) = match player_pos {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        // Find destination club main team
        let dest = sim.find_club_main_team(body.to_club_id);
        let (dci, dcoi, dcli, dti) = match dest {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        // Take player from source
        let mut player = match sim.continents[ci].countries[coi].clubs[cli]
            .teams.teams[ti].players.take_player(&params.player_id)
        {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND,
        };

        // Give new contract
        let expiration = chrono::NaiveDate::from_ymd_opt(date.year() + 3, 6, 30)
            .unwrap_or(date);
        let salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(1000);
        player.contract = Some(PlayerClubContract::new(salary, expiration));

        // Add to destination team
        sim.continents[dci].countries[dcoi].clubs[dcli]
            .teams.teams[dti].players.add(player);

        // Rebuild indexes
        sim.rebuild_indexes();

        return StatusCode::OK;
    }

    StatusCode::NOT_FOUND
}

// ── Loan ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoanRequest {
    pub to_club_id: u32,
    pub seasons: Option<u32>,
}

pub async fn loan_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
    Json(body): Json<LoanRequest>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        let date = sim.date.date();

        let player_pos = sim.find_player_position(params.player_id);
        let (ci, coi, cli, ti) = match player_pos {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let source_club_id = sim.continents[ci].countries[coi].clubs[cli].id;

        let dest = sim.find_club_main_team(body.to_club_id);
        let (dci, dcoi, dcli, dti) = match dest {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let mut player = match sim.continents[ci].countries[coi].clubs[cli]
            .teams.teams[ti].players.take_player(&params.player_id)
        {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND,
        };

        let seasons = body.seasons.unwrap_or(1).clamp(1, 5) as i32;
        let expiration = chrono::NaiveDate::from_ymd_opt(date.year() + seasons, 6, 30)
            .unwrap_or(date);
        let salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(1000);
        player.contract = Some(PlayerClubContract::new_loan(salary, expiration, source_club_id, body.to_club_id));

        sim.continents[dci].countries[dcoi].clubs[dcli]
            .teams.teams[dti].players.add(player);

        sim.rebuild_indexes();

        return StatusCode::OK;
    }

    StatusCode::NOT_FOUND
}

// ── List clubs (for dropdowns) ──────────────────────────────────

#[derive(Serialize)]
pub struct ClubListItem {
    pub id: u32,
    pub name: String,
}

#[derive(Deserialize)]
pub struct ClubsQuery {
    pub q: Option<String>,
}

pub async fn list_clubs_action(
    State(state): State<GameAppData>,
    Query(query): Query<ClubsQuery>,
) -> Json<Vec<ClubListItem>> {
    let guard = state.data.read().await;

    let mut clubs = Vec::new();

    if let Some(ref sim) = *guard {
        let filter = query.q.as_deref().unwrap_or("").to_lowercase();
        for continent in &sim.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    if filter.is_empty() || club.name.to_lowercase().contains(&filter) {
                        clubs.push(ClubListItem {
                            id: club.id,
                            name: club.name.clone(),
                        });
                    }
                }
            }
        }
    }

    clubs.sort_by(|a, b| a.name.cmp(&b.name));
    Json(clubs)
}
