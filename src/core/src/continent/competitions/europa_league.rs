use super::{CompetitionStage, ContinentalMatch, ContinentalMatchResult};
use crate::continent::ContinentalRankings;
use crate::Club;
use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct EuropaLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
}

impl EuropaLeague {
    pub fn new() -> Self {
        EuropaLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            matches: Vec::new(),
            prize_pool: 500_000_000.0, // 500 million euros
        }
    }

    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, _date: NaiveDate) {
        debug!("Europa League draw conducted with {} clubs", clubs.len());
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
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
        5.0
    }
}
