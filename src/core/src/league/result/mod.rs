mod match_events;
mod physical;
mod types;

pub use types::*;

use crate::league::LeagueTableResult;
use crate::r#match::MatchResult;
use crate::simulator::SimulatorData;
use crate::r#match::TeamScore;
use crate::{MatchHistoryItem, SimulationResult};

pub struct LeagueResult {
    pub league_id: u32,
    pub table_result: LeagueTableResult,
    pub match_results: Option<Vec<MatchResult>>,
    pub new_season_started: bool,
}

impl LeagueResult {
    pub fn new(league_id: u32, table_result: LeagueTableResult) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: None,
            new_season_started: false,
        }
    }

    pub fn with_match_result(
        league_id: u32,
        table_result: LeagueTableResult,
        match_results: Vec<MatchResult>,
    ) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: Some(match_results),
            new_season_started: false,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        if let Some(match_results) = self.match_results {
            for mut match_result in match_results {
                Self::process_match_results(&mut match_result, data);

                result.match_results.push(match_result);
            }
        }
    }

    /// Process a cup match result (Champions League, etc.) through the stat pipeline.
    /// Called from continental competition processing.
    pub fn process_cup_match(result: &mut MatchResult, data: &mut SimulatorData) {
        Self::process_match_results(result, data);
    }

    fn process_match_results(result: &mut MatchResult, data: &mut SimulatorData) {
        let now = data.date;

        // Update league schedule (skip for friendlies without a league)
        if let Some(league) = data.league_mut(result.league_id) {
            league.schedule.update_match_result(
                &result.id,
                &result.score,
            );
        }

        let home_team_id = result.score.home_team.team_id;
        let home_team = data.team_mut(home_team_id)
            .expect(&format!("home team not found: {}", home_team_id));
        home_team.match_history.add(MatchHistoryItem::new(
            now,
            home_team_id,
            (
                TeamScore::from(&result.score.home_team),
                TeamScore::from(&result.score.away_team),
            ),
        ));

        let away_team_id = result.score.away_team.team_id;
        let away_team = data.team_mut(away_team_id)
            .expect(&format!("away team not found: {}", away_team_id));
        away_team.match_history.add(MatchHistoryItem::new(
            now,
            away_team_id,
            (
                TeamScore::from(&result.score.away_team),
                TeamScore::from(&result.score.home_team),
            ),
        ));

        Self::process_match_events(result, data);
    }
}
