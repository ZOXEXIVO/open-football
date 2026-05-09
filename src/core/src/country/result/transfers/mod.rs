pub(crate) mod config;
pub(crate) mod execution;
pub(crate) mod free_agent_market_calc;
mod free_agents;
mod listings;
mod negotiations;
pub(crate) mod types;

use super::CountryResult;
use crate::simulator::SimulatorData;
use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use chrono::NaiveDate;
use config::TransferConfig;
use free_agents::{GlobalFreeAgentSigning, execute_global_free_agent_signing};
pub(crate) use free_agents::{GlobalFreeAgentSummary, snapshot_global_free_agents};
use log::debug;
use types::TransferActivitySummary;

impl CountryResult {
    pub(super) fn simulate_transfer_market(
        data: &mut SimulatorData,
        country_id: u32,
        current_date: NaiveDate,
    ) -> TransferActivitySummary {
        let mut summary = TransferActivitySummary::new();

        let window_manager = TransferWindowManager::new();
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
        let deferred_transfers = if let Some(country) = data.country_mut(country_id) {
            // Sync market's window flag. On open→closed transitions this cancels
            // any stranded listings and expires pending negotiations.
            country.transfer_market.check_transfer_window(window_open);

            // Resolve pending negotiations — returns all completed transfers for deferred execution
            let deferred = Self::resolve_pending_negotiations(country, current_date, &mut summary);

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

            // Free agents and contract expirations. Returns deferred
            // signings sourced from the global pool (`sim.free_agents`),
            // which we execute after the country borrow ends.
            global_signings = Self::handle_free_agents(
                country,
                current_date,
                &mut summary,
                &global_free_agents,
                &config,
                &mut domestic_signed_ids,
                &mut global_offered_ids,
                &mut global_rejected_ids,
            );

            if window_open {
                debug!("Transfer window is OPEN - simulating pipeline-driven market activity");

                Self::list_players_from_pipeline(country, current_date, &mut summary);
                PipelineProcessor::evaluate_squads(country, current_date);
                PipelineProcessor::generate_staff_recommendations(country, current_date);
                PipelineProcessor::process_staff_recommendations(country, current_date);
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
        if !global_offered_ids.is_empty() || !global_rejected_ids.is_empty() {
            for player in data.free_agents.iter_mut() {
                if global_offered_ids.contains(&player.id) {
                    player.on_offer_received(current_date);
                }
                if global_rejected_ids.contains(&player.id) {
                    player.on_offer_rejected();
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
