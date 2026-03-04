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

        // Check if transfer window is open
        let window_manager = TransferWindowManager::new();
        if !window_manager.is_window_open(country_id, current_date) {
            return summary;
        }

        debug!("Transfer window is OPEN - simulating pipeline-driven market activity");

        if let Some(country) = data.country_mut(country_id) {
            // Step 1: Resolve pending negotiations from previous days
            Self::resolve_pending_negotiations(country, current_date, &mut summary);

            // Step 2: List players for transfer (must run before shortlists so market has candidates)
            Self::list_players_from_pipeline(country, current_date, &mut summary);

            // Step 3: Evaluate squads (periodic - not daily)
            PipelineProcessor::evaluate_squads(country, current_date);

            // Step 3.5: Staff proactively recommend players (weekly)
            PipelineProcessor::generate_staff_recommendations(country, current_date);

            // Step 3.75: Process staff recommendations into pipeline actions (weekly)
            PipelineProcessor::process_staff_recommendations(country, current_date);

            // Step 4: Assign scouts to pending requests
            PipelineProcessor::assign_scouts(country, current_date);

            // Step 4.5: Assign scouts to youth/reserve team matches
            PipelineProcessor::assign_scouts_to_matches(country, current_date);

            // Step 4.75: Process match-day scouting observations
            PipelineProcessor::process_match_scouting(country, current_date);

            // Step 5: Process scouting observations
            PipelineProcessor::process_scouting(country, current_date);

            // Step 6: Build shortlists from scouting + market listings
            PipelineProcessor::build_shortlists(country, current_date);

            // Step 7: Initiate negotiations from shortlists
            PipelineProcessor::initiate_negotiations(country, current_date);

            // Step 7.5: Small clubs proactively scan the loan market
            PipelineProcessor::scan_loan_market(country, current_date);

            // Step 8: Free agents and contract expirations
            Self::handle_free_agents(country, current_date, &mut summary);

            // Step 9: Expire stale negotiations
            country.transfer_market.update(current_date);

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );
        }

        summary
    }
}
