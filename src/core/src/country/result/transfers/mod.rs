pub(crate) mod config;
pub(crate) mod execution;
pub(crate) mod free_agent_audit;
mod free_agent_depth;
pub(crate) mod free_agent_market_calc;
mod free_agents;
mod listings;
mod negotiations;
mod pre_contract;
mod settlement;
pub(crate) mod types;

use super::CountryResult;
use crate::Country;
use crate::club::player::transfer::FreeAgentBlockReason;
use crate::simulator::SimulatorData;
use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use chrono::NaiveDate;
use config::TransferConfig;
use free_agents::{GlobalFreeAgentSigning, execute_global_free_agent_signing};
pub(crate) use free_agents::{GlobalFreeAgentSummary, snapshot_global_free_agents};
use log::debug;
use pre_contract::PreContractManager;
use settlement::TransferClauseSettler;
use std::collections::{HashMap, HashSet};
use types::DeferredTransfer;
use types::TransferActivitySummary;

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
        }
    }
}

/// World-wide aggregate of free-agent market bumps, collected from every
/// country's `DeferredTransferOps` before the serial Phase-C drain.
///
/// Each per-country `apply_deferred_transfer_ops` used to walk the ENTIRE
/// `data.free_agents` pool twice (offer/reject bump + block-reason stamp),
/// once for every country — `O(countries × pool)`. The orchestrator now
/// concatenates every country's bump ids into one of these and runs a
/// SINGLE pass over the pool via
/// [`PipelineProcessor::apply_free_agent_market_bumps_batch`], matching the
/// documented "one bump per player per tick" intent across the whole world
/// (the previous per-country dedup still allowed a player pursued by two
/// countries to be bumped twice).
#[derive(Default)]
pub struct FreeAgentBumpBatch {
    /// Pool players that fielded an offer this tick (any country).
    pub offered_ids: Vec<u32>,
    /// Pool players whose acceptance roll failed this tick (any country).
    pub rejected_ids: Vec<u32>,
    /// Per-player skip reasons from every country's matcher; merged to the
    /// highest-ranked reason per player before stamping.
    pub block_reasons: Vec<(u32, FreeAgentBlockReason)>,
}

impl FreeAgentBumpBatch {
    /// True when no country recorded any free-agent market activity this
    /// tick — lets the orchestrator skip the pool pass entirely.
    pub fn is_empty(&self) -> bool {
        self.offered_ids.is_empty()
            && self.rejected_ids.is_empty()
            && self.block_reasons.is_empty()
    }
}

