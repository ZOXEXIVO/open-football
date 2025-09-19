use std::collections::HashMap;
use crate::continent::{CompetitionTier, ContinentalMatchResult, ContinentalRankings};
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use crate::SimulationResult;
use crate::transfers::CompletedTransfer;

pub struct ContinentResult {
    pub countries: Vec<CountryResult>,

    // New fields for continental-level results
    pub competition_results: Option<ContinentalCompetitionResults>,
    pub rankings_update: Option<ContinentalRankingsUpdate>,
    pub transfer_summary: Option<CrossBorderTransferSummary>,
    pub economic_impact: Option<EconomicZoneImpact>,
}

impl ContinentResult {
    // Original constructor for backward compatibility
    pub fn new(countries: Vec<CountryResult>) -> Self {
        ContinentResult {
            countries,
            competition_results: None,
            rankings_update: None,
            transfer_summary: None,
            economic_impact: None,
        }
    }

    // Enhanced constructor with all continental data
    pub fn with_enhanced_data(
        countries: Vec<CountryResult>,
        competition_results: ContinentalCompetitionResults,
        rankings: ContinentalRankings,
    ) -> Self {
        ContinentResult {
            countries,
            competition_results: Some(competition_results),
            rankings_update: Some(ContinentalRankingsUpdate::from_rankings(rankings)),
            transfer_summary: None,  // Would be populated if transfers occurred
            economic_impact: None,    // Would be populated quarterly
        }
    }

