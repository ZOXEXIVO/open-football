pub mod routes;

use crate::GameAppData;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Datelike;
use core::PlayerClubContract;
use core::shared::{Currency, CurrencyValue};
use core::transfers::{CompletedTransfer, TransferType};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct PlayerPathParam {
    pub player_id: u32,
}

/// Extended team info that includes club/team IDs for transfer history recording.
struct ExtTeamInfo {
    club_id: u32,
    team_id: u32,
    info: core::TeamInfo,
}

fn get_team_info(
    sim: &core::SimulatorData,
    ci: usize,
    coi: usize,
    cli: usize,
) -> Option<ExtTeamInfo> {
    let club = &sim.continents[ci].countries[coi].clubs[cli];
    let main_team = club
        .teams
        .teams
        .iter()
        .find(|t| t.team_type == core::TeamType::Main)
        .or(club.teams.teams.first())?;
    let league = main_team
        .league_id
        .and_then(|lid| sim.league(lid))
        .map(|l| (l.name.clone(), l.slug.clone()));
    Some(ExtTeamInfo {
        club_id: club.id,
        team_id: main_team.id,
        info: core::TeamInfo {
            name: main_team.name.clone(),
            slug: main_team.slug.clone(),
            reputation: main_team.reputation.world,
            league_name: league.as_ref().map(|l| l.0.clone()).unwrap_or_default(),
            league_slug: league.map(|l| l.1).unwrap_or_default(),
        },
    })
}

fn get_team_info_by_club_id(sim: &core::SimulatorData, club_id: u32) -> Option<ExtTeamInfo> {
    let (ci, coi, cli, _) = sim.find_club_main_team(club_id)?;
    get_team_info(sim, ci, coi, cli)
}

/// Reputation inputs needed to install a permanent contract for a signing:
/// `(club_main_team_world_rep, league_rep)`. Drives `WageCalculator` via
/// `Player::install_permanent_contract`. Missing data falls through to 0
/// — `WageCalculator` will still produce a sensible wage at the bottom
/// of its scale rather than panicking.
fn signing_reputation_inputs(
    sim: &core::SimulatorData,
    ci: usize,
    coi: usize,
    cli: usize,
) -> (u16, u16) {
    let country = &sim.continents[ci].countries[coi];
    let club = &country.clubs[cli];
    let main_team = club.teams.main();
    let club_world_rep = main_team.map(|t| t.reputation.world).unwrap_or(0);
    let league_rep = main_team
        .and_then(|t| t.league_id)
        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
        .map(|l| l.reputation)
        .unwrap_or(0);
    (club_world_rep, league_rep)
}

// ── Move on free ───────────────────────────────────────────────

pub async fn move_on_free_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        let date = sim.date.date();

        let (ci, coi, cli, ti) = match sim.find_player_position(params.player_id) {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let from_info = match get_team_info(sim, ci, coi, cli) {
            Some(t) => t.info,
            None => return StatusCode::NOT_FOUND,
        };

        let mut player = match sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
            .players
            .take_player(&params.player_id)
        {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND,
        };

        // Snapshot in-flight competitive stats onto the source club's
        // career row before the player leaves it; otherwise those games
        // would later be misattributed to the destination — or, in the
        // global-pool path, to a synthetic "Free Agent" row.
        player.on_release(&from_info, date);

        player.contract = None;
        player.contract_loan = None;
        player.statuses.remove(core::PlayerStatusType::Lst);
        player.statuses.remove(core::PlayerStatusType::Trn);
        player.statuses.remove(core::PlayerStatusType::Bid);
        player.statuses.remove(core::PlayerStatusType::Enq);
        player.statuses.remove(core::PlayerStatusType::Wnt);
        player.statuses.remove(core::PlayerStatusType::Req);
        player.statuses.remove(core::PlayerStatusType::Frt);
        player.statuses.remove(core::PlayerStatusType::Unh);
        player.statuses.remove(core::PlayerStatusType::Loa);
        player.happiness.clear();

        sim.free_agents.push(player);
        sim.rebuild_indexes();
        return StatusCode::OK;
    }

    StatusCode::NOT_FOUND
}

// ── Clear unhappy ──────────────────────────────────────────────

pub async fn clear_unhappy_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);

        if let Some(player) = sim.player_mut(params.player_id) {
            player.statuses.remove(core::PlayerStatusType::Unh);
            player.happiness.clear();
            return StatusCode::OK;
        }
    }

    StatusCode::NOT_FOUND
}

