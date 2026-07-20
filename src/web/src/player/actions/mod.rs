pub mod routes;

use crate::GameAppData;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Datelike;
use core::club::player::calculators::WageCalculator;
use core::club::player::transfer::ReleaseContext;
use core::shared::{Currency, CurrencyValue};
use core::transfers::pipeline::PipelineProcessor;
use core::transfers::{CompletedTransfer, TransferType};
use core::{Person, PlayerClubContract, PlayerSquadStatus, SimulatorData};
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

/// Stats-history identity of the team a player physically occupies
/// (`teams[ti]`), mirroring the seeder's rule (`ClubSeedingContext::
/// team_info_for`): Main / B / Second compete under their own brand, so
/// they keep their own slug + league, while Reserve / U18..U23 squads
/// alias to the club's Main team.
///
/// The history layer keys every career spell by `team_slug`, so a release
/// / transfer / loan must mark the player's OWN active spell departed. The
/// plain `get_team_info` always returns the club Main team — correct for an
/// aliased youth/Reserve player, but WRONG for a B/Second player: it leaves
/// that player's real spell `departed_date: None`, so the projection treats
/// it as the still-active spell and hides the genuinely-new destination club
/// on the History page as phantom noise.
fn get_source_history_info(
    sim: &core::SimulatorData,
    ci: usize,
    coi: usize,
    cli: usize,
    ti: usize,
) -> Option<core::TeamInfo> {
    let team = sim.continents[ci].countries[coi].clubs[cli]
        .teams
        .teams
        .get(ti)?;
    if !team.team_type.is_own_team() {
        // Aliased squad — same identity the seeder gave the spell: the
        // club's Main team.
        return get_team_info(sim, ci, coi, cli).map(|t| t.info);
    }
    let league = team
        .league_id
        .and_then(|lid| sim.league(lid))
        .map(|l| (l.name.clone(), l.slug.clone()));
    Some(core::TeamInfo {
        name: team.name.clone(),
        slug: team.slug.clone(),
        reputation: team.reputation.world,
        league_name: league.as_ref().map(|l| l.0.clone()).unwrap_or_default(),
        league_slug: league.map(|l| l.1).unwrap_or_default(),
    })
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

/// Direct editor override: releases the player to the global free-agent
/// pool unconditionally, deliberately bypassing the
/// `AutomaticReleaseEligibility` gate the AI release pipelines must pass.
/// Everything else matches the automatic sweep
/// (`SimulatorData::sweep_released_to_free_agents`): the same
/// `dec_reason_released_free` transfer-history entry, the same
/// `ReleaseContext` market-state snapshot (so free-agent recruitment
/// pressure and retirement logic treat the player identically), and the
/// same release-flavoured cleanup of stale listings, team transfer
/// lists, scouting interest, and live negotiations.
///
/// Returns `false` when the player isn't rostered on any team.
pub(crate) fn execute_move_on_free(sim: &mut SimulatorData, player_id: u32) -> bool {
    let date = sim.date.date();

    let (ci, coi, cli, ti) = match sim.find_player_position(player_id) {
        Some(pos) => pos,
        None => return false,
    };

    // Identity of the team the player actually plays for — a B/Second
    // player departs their own spell, not the club Main team.
    let from_info = match get_source_history_info(sim, ci, coi, cli, ti) {
        Some(t) => t,
        None => return false,
    };

    // Physical-team identity for the transfer-history record (the same
    // source the automatic sweep stamps on its departures) plus the
    // reputation inputs the market-state snapshot needs — all captured
    // before the player leaves the roster.
    let (
        from_club_id,
        from_team_id,
        from_team_name,
        country_id,
        country_reputation,
        league_reputation,
        team_reputation_world,
    ) = {
        let country = &sim.continents[ci].countries[coi];
        let club = &country.clubs[cli];
        let team = &club.teams.teams[ti];
        let league_reputation = team
            .league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(country.reputation);
        (
            club.id,
            team.id,
            team.name.clone(),
            country.id,
            country.reputation,
            league_reputation,
            team.reputation.world,
        )
    };

    let mut player = match sim.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
        .players
        .take_player(&player_id)
    {
        Some(p) => p,
        None => return false,
    };

    // Snapshot in-flight competitive stats onto the source club's
    // career row before the player leaves it; otherwise those games
    // would later be misattributed to the destination — or, in the
    // global-pool path, to a synthetic "Free Agent" row.
    player.on_release(&from_info, date);

    // Market-state snapshot — the same `ReleaseContext` the automatic
    // sweep stamps. Unlike the sweep (whose upstream pipelines already
    // cleared the contract), the real contract is still present here, so
    // salary and squad status are read off it; the sweep's wage-estimate
    // fallback only covers a contractless edge case.
    let club_score = (team_reputation_world as f32 / 10_000.0).clamp(0.0, 1.0);
    let last_salary = player
        .contract
        .as_ref()
        .map(|c| c.salary)
        .unwrap_or_else(|| {
            WageCalculator::expected_annual_wage(
                &player,
                player.age(date),
                club_score,
                league_reputation,
            )
        });
    let last_squad_status = player
        .contract
        .as_ref()
        .map(|c| c.squad_status.clone())
        .unwrap_or(PlayerSquadStatus::FirstTeamSquadRotation);
    if player.free_agent_state().is_none() {
        player.enter_free_agent_market(ReleaseContext {
            date,
            last_club_id: Some(from_club_id),
            last_country_id: Some(country_id),
            last_country_reputation: country_reputation,
            last_league_reputation: league_reputation,
            last_club_reputation_score: club_score,
            last_salary,
            last_squad_status,
        });
    }

    player.contract = None;
    player.contract_loan = None;
    // Canonical end-of-spell reset — the same one the AI release
    // sweep and transfer completion paths run: transient transfer
    // statuses (Lst / Loa / Frt / Req / Unh / ...) plus happiness.
    player.reset_on_club_change();

    let completed = CompletedTransfer::new(
        player_id,
        player.full_name.to_string(),
        from_club_id,
        from_team_id,
        from_team_name,
        0,
        String::from("Free Agent"),
        date,
        CurrencyValue::new(0.0, Currency::Usd),
        TransferType::Free,
    )
    .with_reason("dec_reason_released_free".to_string());
    sim.continents[ci].countries[coi]
        .transfer_market
        .transfer_history
        .push(completed);

    sim.free_agents.push(player);

    // The departed player must vanish from every market surface — open
    // listings end Cancelled, team transfer lists drop their rows,
    // scouting interest is cleared, live negotiations are rejected —
    // exactly like the automatic sweep's release cleanup.
    PipelineProcessor::cleanup_player_release_interest(sim, player_id);

    sim.rebuild_indexes();
    true
}

pub async fn move_on_free_action(
    State(state): State<GameAppData>,
    Path(params): Path<PlayerPathParam>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let sim = Arc::make_mut(arc_data);
        if execute_move_on_free(sim, params.player_id) {
            return StatusCode::OK;
        }
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

        // Identity of the team the player actually plays for (own slug for
        // B/Second, Main alias for Reserve/youth) — the spell that must be
        // marked departed. `None` for a free-agent signing.
        let source_history =
            from_team.and_then(|(ci, coi, cli, ti)| get_source_history_info(sim, ci, coi, cli, ti));

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

        // Capture source-club reps BEFORE the player's `on_manual_*`
        // (which clears `last_transfer_date` & resets) but they live on
        // the destination context anyway — pull from the source `TeamInfo`.
        let source_club_reputation = source_info
            .as_ref()
            .map(|(_, _, s)| s.info.reputation)
            .unwrap_or(0);
        let source_league_reputation = source_info
            .as_ref()
            .and_then(|(ci, coi, s)| {
                let country = &sim.continents[*ci].countries[*coi];
                country
                    .clubs
                    .iter()
                    .find(|c| c.id == s.club_id)
                    .and_then(|c| c.teams.main())
                    .and_then(|t| t.league_id)
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| l.reputation)
            })
            .unwrap_or(0);

        if let Some((_, _, ref source)) = source_info {
            // Depart the player's OWN spell (B/Second keep their slug);
            // fall back to the club Main team if resolution failed.
            let from = source_history.as_ref().unwrap_or(&source.info);
            player.on_manual_transfer(from, &dest.info, Some(fee), date);
        } else {
            // Free agent: no source club, so a phantom "transfer from
            // dest to dest" would record the destination row twice.
            player.on_free_agent_signing(&dest.info, date);
        }

        // Stage the pending signing BEFORE clearing happiness so the
        // desire-carry snapshot can read recent `WantsReturnHome` /
        // `WantsEuropeanCompetition` / `WantsCopaLibertadores` moods and
        // surface the matching satisfaction events on the next sim tick.
        // Position depth rank against the pre-add roster matches the
        // squad-status calculation below: 1 = clear first choice.
        let player_group = player.position().position_group();
        let player_ca = player.player_attributes.current_ability;
        // Existing roster only — don't push the new arrival in until
        // depth rank has been computed. The existing-CAs vector also
        // feeds the squad-status calculation below, but THERE the new
        // arrival is included (squad-status reflects post-signing depth).
        let existing_group_cas: Vec<u8> =
            sim.continents[dci].countries[dcoi].clubs[dcli].teams.teams[dti]
                .players
                .players
                .iter()
                .filter(|p| p.position().position_group() == player_group)
                .map(|p| p.player_attributes.current_ability)
                .collect();
        // Depth rank = 1 + number of strictly-better existing teammates
        // at the same position group. New arrivals tied on CA with
        // incumbents land BEHIND them (incumbency tiebreak).
        let depth_rank = (existing_group_cas
            .iter()
            .filter(|ca| **ca > player_ca)
            .count()
            + 1)
        .min(255) as u8;
        player.stage_manual_pending_signing(
            dest.club_id,
            fee,
            false,
            source_club_reputation,
            source_league_reputation,
            Some(depth_rank),
        );

        // Fresh start at new club — the canonical end-of-spell reset the
        // AI pipeline runs on completion: transfer statuses plus
        // happiness, so old salary/playing-time frustrations don't carry
        // over. The most recent `WantsReturnHome` etc. moods survive
        // through the staged `desire_carry` captured above.
        player.reset_on_club_change();

        // Wage and length come from the canonical contract policy on
        // `Player` — the same one the AI pipeline uses. `agreed_wage =
        // None` means "let the wage calculator decide from ability /
        // age / club + league reputation," which is what we want for a
        // manual signing (the user didn't dictate a number).
        let (club_rep, league_rep) = signing_reputation_inputs(sim, dci, dcoi, dcli);
        player.install_permanent_contract(date, club_rep, league_rep, None);

        // Squad status is club-roster-aware: it depends on the destination
        // team's full position-group depth (existing teammates + the new
        // arrival). Compute and pin on the freshly-installed contract so
        // the UI shows a sensible value immediately. Team-aware: a signing
        // parked in a reserve/development squad must not be crowned "Key
        // Player" of a squad that owns no role labels.
        let dest_team_type =
            sim.continents[dci].countries[dcoi].clubs[dcli].teams.teams[dti].team_type;
        let player_age = core::utils::DateUtils::age(player.birth_date, date);
        let player_group = player.position().position_group();
        let mut full_group_cas = existing_group_cas.clone();
        full_group_cas.push(player_ca);
        full_group_cas.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(contract) = player.contract.as_mut() {
            contract.squad_status = core::PlayerSquadStatus::calculate_for_team(
                dest_team_type,
                &contract.squad_status,
                player_ca,
                player_age,
                player_group,
                &full_group_cas,
            );
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

        // History identity of the squad the player is actually loaned out OF
        // (own slug for B/Second, Main alias for Reserve/youth). The stats
        // layer must depart THIS spell — using the club Main team would leave
        // a reserve player's real spell active and hide the loan club from the
        // History page.
        let source_history = match get_source_history_info(sim, ci, coi, cli, ti) {
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

        // Capture source-side reps + dest position depth BEFORE clearing
        // happiness / mutating roster — these feed the staged
        // `pending_signing` so the next sim tick can emit the same
        // shock / role-fit / promise events the AI loan path emits.
        let source_club_reputation = source.info.reputation;
        let source_league_reputation = {
            let country = &sim.continents[ci].countries[coi];
            country
                .clubs
                .iter()
                .find(|c| c.id == source.club_id)
                .and_then(|c| c.teams.main())
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0)
        };
        let player_group = player.position().position_group();
        let player_ca = player.player_attributes.current_ability;
        let existing_group_cas: Vec<u8> = sim.continents[dest_pos.0].countries[dest_pos.1].clubs
            [dest_pos.2]
            .teams
            .teams[dest_pos.3]
            .players
            .players
            .iter()
            .filter(|p| p.position().position_group() == player_group)
            .map(|p| p.player_attributes.current_ability)
            .collect();
        // Depth rank = 1 + number of strictly-better existing teammates
        // at the same position group. Equal-CA incumbents win the tie.
        let depth_rank = (existing_group_cas
            .iter()
            .filter(|ca| **ca > player_ca)
            .count()
            + 1)
        .min(255) as u8;

        player.on_manual_loan(&source_history, &parent.info, &dest.info, date);

        // Stage the loan pending-signing BEFORE the club-change reset so the
        // desire-carry snapshot captures recent home/EU/Libertadores moods.
        // For a loan the destination is the borrowing club; previous
        // salary is the parent contract's salary (process_transfer_shock
        // skips salary shock for loans anyway).
        player.stage_manual_pending_signing(
            body.to_club_id,
            0.0,
            true,
            source_club_reputation,
            source_league_reputation,
            Some(depth_rank),
        );

        // Fresh start at the borrowing club — the same canonical reset
        // the AI loan completion runs: transfer statuses plus happiness,
        // so old frustrations don't carry over.
        player.reset_on_club_change();

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
        // Mirror AI loan execution: extend the parent contract so it
        // outlives the loan end. Without this a player can be loaned out
        // until June and have his parent contract expire in March, leaving
        // the parent with no recall leverage and an inevitable free-agent
        // walk-out at loan return.
        player.ensure_contract_covers_loan_end(expiration);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};
    use core::club::ClubAcademy;
    use core::club::player::builder::PlayerBuilder;
    use core::competitions::global::GlobalCompetitions;
    use core::continent::Continent;
    use core::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use core::shared::Location;
    use core::shared::fullname::FullName;
    use core::transfers::{TransferListing, TransferListingStatus, TransferListingType};
    use core::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TeamType,
        TrainingSchedule, TransferItem,
    };

    /// World fixture for the manual-release action: one country (id 1)
    /// with one league (id 1), one club (id 100) whose Main team (id 10)
    /// rosters a single contracted player (id 1, salary 80k,
    /// MainBackupPlayer).
    struct Fixture;

    impl Fixture {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 12).unwrap()
        }

        fn sim() -> SimulatorData {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 90;
            attrs.potential_ability = 90;
            let mut contract =
                PlayerClubContract::new(80_000, NaiveDate::from_ymd_opt(2027, 6, 30).unwrap());
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
            let player = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(1996, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap();
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(vec![player]))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 4_000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            let club = Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            );
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            let country = Country::builder()
                .id(1)
                .code("EN".to_string())
                .slug("en".to_string())
                .name("England".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap();
            let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
            SimulatorData::new(
                Self::date().and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            )
        }
    }

    #[test]
    fn move_on_free_seeds_market_state_history_and_cleanup() {
        let mut sim = Fixture::sim();
        // Stale market state pointing at the player: an open country
        // listing and a team transfer-list row.
        sim.continents[0].countries[0]
            .transfer_market
            .add_listing(TransferListing::new(
                1,
                100,
                10,
                CurrencyValue::new(250_000.0, Currency::Usd),
                Fixture::date(),
                TransferListingType::Transfer,
            ));
        sim.continents[0].countries[0].clubs[0].teams.teams[0]
            .transfer_list
            .add(TransferItem::new(
                1,
                CurrencyValue::new(250_000.0, Currency::Usd),
            ));

        assert!(execute_move_on_free(&mut sim, 1));

        let released = sim
            .free_agents
            .iter()
            .find(|p| p.id == 1)
            .expect("manual release must move the player into the global pool");
        assert!(released.contract.is_none());
        let state = released
            .free_agent_state()
            .expect("manual release must seed the free-agent market state");
        assert_eq!(state.last_club_id, Some(100));
        assert_eq!(
            state.last_salary, 80_000,
            "the real contract salary is carried, not an estimate"
        );
        assert!(matches!(
            state.last_squad_status,
            PlayerSquadStatus::MainBackupPlayer
        ));
        assert_eq!(state.free_since, Fixture::date());

        let country = &sim.continents[0].countries[0];
        let entry = country
            .transfer_market
            .transfer_history
            .iter()
            .find(|t| t.player_id == 1)
            .expect("manual release must record transfer history");
        assert_eq!(entry.reason, "dec_reason_released_free");
        assert_eq!(entry.from_club_id, 100);

        // Same release-flavoured cleanup the automatic sweep runs.
        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 1)
            .unwrap();
        assert_eq!(
            listing.status,
            TransferListingStatus::Cancelled,
            "the stale listing must end Cancelled, not stay Available"
        );
        assert!(
            country.clubs[0].teams.teams[0]
                .transfer_list
                .listed_player_ids()
                .is_empty(),
            "the team transfer list must drop the released player"
        );
    }

    #[test]
    fn move_on_free_unknown_player_is_rejected() {
        let mut sim = Fixture::sim();
        assert!(!execute_move_on_free(&mut sim, 999));
        assert!(sim.free_agents.is_empty());
    }
}
