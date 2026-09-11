pub(crate) mod config;
pub(crate) mod execution;
pub mod free;
mod listings;
mod negotiations;

pub use listings::ListingPass;
pub use negotiations::NegotiationPass;
pub(crate) mod settlement;
pub(crate) mod types;

use crate::club::player::events::transfer_social::TransferInterestSignal;
use crate::club::player::transfer::FreeAgentBlockReason;
use crate::country::result::transfers::free::FreeAgentPass;
use crate::country::result::transfers::free::{FreeAgentLedger, FreeAgentWorld};
use crate::simulator::{PerformanceProfiler, SimulatorData};
use crate::transfers::NegotiationStatus;
use crate::transfers::TransferWindowManager;
use crate::transfers::loan::LoanPipeline;
use crate::transfers::pipeline::MarketCirculation;
use crate::transfers::pipeline::PlayerSummary;
use crate::transfers::pipeline::StaffRecommendations;
use crate::transfers::pipeline::approach::ApproachPass;
use crate::transfers::pipeline::shortlist::ShortlistPass;
use crate::transfers::scouting::ScoutingPass;
use crate::transfers::scouting::recruitment::meeting::MeetingPass;
use crate::transfers::scouting::watch::FormWatch;
use crate::transfers::scouting::watchlist::Watchlist;
use crate::transfers::squad::SquadReviewPass;
use crate::transfers::{MarketMap, ScoutMarketDesk};
use crate::{Country, PlayerStatusType};
use chrono::NaiveDate;
use config::TransferConfig;
use execution::TransferExecutor;
use free::GlobalFreeAgentSigning;
use free::precontract::PreContractManager;
pub(crate) use free::{GlobalFreeAgentPool, GlobalFreeAgentSummary};
use log::debug;
use settlement::TransferClauseSettler;
use types::DeferredTransfer;
use types::TransferActivitySummary;
use types::{CountryRoster, PendingPlayerSignal};

/// Cross-country tail of the transfer market — populated by
/// `simulate_transfer_market_local` running on `&mut Country` inside
/// Phase A, drained by `apply_deferred_transfer_ops` on `&mut
/// SimulatorData` in Phase C. Keeps the heavy per-country pipeline
/// (scouting, negotiations, squad eval, shadow reports, …) on the
/// parallel side; the global writes that reach into other countries
/// or `data.free_agents` stay serial.
pub struct DeferredTransferOps {
    pub country_id: u32,
    pub window_open: bool,
    /// Per-country sweep targets — every domestic signing in Phase A
    /// needs `cleanup_player_transfer_interest` to run against *every*
    /// other country's shortlists. Serial in Phase C.
    pub domestic_signed_ids: Vec<u32>,
    /// Free-agent (`data.free_agents` global pool) candidates that
    /// fielded an offer today; bumps the 30-day window counter.
    pub global_offered_ids: Vec<u32>,
    /// Subset of `global_offered_ids` whose acceptance roll failed;
    /// bumps the rejected-total counter.
    pub global_rejected_ids: Vec<u32>,
    /// Why each skipped global-pool candidate was passed over today —
    /// highest-rank reason per player from this country's matcher.
    /// Stamped onto `FreeAgentMarketState::last_block` in Phase C so
    /// the audit layer can explain long sits.
    pub global_block_reasons: Vec<(u32, FreeAgentBlockReason)>,
    /// Free-agent signings to execute against `data.free_agents` after
    /// the parallel pass joins.
    pub global_signings: Vec<GlobalFreeAgentSigning>,
    /// Domestic + cross-country transfers ready for the unified
    /// `execution::execute_transfer` path.
    pub deferred_transfers: Vec<DeferredTransfer>,
    /// `summary.completed_transfers` at the start of this country's
    /// local pass — Phase C compares against `completed_after` to
    /// decide whether to flag `data.dirty_player_index`.
    pub completed_before: u32,
    /// Roster mutations from `handle_free_agents` that already
    /// landed in Phase A. Phase C checks this against `completed_before`
    /// so it can dirty the player index without re-counting.
    pub completed_after: u32,
    /// Pre-contract (Bosman) free moves executed in this country's Phase-A
    /// pass — a subset of `domestic_signed_ids`. Phase C splits the monthly
    /// "signed pre-contract" vs "signed off a domestic expiry" counters
    /// from this plus `domestic_signed_ids.len()`.
    pub pre_contract_signed: u32,
    /// Clause payouts owed to sellers that don't live in this country —
    /// `(club_id, amount)` pairs the settler couldn't route locally.
    /// Drained serially in Phase C via `TransferExecutor::credit_club` so a
    /// cross-country performance add-on actually reaches the foreign
    /// seller instead of vanishing (the buyer was already debited).
    pub cross_country_clause_credits: Vec<(u32, f64)>,
    /// Saga beats for players in OTHER countries — the negotiation runs
    /// in this (buying) country's parallel pass, the player lives in the
    /// seller's partition. Delivered serially in Phase C; without this
    /// every cross-border move was silent to the player at every stage.
    pub(crate) player_signals: Vec<PendingPlayerSignal>,
}

