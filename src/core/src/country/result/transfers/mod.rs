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

        if let Some(country) = data.country_mut(country_id) {
            // Always resolve pending negotiations and expire stale ones,
            // even outside the window — negotiations started during the window
            // must be able to complete or be properly cleaned up.
            Self::resolve_pending_negotiations(country, current_date, &mut summary);

            // Expire stale negotiations and notify the pipeline so
            // active_negotiation_count stays accurate and shortlists advance.
            let expired = country.transfer_market.update(current_date);
            for (buying_club_id, player_id) in expired {
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    buying_club_id,
                    player_id,
                    false,
                );
            }

            // Free agents and contract expirations run regardless of window
            Self::handle_free_agents(country, current_date, &mut summary);

            // The rest of the pipeline (listings, scouting, negotiations) only
            // runs when the transfer window is open.
            if window_open {
                debug!("Transfer window is OPEN - simulating pipeline-driven market activity");

                // List players for transfer (must run before shortlists so market has candidates)
                Self::list_players_from_pipeline(country, current_date, &mut summary);

                // Evaluate squads (periodic - not daily)
                PipelineProcessor::evaluate_squads(country, current_date);

                // Staff proactively recommend players (weekly)
                PipelineProcessor::generate_staff_recommendations(country, current_date);

                // Process staff recommendations into pipeline actions (weekly)
                PipelineProcessor::process_staff_recommendations(country, current_date);

                // Assign scouts to pending requests
                PipelineProcessor::assign_scouts(country, current_date);

                // Assign scouts to youth/reserve team matches
                PipelineProcessor::assign_scouts_to_matches(country, current_date);

                // Process match-day scouting observations
                PipelineProcessor::process_match_scouting(country, current_date);

                // Process scouting observations
                PipelineProcessor::process_scouting(country, current_date);

                // Build shortlists from scouting + market listings
                PipelineProcessor::build_shortlists(country, current_date);

                // Initiate negotiations from shortlists
                PipelineProcessor::initiate_negotiations(country, current_date);

                // Small clubs proactively scan the loan market
                PipelineProcessor::scan_loan_market(country, current_date);
            }

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );
        }

        summary
    }
}
