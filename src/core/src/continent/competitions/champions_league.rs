use super::{CompetitionStage, CompetitionTier, ContinentalMatch, ContinentalMatchResult};
use crate::continent::ContinentalRankings;
use crate::Club;
use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ChampionsLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
}

impl Default for ChampionsLeague {
    fn default() -> Self {
        Self::new()
    }
}

impl ChampionsLeague {
    pub fn new() -> Self {
        ChampionsLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            matches: Vec::new(),
            prize_pool: 2_000_000_000.0, // 2 billion euros
        }
    }

    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, _date: NaiveDate) {
        // Implement draw logic with seeding based on rankings
        debug!("Champions League draw conducted with {} clubs", clubs.len());
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
    }

    pub fn simulate_round(
        &mut self,
        _clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        let mut results = Vec::new();

        for match_to_play in self.matches.iter_mut().filter(|m| m.date == date) {
            // Simulate match (simplified)
            let result = ContinentalMatchResult {
                home_team: match_to_play.home_team,
                away_team: match_to_play.away_team,
                home_score: 0,
                away_score: 0,
                competition: CompetitionTier::ChampionsLeague,
            };

            results.push(result);
        }

        results
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        // Points based on performance
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }

        // Simplified: base points for participation
        10.0
    }
}