impl DeferredTransferOps {
    pub fn empty(country_id: u32) -> Self {
        DeferredTransferOps {
            country_id,
            window_open: false,
            domestic_signed_ids: Vec::new(),
            global_offered_ids: Vec::new(),
            global_rejected_ids: Vec::new(),
            global_block_reasons: Vec::new(),
            global_signings: Vec::new(),
            deferred_transfers: Vec::new(),
            completed_before: 0,
            completed_after: 0,
            pre_contract_signed: 0,
            cross_country_clause_credits: Vec::new(),
            player_signals: Vec::new(),
        }
    }
}

/// One country's transfer day: what it settles, what it plans, and what it defers to the world.
pub struct TransferTick;

impl TransferTick {
    /// Phase-A entry: runs the country-local transfer market pipeline
    /// (negotiations, free agents, listings, scouting, recruitment
    /// meetings, board approvals, shadow reports) on `&mut Country`.
    /// Cross-country writes — `data.free_agents` mutation, the
    /// per-country shortlist sweep, transfer execution that moves
    /// players between countries, foreign-negotiation initiation —
    /// land in the returned `DeferredTransferOps` and the simulator
    /// drains them serially in Phase C via `apply_deferred_transfer_ops`.
    pub(crate) fn simulate_transfer_market_local(
        country: &mut Country,
        current_date: NaiveDate,
        world_pool: &[PlayerSummary],
        global_free_agents: &[GlobalFreeAgentSummary],
        market_map: &MarketMap,
    ) -> DeferredTransferOps {
        let country_id = country.id;
        let mut summary = TransferActivitySummary::new();
        let window_manager =
            TransferWindowManager::for_country(country.id, &country.code, current_date);
        let window_open = window_manager.is_window_open(country_id, current_date);
        let config = TransferConfig::default();

        // Filter foreign players from the pre-built world snapshot.
        // Borrow, don't clone: every country used to deep-copy the entire
        // world pool minus its own players (three `String`s per summary),
        // which dominated the per-country transfer pass. References into
        // the shared snapshot cost nothing — which is also why the view
        // exists year-round now: the breakout watch runs weekly whatever
        // the window says (scouts don't stop watching football in
        // October), while the negotiation-side passes stay window-gated.
        let foreign_players: Vec<&PlayerSummary> = world_pool
            .iter()
            .filter(|s| s.country_id != country_id)
            .collect();

        let completed_before = summary.completed_transfers;
        let mut ops = DeferredTransferOps::empty(country_id);
        ops.window_open = window_open;
        ops.completed_before = completed_before;

        // Detect the open→closed transition BEFORE syncing the flag —
        // the market's stored flag still holds yesterday's state.
        let window_just_closed = country.transfer_market.transfer_window_open && !window_open;

        // Sync market's window flag. Listings and negotiations survive
        // the close (the listing rows are the durable "still for sale"
        // clock); the just-closed beat below tells unsold listed
        // players their limbo is real until the next window.
        country.transfer_market.check_transfer_window(window_open);
        if window_just_closed {
            ListingPass::emit_window_close_limbo(country, current_date);
        }

        Self::settle_open_business(
            country,
            current_date,
            market_map,
            &config,
            global_free_agents,
            &mut summary,
            &mut ops,
        );

        Self::run_year_round_passes(
            country,
            current_date,
            world_pool,
            &foreign_players,
            market_map,
        );

        if window_open {
            Self::run_window_passes(
                country,
                current_date,
                &foreign_players,
                market_map,
                &mut summary,
            );
        }

        Self::run_year_round_tail(country, current_date, &foreign_players);

        ops.completed_after = summary.completed_transfers;
        ops.pre_contract_signed = summary.signed_pre_contract;
        debug!(
            "Transfer Activity (Phase A) - Listings: {}, Negotiations: {}, Completed: {}",
            summary.total_listings, summary.active_negotiations, summary.completed_transfers
        );

        ops
    }

