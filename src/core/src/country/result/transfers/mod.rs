pub(crate) mod types;
mod listings;
mod negotiations;
pub(crate) mod execution;
mod free_agents;

use chrono::NaiveDate;
use log::debug;
use types::TransferActivitySummary;
use super::CountryResult;
use crate::simulator::SimulatorData;
use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline_processor::PipelineProcessor;

impl CountryResult {
    pub(super) fn simulate_transfer_market(
        data: &mut SimulatorData,
        country_id: u32,
        current_date: NaiveDate,
    ) -> TransferActivitySummary {
        let mut summary = TransferActivitySummary::new();

        let window_manager = TransferWindowManager::new();
        let window_open = window_manager.is_window_open(country_id, current_date);

        // Collect foreign player pool from other countries (for cross-country scouting)
        let foreign_players = if window_open {
            data.continents.iter()
                .flat_map(|cont| &cont.countries)
                .filter(|c| c.id != country_id)
                .flat_map(|c| PipelineProcessor::collect_player_pool(c, current_date))
                .collect()
        } else {
            Vec::new()
        };

        // Phase 1: Negotiations & pipeline (per-country)
        let deferred_transfers = if let Some(country) = data.country_mut(country_id) {
            // Resolve pending negotiations — returns all completed transfers for deferred execution
            let deferred = Self::resolve_pending_negotiations(country, current_date, &mut summary);

            // Expire stale negotiations
            let expired = country.transfer_market.update(current_date);
            for (buying_club_id, player_id) in expired {
                PipelineProcessor::on_negotiation_resolved(country, buying_club_id, player_id, false);
            }

            // Free agents and contract expirations
            Self::handle_free_agents(country, current_date, &mut summary);

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
                PipelineProcessor::build_shortlists(country, current_date);

                // Domestic negotiations (local players have priority)
                PipelineProcessor::initiate_negotiations(country, current_date);

                PipelineProcessor::scan_loan_market(country, current_date);
            }

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );

            deferred
        } else {
            Vec::new()
        };

        // Phase 2: Execute all completed transfers (domestic + foreign) via unified path
        for transfer in deferred_transfers {
            execution::execute_transfer(
                data,
                transfer.player_id,
                transfer.selling_country_id,
                transfer.selling_club_id,
                transfer.buying_country_id,
                transfer.buying_club_id,
                transfer.fee,
                transfer.is_loan,
                current_date,
            );
        }

        // Phase 3: Foreign negotiation initiation (after domestic, so local has priority)
        if window_open {
            PipelineProcessor::initiate_foreign_negotiations(data, country_id, current_date);
        }

        summary
    }
}