// ── Toggle force match selection ───────────────────────────────

pub async fn toggle_force_match_selection_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        if let Some(player) = sim.player_mut(params.player_id) {
            player.is_force_match_selection = !player.is_force_match_selection;
            return StatusCode::OK;
        }
    }

    StatusCode::NOT_FOUND
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
        let date = sim.date.date();

        // Find player and validate loan
        let (ci, coi, cli, ti) = match sim.find_player_position(params.player_id) {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let parent_club_id = {
            let player = sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                .players
                .players
                .iter()
                .find(|p| p.id == params.player_id);
            match player.and_then(|p| p.contract_loan.as_ref()) {
                Some(c) => c.loan_from_club_id,
                _ => return StatusCode::NOT_FOUND,
            }
        };

        let parent_club_id = match parent_club_id {
            Some(id) => id,
            None => return StatusCode::NOT_FOUND,
        };

        let borrowing = match get_team_info(sim, ci, coi, cli) {
            Some(t) => t,
            None => return StatusCode::NOT_FOUND,
        };

        let parent = match get_team_info_by_club_id(sim, parent_club_id) {
            Some(t) => t,
            None => return StatusCode::NOT_FOUND,
        };

        // Take player
        let mut player = match sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
            .players
            .take_player(&params.player_id)
        {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND,
        };

        player.on_cancel_loan(&borrowing.info, &parent.info, date);
        player.contract_loan = None;

        let (dci, dcoi, dcli, dti) = match sim.find_club_main_team(parent_club_id) {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };
        sim.continents[dci].countries[dcoi].clubs[dcli].teams.teams[dti]
            .players
            .add(player);

        sim.rebuild_indexes();
        return StatusCode::OK;
    }

    StatusCode::NOT_FOUND
}

