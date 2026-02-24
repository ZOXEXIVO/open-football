use super::{CompetitionStage, ContinentalMatch, ContinentalMatchResult};
use crate::continent::ContinentalRankings;
use crate::Club;
use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ConferenceLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
}

impl ConferenceLeague {
    pub fn new() -> Self {
        ConferenceLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            matches: Vec::new(),
            prize_pool: 250_000_000.0, // 250 million euros
        }
    }

    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, _date: NaiveDate) {
        debug!(
            "Conference League draw conducted with {} clubs",
            clubs.len()
        );
    }

    pub fn has_matches_today(&self, _date: NaiveDate) -> bool {
        false // Simplified
    }

    pub fn simulate_round(
        &mut self,
        _clubs: &HashMap<u32, &Club>,
        _date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        Vec::new() // Simplified
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }
        3.0
    }
}
