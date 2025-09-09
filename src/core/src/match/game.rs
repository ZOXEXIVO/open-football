use super::engine::FootballEngine;
use crate::r#match::{MatchResult, MatchSquad};
use log::debug;

#[derive(Debug, Clone)]
pub struct Match {
    id: String,
    league_id: u32,
    league_slug: String,
    pub home_squad: MatchSquad,
    pub away_squad: MatchSquad,
}

impl Match {
    pub fn make(
        id: String,
        league_id: u32,
        league_slug: &str,
        home_squad: MatchSquad,
        away_squad: MatchSquad,
    ) -> Self {
        Match {
            id,
            league_id,
            league_slug: String::from(league_slug),
            home_squad,
            away_squad,
        }
    }

    pub fn play(self) -> MatchResult {
        let home_team_id = self.home_squad.team_id;
        let home_team_name = String::from(&self.home_squad.team_name);

        let away_team_id = self.away_squad.team_id;
        let away_team_name = String::from(&self.away_squad.team_name);

        let match_result = FootballEngine::<840, 545>::play(self.home_squad, self.away_squad);

        let score = match_result.score.as_ref().expect("no score");

        debug!(
            "match played: {} {}:{} {}",
            home_team_name,
            score.home_team.get(),
            away_team_name,
            score.away_team.get(),
        );

        MatchResult {
            id: self.id,
            league_id: self.league_id,
            league_slug: String::from(&self.league_slug),
            home_team_id,
            away_team_id,
            score: score.clone(),
            details: Some(match_result),
        }
    }
}