// ── Transfer ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TransferRequest {
    pub to_club_id: u32,
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
        let fee = body.fee.unwrap_or(0) as f64;

        let (dci, dcoi, dcli, dti) = match sim.find_club_main_team(body.to_club_id) {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let dest = match get_team_info(sim, dci, dcoi, dcli) {
            Some(t) => t,
            None => return StatusCode::NOT_FOUND,
        };

        // Try to take player from a team, or from the free agents pool
        let from_team = sim.find_player_position(params.player_id);

        let (mut player, source_info) = if let Some((ci, coi, cli, ti)) = from_team {
            let source = match get_team_info(sim, ci, coi, cli) {
                Some(t) => t,
                None => return StatusCode::NOT_FOUND,
            };

            let p = match sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                .players
                .take_player(&params.player_id)
            {
                Some(p) => p,
                None => return StatusCode::NOT_FOUND,
            };

            (p, Some((ci, coi, source)))
        } else {
            // Take from free agents pool
            let idx = match sim
                .free_agents
                .iter()
                .position(|p| p.id == params.player_id)
            {
                Some(i) => i,
                None => return StatusCode::NOT_FOUND,
            };
            let p = sim.free_agents.swap_remove(idx);
            (p, None)
        };

        let player_name = player.full_name.to_string();

        if let Some((_, _, ref source)) = source_info {
            player.on_manual_transfer(&source.info, &dest.info, Some(fee), date);
        } else {
            // Free agent: no source club, so a phantom "transfer from
            // dest to dest" would record the destination row twice.
            player.on_free_agent_signing(&dest.info, date);
        }

        // Clear transfer-related statuses: player is joining a new club
        player.statuses.remove(core::PlayerStatusType::Lst);
        player.statuses.remove(core::PlayerStatusType::Trn);
        player.statuses.remove(core::PlayerStatusType::Bid);
        player.statuses.remove(core::PlayerStatusType::Enq);
        player.statuses.remove(core::PlayerStatusType::Wnt);
        player.statuses.remove(core::PlayerStatusType::Req);
        player.statuses.remove(core::PlayerStatusType::Frt);
        player.statuses.remove(core::PlayerStatusType::Unh);

        // Fresh start at new club: reset happiness so old salary/playing-time
        // frustrations don't carry over from the previous club.
        player.happiness.clear();

        // Wage and length come from the canonical contract policy on
        // `Player` — the same one the AI pipeline uses. `agreed_wage =
        // None` means "let the wage calculator decide from ability /
        // age / club + league reputation," which is what we want for a
        // manual signing (the user didn't dictate a number).
        let (club_rep, league_rep) = signing_reputation_inputs(sim, dci, dcoi, dcli);
        player.install_permanent_contract(date, club_rep, league_rep, None);

        // Squad status is club-roster-aware: it depends on the destination
        // team's existing position-group depth. Compute against the
        // pre-add roster (the player isn't in the team yet) and pin it
        // on the freshly-installed contract so the UI shows a sensible
        // value immediately rather than waiting for the monthly batch.
        let player_ca = player.player_attributes.current_ability;
        let player_age = core::utils::DateUtils::age(player.birth_date, date);
        let player_group = player.position().position_group();
        let mut group_cas: Vec<u8> = sim.continents[dci].countries[dcoi].clubs[dcli].teams.teams
            [dti]
            .players
            .players
            .iter()
            .filter(|p| p.position().position_group() == player_group)
            .map(|p| p.player_attributes.current_ability)
            .collect();
        group_cas.push(player_ca);
        group_cas.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(contract) = player.contract.as_mut() {
            contract.squad_status =
                core::PlayerSquadStatus::calculate(player_ca, player_age, &group_cas);
        }

        sim.continents[dci].countries[dcoi].clubs[dcli].teams.teams[dti]
            .players
            .add(player);

        // Record in transfer history
        let transfer_type = if fee > 0.0 {
            TransferType::Permanent
        } else {
            TransferType::Free
        };

        if let Some((_ci, _coi, ref source)) = source_info {
            let completed = CompletedTransfer::new(
                params.player_id,
                player_name,
                source.club_id,
                source.team_id,
                source.info.name.clone(),
                dest.club_id,
                dest.info.name.clone(),
                date,
                CurrencyValue::new(fee, Currency::Usd),
                transfer_type,
            )
            .with_reason("Manual".to_string());

            // Convention (matches the AI pipeline at `transfers/market.rs`):
            // a transfer-history entry lives in the *buying* country only.
            // Pushing to both sides would double-render on the buying
            // team's transfer page, which iterates every country's history
            // to make foreign sales visible.
            sim.continents[dci].countries[dcoi]
                .transfer_market
                .transfer_history
                .push(completed);
        } else {
            // Free agent signing — record only in destination country
            let completed = CompletedTransfer::new(
                params.player_id,
                player_name,
                0,
                0,
                String::from("Free Agent"),
                dest.club_id,
                dest.info.name.clone(),
                date,
                CurrencyValue::new(0.0, Currency::Usd),
                TransferType::Free,
            )
            .with_reason("Manual".to_string());

            sim.continents[dci].countries[dcoi]
                .transfer_market
                .transfer_history
                .push(completed);
        }

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

        let (ci, coi, cli, ti) = match sim.find_player_position(params.player_id) {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let source_club_id = sim.continents[ci].countries[coi].clubs[cli].id;

        // Detect re-loan: preserve original parent club and team
        let source_team_id = sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti].id;
        let (parent_club_id, parent_team_id) = {
            let player = sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                .players
                .players
                .iter()
                .find(|p| p.id == params.player_id);
            player
                .and_then(|p| p.contract_loan.as_ref())
                .map(|c| {
                    (
                        c.loan_from_club_id.unwrap_or(source_club_id),
                        c.loan_from_team_id.unwrap_or(source_team_id),
                    )
                })
                .unwrap_or((source_club_id, source_team_id))
        };

        let source = match get_team_info(sim, ci, coi, cli) {
            Some(t) => t,
            None => return StatusCode::NOT_FOUND,
        };

        let parent = match get_team_info_by_club_id(sim, parent_club_id) {
            Some(t) => t,
            None => return StatusCode::NOT_FOUND,
        };

        let dest_pos = match sim.find_club_main_team(body.to_club_id) {
            Some(pos) => pos,
            None => return StatusCode::NOT_FOUND,
        };

        let dest = match get_team_info(sim, dest_pos.0, dest_pos.1, dest_pos.2) {
            Some(t) => t,
            None => return StatusCode::NOT_FOUND,
        };

        // Get player name before taking
        let player_name = sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
            .players
            .players
            .iter()
            .find(|p| p.id == params.player_id)
            .map(|p| p.full_name.to_string())
            .unwrap_or_default();

        let mut player = match sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
            .players
            .take_player(&params.player_id)
        {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND,
        };

        player.on_manual_loan(&source.info, &parent.info, &dest.info, date);

        // Clear transfer-related statuses
        player.statuses.remove(core::PlayerStatusType::Lst);
        player.statuses.remove(core::PlayerStatusType::Trn);
        player.statuses.remove(core::PlayerStatusType::Bid);
        player.statuses.remove(core::PlayerStatusType::Enq);
        player.statuses.remove(core::PlayerStatusType::Wnt);
        player.statuses.remove(core::PlayerStatusType::Req);
        player.statuses.remove(core::PlayerStatusType::Loa);
        player.statuses.remove(core::PlayerStatusType::Unh);

        // Fresh start at new club: reset happiness so old frustrations
        // don't carry over from the previous club.
        player.happiness.clear();

        // Loan contract with original parent — expiration from league season end
        let seasons = body.seasons.unwrap_or(1).clamp(1, 5) as i32;
        let expiration = {
            let country = &sim.continents[ci].countries[coi];
            let league_end = country.clubs[cli]
                .teams
                .teams
                .iter()
                .find(|t| t.team_type == core::TeamType::Main)
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| {
                    (
                        l.settings.season_ending_half.to_month as u32,
                        l.settings.season_ending_half.to_day as u32,
                    )
                });
            match league_end {
                Some((end_month, end_day)) => {
                    let base_year = if date.month() > end_month
                        || (date.month() == end_month && date.day() > end_day)
                    {
                        date.year() + 1
                    } else {
                        date.year()
                    };
                    chrono::NaiveDate::from_ymd_opt(base_year + (seasons - 1), end_month, end_day)
                        .unwrap_or(date)
                }
                None => {
                    let base_year = if date.month() >= 6 {
                        date.year() + 1
                    } else {
                        date.year()
                    };
                    chrono::NaiveDate::from_ymd_opt(base_year + (seasons - 1), 5, 31)
                        .unwrap_or(date)
                }
            }
        };
        let salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(1000);
        // Match fee based on player salary — parent club pays per official appearance
        let match_fee = (salary / 4).max(500);
        player.contract_loan = Some(
            PlayerClubContract::new_loan(
                salary,
                expiration,
                parent_club_id,
                parent_team_id,
                body.to_club_id,
            )
            .with_loan_match_fee(match_fee),
        );

        sim.continents[dest_pos.0].countries[dest_pos.1].clubs[dest_pos.2]
            .teams
            .teams[dest_pos.3]
            .players
            .add(player);

        // Record in transfer history. Convention (matches the AI
        // pipeline at `transfers/market.rs`): the entry lives in the
        // *borrowing* country only — same rule as permanent transfers,
        // since the team transfers page iterates every country and
        // double-pushing renders the same loan twice.
        let completed = CompletedTransfer::new(
            params.player_id,
            player_name,
            source.club_id,
            source.team_id,
            source.info.name.clone(),
            dest.club_id,
            dest.info.name.clone(),
            date,
            CurrencyValue::new(0.0, Currency::Usd),
            TransferType::Loan(expiration),
        )
        .with_reason("Manual".to_string());

        sim.continents[dest_pos.0].countries[dest_pos.1]
            .transfer_market
            .transfer_history
            .push(completed);

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
    pub country: String,
}

#[derive(Deserialize)]
pub struct ClubsQuery {
    pub q: Option<String>,
    pub exclude: Option<u32>,
}

pub async fn list_clubs_action(
    State(state): State<GameAppData>,
    Query(query): Query<ClubsQuery>,
) -> Json<Vec<ClubListItem>> {
    let guard = state.data.read().await;

    let mut clubs = Vec::new();
    let exclude_id = query.exclude.unwrap_or(0);

    if let Some(ref sim) = *guard {
        let filter = query.q.as_deref().unwrap_or("").to_lowercase();
        for continent in &sim.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    if club.id == exclude_id {
                        continue;
                    }
                    if filter.is_empty() || club.name.to_lowercase().contains(&filter) {
                        clubs.push(ClubListItem {
                            id: club.id,
                            name: club.name.clone(),
                            country: country.name.clone(),
                        });
                    }
                }
            }
        }
    }

    clubs.sort_by(|a, b| a.name.cmp(&b.name));
    Json(clubs)
}