    /// Phase-C tail: apply the cross-country mutations the parallel
    /// Phase-A pass stashed into `DeferredTransferOps`. Runs against
    /// `&mut SimulatorData` so it can sweep every country's shortlist
    /// for a signed player, mutate `data.free_agents`, and execute
    /// transfers that move players between countries.
    pub(crate) fn apply_deferred_transfer_ops(
        data: &mut SimulatorData,
        ops: DeferredTransferOps,
        current_date: NaiveDate,
    ) {
        let config = TransferConfig::default();

        // Cross-country interest sweep used to fire per signed id here.
        // Hoisted into the simulator orchestrator: it now aggregates
        // every country's `domestic_signed_ids` and calls
        // `cleanup_player_transfer_interest_batch` once per tick,
        // collapsing O(countries × signings × countries) into a single
        // parallel pass over the world.

        // Free-agent market-state bumps (offer / reject / block-reason)
        // were likewise hoisted into the orchestrator: every country's
        // `global_offered_ids` / `global_rejected_ids` /
        // `global_block_reasons` are aggregated into a single
        // `FreeAgentBumpBatch` and applied in ONE pass over
        // `data.free_agents` per tick via
        // `ApproachPass::apply_free_agent_market_bumps_batch`,
        // collapsing the old O(countries × pool) double-walk.

        // Route clause credits owed to sellers outside the settling
        // country — the buyer was debited during the parallel Phase-A
        // pass; the foreign seller is credited here or the money is
        // destroyed.
        let stage = PerformanceProfiler::stage_scope("drain_clause_credits", 3);
        for (club_id, amount) in &ops.cross_country_clause_credits {
            TransferExecutor::credit_club(data, *club_id, *amount);
        }
        drop(stage);

        // Deliver saga beats to foreign players — the buying country's
        // parallel pass couldn't reach them. Player-dependent facts
        // (former club, homecoming, seller rivalry) resolve here where
        // the seller's country is addressable. Delivered BEFORE the
        // deferred executions below so a player whose deal collapsed
        // hears about it before any unrelated roster churn.
        PerformanceProfiler::stage("drain_player_signals", 3, || {
            Self::deliver_pending_player_signals(data, &ops.player_signals)
        });

        // Execute global free-agent signings (Move-on-Free players from
        // `data.free_agents`).
        let mut completed = ops.completed_after;
        let stage = PerformanceProfiler::stage_scope("drain_free_agent_signings", 3);
        let mut placed_from_pool: Vec<u32> = Vec::new();
        for signing in &ops.global_signings {
            if GlobalFreeAgentPool::execute_signing(data, signing, current_date, &config) {
                completed += 1;
                placed_from_pool.push(signing.player_id);
            }
        }
        // One world sweep for everyone this country just took out of the
        // pool, not one per signing. Still ahead of the foreign-negotiation
        // kickoff below, so no club can open a saga for a player who was
        // signed a moment ago.
        ApproachPass::cleanup_player_transfer_interest_batch(data, &placed_from_pool);

        drop(stage);

        // If anything moved this tick the global indexes need refreshing.
        if completed > ops.completed_before {
            data.dirty_player_index = true;
        }

        // Monthly diagnostics flow: every domestic signed id is an
        // in-country free-agent signing (the global-pool ones defer to
        // `global_signings` above and are counted by the executor). Split
        // them into the pre-contract subset carried up on `ops` and the
        // remaining ordinary domestic-expiry signings.
        let pre_contract = ops.pre_contract_signed;
        let domestic_expiry = (ops.domestic_signed_ids.len() as u32).saturating_sub(pre_contract);
        data.free_agent_flow.signed_pre_contract = data
            .free_agent_flow
            .signed_pre_contract
            .saturating_add(pre_contract);
        data.free_agent_flow.signed_same_country_expired = data
            .free_agent_flow
            .signed_same_country_expired
            .saturating_add(domestic_expiry);

        // Phase 2: Execute all completed transfers (domestic + foreign).
        // One call for the whole country: the executor moves every player,
        // then sweeps the world once for all of them, then stages the
        // development loans — the same order a per-transfer call keeps, at
        // one world walk instead of one per transfer.
        let stage = PerformanceProfiler::stage_scope("drain_execute_transfers", 3);
        let outcomes =
            execution::TransferExecutor::batch(data, &ops.deferred_transfers, current_date);
        for (transfer, success) in ops.deferred_transfers.iter().zip(outcomes) {
            if success {
                data.dirty_player_index = true;
                continue;
            }
            if let Some(country) = data.country_mut(transfer.buying_country_id) {
                country.transfer_market.transfer_history.retain(|t| {
                    !(t.player_id == transfer.player_id
                        && t.to_club_id == transfer.buying_club_id
                        && t.transfer_date == current_date)
                });
            }
            // The deal was optimistically finalised at medical stage but
            // never executed — roll the market state back so the player
            // stays visible and the buyer keeps looking.
            execution::TransferExecutor::compensate_failure(data, transfer);
        }
        drop(stage);

        // Phase 3: Foreign negotiation initiation (domestic priority).
        if ops.window_open {
            PerformanceProfiler::stage("drain_foreign_negotiations", 3, || {
                ApproachPass::initiate_foreign_negotiations(data, ops.country_id, current_date)
            });
        }
    }