impl CountryResult {
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
    ) -> DeferredTransferOps {
        let country_id = country.id;
        let mut summary = TransferActivitySummary::new();
        let window_manager = TransferWindowManager::for_country(country, current_date);
        let window_open = window_manager.is_window_open(country_id, current_date);
        let config = TransferConfig::default();

        // Filter foreign players from the pre-built world snapshot.
        let foreign_players: Vec<PlayerSummary> = if window_open {
            world_pool
                .iter()
                .filter(|s| s.country_id != country_id)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let completed_before = summary.completed_transfers;
        let mut ops = DeferredTransferOps::empty(country_id);
        ops.window_open = window_open;
        ops.completed_before = completed_before;

        // Sync market's window flag. On open→closed transitions this cancels
        // any stranded listings and expires pending negotiations.
        country.transfer_market.check_transfer_window(window_open);

        // Resolve pending negotiations — club-to-club moves for the
        // deferred execution queue, plus free-agent negotiation
        // outcomes: pool signings whose medical just cleared (executed
        // against `data.free_agents` in Phase C) and rejected-offer
        // counters for pool players who declined personal terms.
        let outcomes = Self::resolve_pending_negotiations(country, current_date, &mut summary);
        ops.deferred_transfers = outcomes.deferred;
        ops.global_signings = outcomes.free_agent_signings;
        ops.global_rejected_ids = outcomes.free_agent_rejected_ids;

        // Expire stale negotiations
        let expired = country.transfer_market.update(current_date);
        for (buying_club_id, player_id) in expired {
            PipelineProcessor::on_negotiation_resolved(country, buying_club_id, player_id, false);
        }

        // Settle any installment tranches that came due today and any
        // performance / promotion add-ons whose triggers have just
        // fired. The settler routes cash buyer → seller (or buyer →
        // beneficiary for sell-on) so the deal's deferred cost
        // actually lands on the books over time.
        TransferClauseSettler::settle_due(country, current_date);

        // Free agents and contract expirations. Returns deferred
        // signings sourced from the global pool (`data.free_agents`),
        // which we execute after the country borrow ends — appended to
        // the negotiation-driven pool signings collected above.
        let pool_signings = Self::handle_free_agents(
            country,
            current_date,
            &mut summary,
            global_free_agents,
            &config,
            &mut ops.domestic_signed_ids,
            &mut ops.global_offered_ids,
            &mut ops.global_rejected_ids,
            &mut ops.global_block_reasons,
        );
        ops.global_signings.extend(pool_signings);

        // Pre-contracts (Bosman): stage future free transfers for useful
        // players in the final months of an expiring deal their club won't
        // renew, so they move directly to a domestic rival on expiry
        // instead of lingering in the open pool. Window-independent — a
        // pre-contract is legal year-round inside the six-month window.
        PreContractManager::stage(country, current_date, &config);

        if window_open {
            debug!("Transfer window is OPEN - simulating pipeline-driven market activity");
            Self::list_players_from_pipeline(country, current_date, &mut summary);
            PipelineProcessor::evaluate_squads(country, current_date);
            PipelineProcessor::generate_staff_recommendations(country, current_date);
            PipelineProcessor::process_staff_recommendations(country, current_date);
            // Market-circulation / diagnosis: record interest in (or a
            // coherent block reason for) every available signed player,
            // right after the recommendation sweep so this tick's interest
            // is already visible.
            PipelineProcessor::circulate_available_players(country, current_date);
            PipelineProcessor::assign_scouts(country, current_date);
            PipelineProcessor::assign_scouts_to_matches(country, current_date);
            PipelineProcessor::process_match_scouting(country, current_date);
            PipelineProcessor::process_scouting(country, &foreign_players, current_date);
            PipelineProcessor::run_recruitment_meetings(country, current_date);
            PipelineProcessor::build_shortlists(country, current_date);
            PipelineProcessor::evaluate_board_approvals(country, current_date);
            PipelineProcessor::initiate_negotiations(country, current_date);
            PipelineProcessor::scan_loan_market(country, current_date);
            // Seller-side push: National+ clubs broadcast their loan-listed
            // players to scouts tier-by-tier (high → low) instead of only
            // waiting to be scanned.
            PipelineProcessor::broadcast_listed_loans(country, current_date);
            PipelineProcessor::scan_foreign_loan_market(country, &foreign_players, current_date);
        }

        PipelineProcessor::refresh_shadow_reports(country, current_date);
        // Year-round breakout watch: discover high-form players on plausible
        // buyers' books even with the window shut. Runs outside the window
        // block (weekly cadence enforced inside) and only records scout
        // monitoring — never a negotiation.
        PipelineProcessor::scan_breakout_form(country, current_date);
        PipelineProcessor::sync_wanted_status(country);

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
        // `PipelineProcessor::apply_free_agent_market_bumps_batch`,
        // collapsing the old O(countries × pool) double-walk.

        // Execute global free-agent signings (Move-on-Free players from
        // `data.free_agents`).
        let mut completed = ops.completed_after;
        for signing in &ops.global_signings {
            if execute_global_free_agent_signing(data, signing, current_date, &config) {
                completed += 1;
            }
        }

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
        for transfer in &ops.deferred_transfers {
            let success = execution::execute_transfer(data, transfer, current_date);
            if success {
                data.dirty_player_index = true;
            }
            if !success {
                if let Some(country) = data.country_mut(transfer.buying_country_id) {
                    country.transfer_market.transfer_history.retain(|t| {
                        !(t.player_id == transfer.player_id
                            && t.to_club_id == transfer.buying_club_id
                            && t.transfer_date == current_date)
                    });
                }
            }
        }

        // Phase 3: Foreign negotiation initiation (domestic priority).
        if ops.window_open {
            PipelineProcessor::initiate_foreign_negotiations(data, ops.country_id, current_date);
        }
    }

    /// Legacy monolithic path — kept only for tests / external
    /// callers that don't go through the parallel Phase-A split.
    /// Production callers should use `simulate_transfer_market_local`
    /// + `apply_deferred_transfer_ops`.
    #[allow(dead_code)]
    pub(super) fn simulate_transfer_market(
        data: &mut SimulatorData,
        country_id: u32,
        current_date: NaiveDate,
    ) -> TransferActivitySummary {
        let mut summary = TransferActivitySummary::new();

        let window_manager = data
            .country(country_id)
            .map(|c| TransferWindowManager::for_country(c, current_date))
            .unwrap_or_else(TransferWindowManager::new);
        let window_open = window_manager.is_window_open(country_id, current_date);
        // Single source of truth for tunable knobs (probability tiers,
        // squad caps, default contract terms). One day we'll thread a
        // per-save override through here; for now `default()` keeps the
        // game's published balance.
        let config = TransferConfig::default();

        // Collect foreign player pool from other countries (for cross-country scouting).
        //
        // Phase C in `simulator.rs` builds a world-wide pool once per
        // tick and stows it on `data.daily_world_player_pool` so each
        // country can borrow from the cached snapshot instead of
        // re-walking every other country's players. Filtering out
        // own-country entries here is O(N) over the cache. Falls back
        // to a per-country rebuild when the cache is absent (test
        // harnesses, future callers outside Phase C).
        let foreign_players: Vec<PlayerSummary> = if window_open {
            if let Some(world_pool) = data.daily_world_player_pool.as_ref() {
                world_pool
                    .iter()
                    .filter(|s| s.country_id != country_id)
                    .cloned()
                    .collect()
            } else {
                data.continents
                    .iter()
                    .flat_map(|cont| &cont.countries)
                    .filter(|c| c.id != country_id)
                    .flat_map(|c| PipelineProcessor::collect_player_pool(c, current_date))
                    .collect()
            }
        } else {
            Vec::new()
        };

        // Snapshot the global "Move on Free" pool. Phase C in
        // `simulator.rs` builds this snapshot once per tick and stows
        // it on `data.daily_global_free_agents`; per-country callers
        // share the same view (the matching loop is read-only).
        // Falls back to a per-country rebuild when the cache is
        // absent (test paths and one-off callers).
        let global_free_agents: Vec<GlobalFreeAgentSummary> =
            if let Some(cached) = data.daily_global_free_agents.as_ref() {
                cached.clone()
            } else {
                snapshot_global_free_agents(data, current_date)
            };

        // Snapshot completed count so we can detect any free-agent / negotiation
        // signings that bypass the deferred execution path below.
        let completed_before = summary.completed_transfers;

        // Phase 1: Negotiations & pipeline (per-country)
        let mut global_signings: Vec<GlobalFreeAgentSigning> = Vec::new();
        let mut domestic_signed_ids: Vec<u32> = Vec::new();
        // Side-channel for free-agent market state: each global-pool
        // candidate that fielded an offer today, and the subset whose
        // acceptance roll failed. Applied to the player's
        // `FreeAgentMarketState` after the country borrow ends.
        let mut global_offered_ids: Vec<u32> = Vec::new();
        let mut global_rejected_ids: Vec<u32> = Vec::new();
        let mut global_block_reasons: Vec<(u32, FreeAgentBlockReason)> = Vec::new();
        let deferred_transfers = if let Some(country) = data.country_mut(country_id) {
            // Sync market's window flag. On open→closed transitions this cancels
            // any stranded listings and expires pending negotiations.
            country.transfer_market.check_transfer_window(window_open);

            // Resolve pending negotiations — returns all completed transfers for deferred execution
            let outcomes = Self::resolve_pending_negotiations(country, current_date, &mut summary);
            global_signings.extend(outcomes.free_agent_signings);
            global_rejected_ids.extend(outcomes.free_agent_rejected_ids);
            let deferred = outcomes.deferred;

            // Expire stale negotiations
            let expired = country.transfer_market.update(current_date);
            for (buying_club_id, player_id) in expired {
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    buying_club_id,
                    player_id,
                    false,
                );
            }

            // Daily clause settlement — installment tranches that
            // came due today, performance/promotion add-ons whose
            // triggers have crossed. Keeps the legacy path aligned
            // with the parallel Phase-A path above.
            TransferClauseSettler::settle_due(country, current_date);

            // Free agents and contract expirations. Returns deferred
            // signings sourced from the global pool (`sim.free_agents`),
            // which we execute after the country borrow ends.
            global_signings.extend(Self::handle_free_agents(
                country,
                current_date,
                &mut summary,
                &global_free_agents,
                &config,
                &mut domestic_signed_ids,
                &mut global_offered_ids,
                &mut global_rejected_ids,
                &mut global_block_reasons,
            ));

            if window_open {
                debug!("Transfer window is OPEN - simulating pipeline-driven market activity");

                Self::list_players_from_pipeline(country, current_date, &mut summary);
                PipelineProcessor::evaluate_squads(country, current_date);
                PipelineProcessor::generate_staff_recommendations(country, current_date);
                PipelineProcessor::process_staff_recommendations(country, current_date);
                // Market-circulation / diagnosis pass — see the local
                // pipeline path above for rationale.
                PipelineProcessor::circulate_available_players(country, current_date);
                PipelineProcessor::assign_scouts(country, current_date);
                PipelineProcessor::assign_scouts_to_matches(country, current_date);
                PipelineProcessor::process_match_scouting(country, current_date);
                PipelineProcessor::process_scouting(country, &foreign_players, current_date);
                // Weekly recruitment meeting — scouts vote, chief
                // scout / DoF / manager weigh in, decisions are stamped
                // onto monitoring rows + shortlist. Runs only on Mondays
                // inside the function so daily ticks pay only the
                // weekday check.
                PipelineProcessor::run_recruitment_meetings(country, current_date);
                PipelineProcessor::build_shortlists(country, current_date);
                // Board review: veto named targets that blow past the
                // approved budget or clash with the chairman's financial
                // stance. Coach-driven requests the board rejects are
                // tracked via `board_approved = Some(false)` + Abandoned.
                PipelineProcessor::evaluate_board_approvals(country, current_date);

                // Domestic negotiations (local players have priority)
                PipelineProcessor::initiate_negotiations(country, current_date);

                PipelineProcessor::scan_loan_market(country, current_date);

                // Cross-country loan scanning: clubs can find loan targets abroad
                PipelineProcessor::scan_foreign_loan_market(
                    country,
                    &foreign_players,
                    current_date,
                );
            }

            // Year-round shadow-squad maintenance: runs on every tick (weekly
            // cadence enforced inside the function). Keeps tracked players
            // fresh between windows so the next window opens with current data.
            PipelineProcessor::refresh_shadow_reports(country, current_date);

            // Year-round breakout watch — records scout monitoring on high-form
            // players (weekly cadence enforced inside; never a negotiation).
            PipelineProcessor::scan_breakout_form(country, current_date);

            // Prune stale `Wnt` statuses whose originating interest has
            // since been cleared (window reset, transfer completion, or
            // shortlist exhaustion). Without this the flag latches forever
            // and the transfer page reports "Wanted" with no interested clubs.
            PipelineProcessor::sync_wanted_status(country);

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );

            deferred
        } else {
            Vec::new()
        };

        // Cross-country interest sweep for the in-country free-agent
        // signings that just executed: their per-country cleanup ran
        // inside `handle_free_agents`, but clubs in OTHER countries may
        // still have the player on a shortlist or in scout monitoring.
        for signed_id in &domestic_signed_ids {
            PipelineProcessor::cleanup_player_transfer_interest(data, *signed_id);
        }

        // Apply free-agent market state outside the country borrow. Each
        // global-pool offer made today bumps the 30-day window counter;
        // declined offers also bump the rejected-total. Done before the
        // signing executor because signing clears the player's state
        // anyway, so updating both for a successful candidate is harmless
        // but updating after would race against `clear_free_agent_state`.
        // De-duplicated to one bump per player per tick, mirroring the
        // Phase-C path above.
        if !global_offered_ids.is_empty() || !global_rejected_ids.is_empty() {
            let offered: HashSet<u32> = global_offered_ids.iter().copied().collect();
            let rejected: HashSet<u32> = global_rejected_ids.iter().copied().collect();
            for player in data.free_agents.iter_mut() {
                if offered.contains(&player.id) {
                    player.on_offer_received(current_date);
                }
                if rejected.contains(&player.id) {
                    player.on_offer_rejected();
                }
            }
        }

        // Stamp the tick's block reasons, mirroring the Phase-C path.
        if !global_block_reasons.is_empty() {
            let mut merged: HashMap<u32, FreeAgentBlockReason> = HashMap::new();
            for (player_id, reason) in &global_block_reasons {
                merged
                    .entry(*player_id)
                    .and_modify(|existing| {
                        if reason.rank() > existing.rank() {
                            *existing = *reason;
                        }
                    })
                    .or_insert(*reason);
            }
            for player in data.free_agents.iter_mut() {
                if let Some(reason) = merged.get(&player.id) {
                    player.on_market_blocked(current_date, *reason);
                }
            }
        }

        // Execute any deferred global free-agent signings (players from
        // `data.free_agents`, populated by the "Move on Free" UI action).
        // Each signing is independent and may fail silently if another
        // country signed the same player earlier in this tick — we deduce
        // success from the executor's return value.
        for signing in &global_signings {
            if execute_global_free_agent_signing(data, signing, current_date, &config) {
                summary.completed_transfers += 1;
            }
        }

        // Free-agent / in-country signings already mutated club rosters
        // while the country borrow was active — flag the index as dirty.
        if summary.completed_transfers > completed_before {
            data.dirty_player_index = true;
        }

        // Phase 2: Execute all completed transfers (domestic + foreign) via unified path
        for transfer in &deferred_transfers {
            let success = execution::execute_transfer(data, transfer, current_date);

            if success {
                data.dirty_player_index = true;
            }

            // Remove phantom transfer record if execution failed
            // (e.g. player already moved via another negotiation)
            if !success {
                if let Some(country) = data.country_mut(transfer.buying_country_id) {
                    country.transfer_market.transfer_history.retain(|t| {
                        !(t.player_id == transfer.player_id
                            && t.to_club_id == transfer.buying_club_id
                            && t.transfer_date == current_date)
                    });
                }
            }
        }

        // Phase 3: Foreign negotiation initiation (after domestic, so local has priority)
        if window_open {
            PipelineProcessor::initiate_foreign_negotiations(data, country_id, current_date);
        }

        summary
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
    //! `PipelineProcessor::apply_free_agent_market_bumps_batch`, which
    //! must collapse them to one market-state bump per player per tick.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
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
        PipelineProcessor::apply_free_agent_market_bumps_batch(&mut data, &batch, date);

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
