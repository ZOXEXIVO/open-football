mod awards;
mod competitions;
mod economics;
mod rankings;
mod regulations;
mod types;

pub(crate) use awards::ContinentAwardOutcome;
pub use types::*;

use crate::SimulationResult;
use crate::country::CountryResult;
use crate::r#match::MatchResult;
use crate::simulator::SimulatorData;

pub struct ContinentResult {
    pub continent_id: u32,
    pub countries: Vec<CountryResult>,

    // New fields for continental-level results
    pub competition_results: Option<ContinentalCompetitionResults>,
    pub rankings_update: Option<ContinentalRankingsUpdate>,
    pub transfer_summary: Option<CrossBorderTransferSummary>,
    pub economic_impact: Option<EconomicZoneImpact>,

    /// Match results from national team competitions (for global match_store)
    pub national_match_results: Vec<MatchResult>,
}

impl ContinentResult {
    pub fn new(
        continent_id: u32,
        countries: Vec<CountryResult>,
        national_match_results: Vec<MatchResult>,
    ) -> Self {
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

        // Phase 3: Continental Competition Processing
        if self.is_competition_draw_period(current_date) {
            self.conduct_competition_draws(data, current_date);
        }

        let competition_results = self.simulate_continental_competitions(data, current_date);
        if let Some(comp_results) = competition_results {
            self.process_competition_results(comp_results, data, result);
        }

        // Phases 2/4/5/6 — continental rankings (monthly), economic zone
        // (quarterly), regulations (yearly), and the player-of-year /
        // cup-finals rank step (yearly) — were hoisted into the
        // simulator's parallel continent pre-pass
        // (`run_continent_periodic_subphases`). Each is disjoint across
        // continents and used to serialise the four heaviest periodic
        // walks behind this `for continent_result in results { … }`
        // drain. The cross-continent player-event slice of awards is
        // applied immediately after that parallel pass, before this
        // method runs.

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