    /// Serial Phase-C delivery of cross-border saga beats. Each pending
    /// signal names the country the player lives in; the seller-side
    /// facts the parallel pass couldn't read (league reputation,
    /// rivalry) and the player-dependent facts (former club, homecoming)
    /// are resolved here before the structured signal is handed to the
    /// player exactly as a domestic beat would be.
    fn deliver_pending_player_signals(data: &mut SimulatorData, signals: &[PendingPlayerSignal]) {
        for pending in signals {
            let Some(country) = data.country_mut(pending.selling_country_id) else {
                continue;
            };
            let (is_rival, seller_league_rep) = country
                .clubs
                .iter()
                .find(|c| c.id == pending.selling_club_id)
                .map(|club| {
                    let league_rep = club
                        .teams
                        .teams
                        .first()
                        .and_then(|t| t.league_id)
                        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                        .map(|l| l.reputation)
                        .unwrap_or(0);
                    (club.is_rival(pending.interested_club_id), league_rep)
                })
                .unwrap_or((false, 0));
            let Some(player) = CountryRoster::find_mut(country, pending.player_id) else {
                continue;
            };
            let sig = TransferInterestSignal {
                interested_club_id: pending.interested_club_id,
                interested_league_id: pending.interested_league_id,
                buyer_rep: pending.buyer_rep,
                seller_rep: pending.seller_rep,
                buyer_league_rep: pending.buyer_league_rep,
                seller_league_rep,
                stage: pending.stage,
                source: pending.source,
                repeated_attention: pending.repeated_attention,
                is_rival,
                is_home_country: player.country_id == pending.buyer_country_id,
                is_seller_in_home_country: player.country_id == pending.selling_country_id,
                is_former_club: player
                    .sold_from
                    .as_ref()
                    .map(|(cid, _)| *cid == pending.interested_club_id)
                    .unwrap_or(false),
                buyer_country_id: pending.buyer_country_id,
                buyer_continent_id: pending.buyer_continent_id,
                buyer_has_continental_path: pending.buyer_has_continental_path,
                buyer_competition_path: pending.buyer_competition_path,
            };
            player.on_transfer_interest_signal(&sig);
        }
    }

