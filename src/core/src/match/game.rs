use super::engine::FootballEngine;
use crate::MatchRuntime;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::{FixtureContext, MatchResult, MatchSquad};
use log::debug;

#[derive(Debug, Clone)]
pub struct Match {
    id: String,
    league_id: u32,
    league_slug: String,
    pub home_squad: MatchSquad,
    pub away_squad: MatchSquad,
    /// When, where and in what competition — and so its format, weather,
    /// crowd and referee.
    pub fixture: FixtureContext,
}

impl Match {
    pub fn make(
        id: String,
        league_id: u32,
        league_slug: &str,
        home_squad: MatchSquad,
        away_squad: MatchSquad,
        fixture: FixtureContext,
    ) -> Self {
        Match {
            id,
            league_id,
            league_slug: String::from(league_slug),
            home_squad,
            away_squad,
            fixture,
        }
    }

    pub fn is_friendly(&self) -> bool {
        self.fixture.is_friendly()
    }

    /// Level after 90 minutes, it goes to extra time and penalties.
    pub fn is_knockout(&self) -> bool {
        self.fixture.knockout
    }

    /// Accessors for the private identity fields (used by the
    /// distributed worker wire layer to flatten a Match across the
    /// network). Internal mutation still flows through `make` /
    /// `make_knockout`, so keeping the fields private elsewhere is
    /// intentional.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn league_id(&self) -> u32 {
        self.league_id
    }

    pub fn league_slug(&self) -> &str {
        &self.league_slug
    }

    pub fn play(self) -> MatchResult {
        let home_team_id = self.home_squad.team_id;
        let home_team_name = String::from(&self.home_squad.team_name);

        let away_team_id = self.away_squad.team_id;
        let away_team_name = String::from(&self.away_squad.team_name);

        // One flag for every match. Friendlies used to be excluded here — and
        // again at the store and again on the match page — from when a
        // recording meant a full ninety minutes of samples and there were six
        // times as many youth fixtures as senior ones to pay for. A recording
        // is now the goals and nothing else by default
        // (`RecordingScope::Goals`), which is the same reason
        // `Settings::match_recordings` became opt-OUT, and it applies just as
        // well to a reserve derby: there is no longer a cost worth carrying a
        // special case for. A goal in an U19 game is a goal somebody wants to
        // watch.
        let config =
            MatchEngineConfig::for_fixture(&self.fixture, MatchRuntime::recordings_mode());
        let match_result =
            FootballEngine::<840, 545>::play_with_config(self.home_squad, self.away_squad, config);

        let score = match_result.score.as_ref().expect("no score");

        if score.had_shootout() {
            debug!(
                "match played: {} {}:{} {} ({}:{} pens)",
                home_team_name,
                score.home_team.get(),
                away_team_name,
                score.away_team.get(),
                score.home_shootout,
                score.away_shootout,
            );
        } else {
            debug!(
                "match played: {} {}:{} {}",
                home_team_name,
                score.home_team.get(),
                away_team_name,
                score.away_team.get(),
            );
        }

        MatchResult {
            id: self.id,
            league_id: self.league_id,
            league_slug: String::from(&self.league_slug),
            home_team_id,
            away_team_id,
            score: score.clone(),
            details: Some(match_result),
            friendly: self.fixture.is_friendly(),
        }
    }
}
