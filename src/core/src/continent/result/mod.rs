mod types;
mod rankings;
mod competitions;
mod economics;
mod regulations;
mod awards;

pub use types::*;

use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::SimulationResult;

pub struct ContinentResult {
    pub continent_id: u32,
    pub countries: Vec<CountryResult>,

    // New fields for continental-level results
    pub competition_results: Option<ContinentalCompetitionResults>,
    pub rankings_update: Option<ContinentalRankingsUpdate>,
    pub transfer_summary: Option<CrossBorderTransferSummary>,
    pub economic_impact: Option<EconomicZoneImpact>,

    /// Match results from national team competitions (for global match_store)
    pub national_match_results: Vec<crate::r#match::MatchResult>,
}

impl ContinentResult {
    pub fn new(continent_id: u32, countries: Vec<CountryResult>, national_match_results: Vec<crate::r#match::MatchResult>) -> Self {
        ContinentResult {
            continent_id,
            countries,
            competition_results: None,
            rankings_update: None,
            transfer_summary: None,
            economic_impact: None,
            national_match_results,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        let current_date = data.date.date();

        for match_result in &self.national_match_results {
            data.match_store.push(match_result.clone(), current_date);
        }

        // Phase 2: Update Continental Rankings (monthly)
        if DateUtils::is_month_beginning(current_date) {
            self.update_continental_rankings(data, result);
        }

        // Phase 3: Continental Competition Processing
        if self.is_competition_draw_period(current_date) {
            self.conduct_competition_draws(data, current_date);
        }

        let competition_results = self.simulate_continental_competitions(data, current_date);
        if let Some(comp_results) = competition_results {
            self.process_competition_results(comp_results, data, result);
        }

        // Phase 4: Continental Economic Updates (quarterly)
        if DateUtils::is_quarter_start(current_date) {
            self.update_economic_zone(data, &self.countries);
        }

        // Phase 5: Continental Regulatory Updates (yearly)
        if DateUtils::is_year_start(current_date) {
            self.update_continental_regulations(data, current_date);
        }

        // Phase 6: Continental Awards & Recognition (yearly)
        if DateUtils::is_year_end(current_date) {
            self.process_continental_awards(data, &self.countries);
        }

        for country_result in self.countries {
            country_result.process(data, result);
        }
    }

    pub(crate) fn get_continent_id(&self) -> u32 {
        self.continent_id
    }
}

// Extension to SimulationResult to include continental matches
impl SimulationResult {
    // Note: This would need to be added to the actual SimulationResult struct
    // pub continental_matches: Vec<ContinentalMatchResult>,
}