    /// Business already on the books, settled before anything new is opened:
    /// negotiations that resolved or expired, instalments and add-ons that
    /// came due, free agents, and the pre-contracts a Bosman window allows.
    #[allow(clippy::too_many_arguments)]
    fn settle_open_business(
        country: &mut Country,
        current_date: NaiveDate,
        market_map: &MarketMap,
        config: &TransferConfig,
        global_free_agents: &[GlobalFreeAgentSummary],
        mut summary: &mut TransferActivitySummary,
        ops: &mut DeferredTransferOps,
    ) {
        let country_name = country.name.clone();

        // Resolve pending negotiations — club-to-club moves for the
        // deferred execution queue, plus free-agent negotiation
        // outcomes: pool signings whose medical just cleared (executed
        // against `data.free_agents` in Phase C) and rejected-offer
        // counters for pool players who declined personal terms.
        let outcomes = PerformanceProfiler::stage_labelled(
            "tm_resolve_negotiations",
            3,
            || country_name.clone(),
            || {
                NegotiationPass::resolve_pending_negotiations(
                    country,
                    current_date,
                    market_map,
                    &mut summary,
                )
            },
        );
        ops.deferred_transfers = outcomes.deferred;
        ops.global_signings = outcomes.free_agent_signings;
        ops.global_rejected_ids = outcomes.free_agent_rejected_ids;
        ops.player_signals = outcomes.player_signals;

        // Expire stale negotiations. A dead saga must also surrender the
        // player's Bid/Trn badges — a leaked `Trn` would quietly bench a
        // domestic player forever (selection rests near-sold assets).
        let expired = country.transfer_market.update(current_date);
        for (buying_club_id, player_id) in expired {
            ApproachPass::on_negotiation_resolved(country, buying_club_id, player_id, false);
            let saga_still_live = country.transfer_market.negotiations.values().any(|n| {
                n.player_id == player_id
                    && matches!(
                        n.status,
                        NegotiationStatus::Pending | NegotiationStatus::Countered
                    )
            });
            if !saga_still_live {
                if let Some(player) = CountryRoster::find_mut(country, player_id) {
                    player.statuses.remove(PlayerStatusType::Bid);
                    player.statuses.remove(PlayerStatusType::Trn);
                }
            }
        }

        // Settle any installment tranches that came due today and any
        // performance / promotion add-ons whose triggers have just
        // fired. The settler routes cash buyer → seller (or buyer →
        // beneficiary for sell-on) so the deal's deferred cost
        // actually lands on the books over time. Credits owed to
        // foreign sellers can't be applied inside this country borrow —
        // they ride up on `ops` and drain globally in Phase C.
        ops.cross_country_clause_credits = PerformanceProfiler::stage_labelled(
            "tm_clause_settle",
            3,
            || country_name.clone(),
            || TransferClauseSettler::settle_due(country, current_date),
        );

        // Free agents and contract expirations. Returns deferred
        // signings sourced from the global pool (`data.free_agents`),
        // which we execute after the country borrow ends — appended to
        // the negotiation-driven pool signings collected above.
        let pool_signings = PerformanceProfiler::stage_labelled(
            "tm_handle_free_agents",
            3,
            || country_name.clone(),
            || {
                FreeAgentPass::handle_free_agents(
                    country,
                    current_date,
                    &FreeAgentWorld {
                        global_pool: global_free_agents,
                        market_map,
                        config: &config,
                    },
                    &mut FreeAgentLedger {
                        summary: &mut summary,
                        domestic_signed_ids: &mut ops.domestic_signed_ids,
                        global_offered_ids: &mut ops.global_offered_ids,
                        global_rejected_ids: &mut ops.global_rejected_ids,
                        global_blocked: &mut ops.global_block_reasons,
                    },
                )
            },
        );
        ops.global_signings.extend(pool_signings);

        // Pre-contracts (Bosman): stage future free transfers for useful
        // players in the final months of an expiring deal their club won't
        // renew, so they move directly to a domestic rival on expiry
        // instead of lingering in the open pool. Window-independent — a
        // pre-contract is legal year-round inside the six-month window.
        PerformanceProfiler::stage_labelled(
            "tm_pre_contracts",
            3,
            || country_name.clone(),
            || PreContractManager::stage(country, current_date, &config),
        );
    }