    // Enhanced constructor with full data
    pub fn with_full_data(
        countries: Vec<CountryResult>,
        competition_results: Option<ContinentalCompetitionResults>,
        rankings_update: Option<ContinentalRankingsUpdate>,
        transfer_summary: Option<CrossBorderTransferSummary>,
        economic_impact: Option<EconomicZoneImpact>,
    ) -> Self {
        ContinentResult {
            countries,
            competition_results,
            rankings_update,
            transfer_summary,
            economic_impact,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        // Process country results first
        for country_result in self.countries {
            country_result.process(data, result);
        }

        // Process continental competition results
        if let Some(comp_results) = self.competition_results {
            //self.process_competition_results(comp_results, data, result);
        }

        // Update continental rankings in data
        if let Some(rankings) = self.rankings_update {
           // self.process_rankings_update(rankings, data);
        }

        // Apply economic impacts
        if let Some(economic) = self.economic_impact {
           // self.process_economic_impact(economic, data);
        }
    }

    fn process_competition_results(
        &self,
        results: ContinentalCompetitionResults,
        data: &mut SimulatorData
    ) {
        // Process Champions League results
        if let Some(cl_results) = results.champions_league_results {
            for match_result in cl_results {
                // Update club statistics
                self.update_club_continental_stats(match_result.home_team, &match_result, data);
                self.update_club_continental_stats(match_result.away_team, &match_result, data);

                // Store for output
                //result.continental_matches.push(match_result);
            }
        }

        // Process Europa League results
        if let Some(el_results) = results.europa_league_results {
            for match_result in el_results {
                self.update_club_continental_stats(match_result.home_team, &match_result, data);
                self.update_club_continental_stats(match_result.away_team, &match_result, data);

                //result.continental_matches.push(match_result);
            }
        }

        // Process Conference League results
        if let Some(conf_results) = results.conference_league_results {
            for match_result in conf_results {
                self.update_club_continental_stats(match_result.home_team, &match_result, data);
                self.update_club_continental_stats(match_result.away_team, &match_result, data);
                //result.continental_matches.push(match_result);
            }
        }
    }

    fn update_club_continental_stats(
        &self,
        club_id: u32,
        match_result: &ContinentalMatchResult,
        data: &mut SimulatorData,
    ) {
        if let Some(club) = data.club_mut(club_id) {
            // Update club's continental record
            // This would require adding continental_record to Club struct

            // Update finances with match revenue
            let match_revenue = self.calculate_match_revenue(match_result);
            club.finance.balance.push_income(match_revenue as i32);

            // Update reputation based on result
            if match_result.home_team == club_id {
                if match_result.home_score > match_result.away_score {
                    // Win bonus to reputation
                    // club.reputation.continental += 1;
                }
            } else if match_result.away_team == club_id {
                if match_result.away_score > match_result.home_score {
                    // Win bonus to reputation
                    // club.reputation.continental += 1;
                }
            }
        }
    }

    fn calculate_match_revenue(&self, match_result: &ContinentalMatchResult) -> f64 {
        // Calculate revenue based on competition tier
        match match_result.competition {
            CompetitionTier::ChampionsLeague => 3_000_000.0,   // €3M per match
            CompetitionTier::EuropaLeague => 1_000_000.0,      // €1M per match
            CompetitionTier::ConferenceLeague => 500_000.0,    // €500K per match
        }
    }

    fn process_rankings_update(&self, rankings: ContinentalRankingsUpdate, data: &mut SimulatorData) {
        // Update continental rankings in the simulator data
        // This would require adding continental rankings to the continent structure

        for (country_id, coefficient) in rankings.country_updates {
            if let Some(country) = data.country_mut(country_id) {
                // Update country's continental coefficient
                // country.continental_coefficient = coefficient;
            }
        }

        for (club_id, points) in rankings.club_updates {
            if let Some(club) = data.club_mut(club_id) {
                // Update club's continental points
                // club.continental_points = points;
            }
        }
    }

    fn process_economic_impact(&self, impact: EconomicZoneImpact, data: &mut SimulatorData) {
        // Apply economic impacts to all countries in the continent
        let multiplier = impact.economic_multiplier;

        // This would require accessing the continent and updating all countries
        // For example:
        // for country in continent.countries {
        //     country.economic_factors.apply_continental_multiplier(multiplier);
        // }
    }
}

// Supporting structures for the result

#[derive(Debug, Clone)]
pub struct ContinentalRankingsUpdate {
    pub country_updates: Vec<(u32, f32)>,  // country_id, new coefficient
    pub club_updates: Vec<(u32, f32)>,     // club_id, new points
    pub qualification_changes: Vec<QualificationChange>,
}

impl ContinentalRankingsUpdate {
    pub fn from_rankings(rankings: ContinentalRankings) -> Self {
        ContinentalRankingsUpdate {
            country_updates: rankings.country_rankings,
            club_updates: rankings.club_rankings,
            qualification_changes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualificationChange {
    pub country_id: u32,
    pub competition: CompetitionTier,
    pub old_spots: u8,
    pub new_spots: u8,
}

#[derive(Debug, Clone)]
pub struct CrossBorderTransferSummary {
    pub completed_transfers: Vec<CompletedTransfer>,
    pub total_value: f64,
    pub most_expensive: Option<CompletedTransfer>,
    pub by_country_flow: HashMap<u32, TransferFlow>,  // country_id -> flow stats
}

#[derive(Debug, Clone)]
pub struct TransferFlow {
    pub incoming_transfers: u32,
    pub outgoing_transfers: u32,
    pub net_spend: f64,
}

#[derive(Debug, Clone)]
pub struct EconomicZoneImpact {
    pub economic_multiplier: f32,
    pub tv_rights_change: f64,
    pub sponsorship_change: f64,
    pub overall_health_change: f32,
}

// Extension to SimulationResult to include continental matches
impl SimulationResult {
    // Note: This would need to be added to the actual SimulationResult struct
    // pub continental_matches: Vec<ContinentalMatchResult>,
}

#[derive(Debug)]
pub struct ContinentalCompetitionResults {
    pub champions_league_results: Option<Vec<ContinentalMatchResult>>,
    pub europa_league_results: Option<Vec<ContinentalMatchResult>>,
    pub conference_league_results: Option<Vec<ContinentalMatchResult>>,
}

impl ContinentalCompetitionResults {
    pub fn new() -> Self {
        ContinentalCompetitionResults {
            champions_league_results: None,
            europa_league_results: None,
            conference_league_results: None,
        }
    }
}