    /// Planning and knowledge, all year.
    ///
    /// Everything from the squad review to the recruitment meeting used to sit
    /// inside the window gate, which is what made the pipeline a procurement
    /// department rather than a market: close the window and the recruitment
    /// department stopped existing, so a club walked into June with no plan,
    /// no watchlist and no dossiers, and spent the window discovering what it
    /// needed.
    ///
    /// These passes carry their own cadences (monthly planning, weekly
    /// scouting and meetings), so running them out here does not multiply the
    /// work by the length of the year — it spreads the same work across it,
    /// which is the point.
    fn run_year_round_passes(
        country: &mut Country,
        current_date: NaiveDate,
        world_pool: &[PlayerSummary],
        foreign_players: &[&PlayerSummary],
        market_map: &MarketMap,
    ) {
        let country_name = country.name.clone();

        // ── Year-round: planning and knowledge ──────────────────────
        //
        // Everything from the squad review to the recruitment meeting used
        // to sit inside the window gate, which is what made the pipeline a
        // procurement department rather than a market: close the window and
        // the recruitment department stopped existing, so a club walked into
        // June with no plan, no watchlist and no dossiers, and spent the
        // window discovering what it needed.
        //
        // These passes carry their own cadences (monthly planning, weekly
        // scouting and meetings), so moving them out here does not multiply
        // the work by the length of the year — it spreads the same work
        // across it, which is the point. The passes that MOVE money or
        // players stay inside the window below.
        PerformanceProfiler::stage_labelled(
            "tm_evaluate_squads",
            3,
            || country_name.clone(),
            || SquadReviewPass::evaluate_squads(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_staff_recommendations",
            3,
            || country_name.clone(),
            || StaffRecommendations::generate_staff_recommendations(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_process_staff_recs",
            3,
            || country_name.clone(),
            || StaffRecommendations::process_staff_recommendations(country, current_date),
        );
        // The club's standing knowledge of the market: names within reach
        // and within the brief's envelope for each shirt it means to fill.
        // Weekly, all year — a scout does not stop watching football in
        // October, and this is what makes the first day of the window start
        // from a written agenda instead of a cold pool.
        // Once a year, pre-season: the recruitment department reviews which
        // markets it wants covered and hires the person who covers one it
        // does not. The only channel by which a corridor the shipped data
        // never named can appear in a save — see [`ScoutMarketDesk`].
        PerformanceProfiler::stage_labelled(
            "tm_scout_market_desk",
            3,
            || country_name.clone(),
            || ScoutMarketDesk::run(country, market_map, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_refresh_watchlists",
            3,
            || country_name.clone(),
            || Watchlist::refresh_watchlists(country, world_pool, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_assign_scouts",
            3,
            || country_name.clone(),
            || ScoutingPass::assign_scouts(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_assign_match_scouts",
            3,
            || country_name.clone(),
            || ScoutingPass::assign_scouts_to_matches(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_match_scouting",
            3,
            || country_name.clone(),
            || ScoutingPass::process_match_scouting(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_process_scouting",
            3,
            || country_name.clone(),
            || ScoutingPass::process_scouting(country, &foreign_players, current_date, market_map),
        );
        PerformanceProfiler::stage_labelled(
            "tm_recruitment_meetings",
            3,
            || country_name.clone(),
            || MeetingPass::run_recruitment_meetings(country, current_date),
        );
    }

    /// The passes that move money or players. Listing, circulation,
    /// shortlisting, board approval, then the three ways a club opens a
    /// conversation: its own approach, the seller's push, and the loan scan.
    fn run_window_passes(
        country: &mut Country,
        current_date: NaiveDate,
        foreign_players: &[&PlayerSummary],
        market_map: &MarketMap,
        mut summary: &mut TransferActivitySummary,
    ) {
        let country_name = country.name.clone();

        debug!("Transfer window is OPEN - simulating pipeline-driven market activity");
        PerformanceProfiler::stage_labelled(
            "tm_list_players",
            3,
            || country_name.clone(),
            || ListingPass::list_players_from_pipeline(country, current_date, &mut summary),
        );
        // Market-circulation / diagnosis: record interest in (or a
        // coherent block reason for) every available signed player,
        // right after the recommendation sweep so this tick's interest
        // is already visible.
        PerformanceProfiler::stage_labelled(
            "tm_circulate_available",
            3,
            || country_name.clone(),
            || MarketCirculation::circulate_available_players(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_build_shortlists",
            3,
            || country_name.clone(),
            || ShortlistPass::build_shortlists(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_board_approvals",
            3,
            || country_name.clone(),
            || ShortlistPass::evaluate_board_approvals(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_initiate_negotiations",
            3,
            || country_name.clone(),
            || ApproachPass::initiate_negotiations(country, current_date),
        );
        // Seller-side push runs BEFORE the borrower scan: a National+ parent
        // evaluates the whole market and places each loan-listed development
        // prospect at the best (highest-level) club where he'd still start,
        // so it gets first crack at sending him UP rather than a constantly-
        // scanning lower club snatching him first. The scan then fills
        // everything the broadcast didn't place — the bulk of loan volume —
        // so prospects are never starved of takers (no scan deferral).
        PerformanceProfiler::stage_labelled(
            "tm_broadcast_loans",
            3,
            || country_name.clone(),
            || LoanPipeline::broadcast_listed_loans(country, current_date),
        );
        // Stale permanent listings get the same push, permanent
        // flavor: a player unsold past the grace weeks asks the club
        // to find him a move and the scouts offer him around, the
        // tier reach widening cumulatively downward until a buyer
        // responds. Paired with the year-unsold free-exit valve
        // below, no listing lingers for seasons.
        PerformanceProfiler::stage_labelled(
            "tm_broadcast_transfers",
            3,
            || country_name.clone(),
            || LoanPipeline::broadcast_listed_transfers(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_scan_loan_market",
            3,
            || country_name.clone(),
            || LoanPipeline::scan_loan_market(country, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_scan_foreign_loans",
            3,
            || country_name.clone(),
            || {
                LoanPipeline::scan_foreign_loan_market(
                    country,
                    &foreign_players,
                    current_date,
                    market_map,
                )
            },
        );
    }

    /// The passes that run whatever the window says, after it: the stranded-
    /// listing escape valve, the shadow reports, the breakout watch, and the
    /// `Wnt` reconciliation.
    fn run_year_round_tail(
        country: &mut Country,
        current_date: NaiveDate,
        foreign_players: &[&PlayerSummary],
    ) {
        let country_name = country.name.clone();

        // Escape valve for stranded listings: a player still unsold a full
        // year after being transfer-listed forces a mutual termination and
        // leaves on a free. Window-independent — tearing up a contract is
        // legal year-round; the free-agent sweep collects him next tick.
        PerformanceProfiler::stage_labelled(
            "tm_release_unsold",
            3,
            || country_name.clone(),
            || ListingPass::release_unsold_listed_players(country, current_date),
        );

        PerformanceProfiler::stage_labelled(
            "tm_shadow_reports",
            3,
            || country_name.clone(),
            || ScoutingPass::refresh_shadow_reports(country, current_date),
        );
        // Year-round breakout watch: discover high-form players on plausible
        // buyers' books even with the window shut. Runs outside the window
        // block (weekly cadence enforced inside) and only records scout
        // monitoring — never a negotiation.
        PerformanceProfiler::stage_labelled(
            "tm_breakout_form",
            3,
            || country_name.clone(),
            || FormWatch::scan_breakout_form(country, &foreign_players, current_date),
        );
        PerformanceProfiler::stage_labelled(
            "tm_sync_wanted",
            3,
            || country_name.clone(),
            || ApproachPass::sync_wanted_status(country),
        );
    }
}

#[cfg(test)]
mod side_channel_tests {
    //! The offered / rejected side channels are collected as plain vecs
    //! across the emergency pass, the staged-negotiation flow, and the
    //! resolver — the same player can legitimately appear several times
    //! in one tick (retry at a second club, offer + rejection chain).
    //! These per-country vecs are aggregated world-wide into a
    //! `FreeAgentBumpBatch` and applied in ONE pass over the pool by
    //! `ApproachPass::apply_free_agent_market_bumps_batch`, which
    //! must collapse them to one market-state bump per player per tick.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
    use crate::transfers::pool::FreeAgentBumpBatch;
    use crate::{
        Country, PersonAttributes, Player, PlayerAttributes, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    struct SideChannelFixtures;

    impl SideChannelFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn pool_player(id: u32, date: NaiveDate) -> Player {
            let mut player = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Pool".to_string(), format!("P{id}")))
                .birth_date(Self::d(1996, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 16,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            player.ensure_free_agent_state(date, 4000);
            player
        }

        fn simulator(date: NaiveDate, free_agents: Vec<Player>) -> SimulatorData {
            let country = Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(5000)
                .leagues(LeagueCollection::new(vec![League::new(
                    1,
                    "L".to_string(),
                    "english".to_string(),
                    1,
                    5000,
                    LeagueSettings {
                        season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                        season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                        tier: 1,
                        promotion_spots: 0,
                        relegation_spots: 0,
                        league_group: None,
                        split_season: false,
                    },
                    false,
                )]))
                .clubs(Vec::new())
                .build()
                .unwrap();
            let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
            let mut data = SimulatorData::new(
                date.and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            );
            data.free_agents = free_agents;
            data
        }
    }

    #[test]
    fn duplicate_same_day_offer_and_rejection_ids_bump_market_state_once() {
        let date = SideChannelFixtures::d(2026, 6, 10);
        let player = SideChannelFixtures::pool_player(900, date);
        let mut data = SideChannelFixtures::simulator(date, vec![player]);

        // Same-day duplicates — whether from one country's retries or from
        // several countries pursuing the same pool player this tick — are
        // concatenated into the world-wide batch and must still count once
        // after the single dedup'd pool pass.
        let batch = FreeAgentBumpBatch {
            offered_ids: vec![900, 900, 900],
            rejected_ids: vec![900, 900],
            block_reasons: Vec::new(),
        };
        ApproachPass::apply_free_agent_market_bumps_batch(&mut data, &batch, date);

        let state = data.free_agents[0]
            .free_agent_state()
            .expect("pool player keeps market state when no signing executed");
        assert_eq!(
            state.offers_received_30d(date),
            1,
            "repeated same-day attempts must count as one received offer"
        );
        assert_eq!(
            state.offers_rejected_total, 1,
            "repeated same-day rejections must count once"
        );
    }
}

#[cfg(test)]
mod pending_signal_delivery_tests {
    //! Phase-C delivery of cross-border saga beats: the buying country's
    //! parallel pass queued the signal; the serial drain must find the
    //! player in the SELLING country and hand him the structured event.

    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, HappinessEventType,
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, Team, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule, TransferInterestSource, TransferInterestStage,
    };
    use crate::{PlayerClubContract, PlayerSquadStatus};
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn seller_player(id: u32) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = 120;
        attrs.current_reputation = 2000;
        let mut contract = PlayerClubContract::new(50_000, d(2029, 6, 30));
        contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Foreign".to_string(), format!("P{id}")))
            .birth_date(d(2000, 1, 1))
            .country_id(2)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::flat_for_ability(120))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .contract(Some(contract))
            .build()
            .unwrap()
    }

    fn seller_country(players: Vec<Player>) -> Country {
        let team = Team::builder()
            .id(10)
            .league_id(Some(1))
            .club_id(1)
            .name("Seller".to_string())
            .slug("seller".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(5000, 5000, 5000))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap();
        let club = Club::new(
            1,
            "Seller".to_string(),
            Location::new(2),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            ClubFacilities::default(),
        );
        Country::builder()
            .id(2)
            .code("it".to_string())
            .slug("italy".to_string())
            .name("Italy".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![League::new(
                1,
                "Serie A".to_string(),
                "serie-a".to_string(),
                2,
                7000,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            )]))
            .clubs(vec![club])
            .build()
            .unwrap()
    }

    #[test]
    fn cross_border_collapse_reaches_the_foreign_player() {
        let country = seller_country(vec![seller_player(500)]);
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        let mut data = SimulatorData::new(
            d(2026, 7, 10).and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        );

        let signal = PendingPlayerSignal {
            player_id: 500,
            selling_country_id: 2,
            selling_club_id: 1,
            stage: TransferInterestStage::MoveCollapsed,
            source: TransferInterestSource::ConfirmedApproach,
            repeated_attention: false,
            interested_club_id: 77,
            interested_league_id: Some(9),
            buyer_rep: 0.9,
            seller_rep: 0.5,
            buyer_league_rep: 8000,
            buyer_country_id: 3,
            buyer_continent_id: 1,
            buyer_has_continental_path: false,
            buyer_competition_path: None,
        };

        TransferTick::deliver_pending_player_signals(&mut data, &[signal]);

        let player = data
            .country(2)
            .and_then(|c| c.clubs[0].teams.teams[0].players.find(500))
            .expect("player still at the seller");
        assert!(
            player
                .happiness
                .recent_events
                .iter()
                .any(|e| e.event_type == HappinessEventType::DreamMoveCollapsed),
            "the cross-border collapse must reach the foreign player's feed"
        );
    }
}
