use crate::league::{LeagueTableResult, ScheduleItem};
use crate::r#match::{GoalDetail, MatchResult, Score, TeamScore};
use crate::simulator::SimulatorData;
use crate::{MatchHistoryItem, SimulationResult};
use chrono::NaiveDateTime;

pub struct LeagueResult {
    pub league_id: u32,
    pub table_result: LeagueTableResult,
    pub match_results: Option<Vec<MatchResult>>,
}

impl LeagueResult {
    pub fn new(league_id: u32, table_result: LeagueTableResult) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: None,
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
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        if let Some(match_results) = self.match_results {
            for match_result in match_results {
                Self::process_match_results(&match_result, data);

                result.match_results.push(match_result);
            }
        }
    }

    fn process_match_results(result: &MatchResult, data: &mut SimulatorData) {
        let now = data.date;

        let league = data.league_mut(result.league_id).unwrap();

        league.schedule.update_match_result(
            &result.id,
            &result.score
        );

        let home_team = data.team_mut(result.score.home_team.team_id).unwrap();
        home_team.match_history.add(MatchHistoryItem::new(
            now,
            result.score.home_team.team_id,
            (
                TeamScore::from(&result.score.home_team),
                TeamScore::from(&result.score.away_team),
            ),
        ));

        let away_team = data.team_mut(result.score.away_team.team_id).unwrap();
        away_team.match_history.add(MatchHistoryItem::new(
            now,
            result.score.away_team.team_id,
            (
                TeamScore::from(&result.score.away_team),
                TeamScore::from(&result.score.home_team),
            ),
        ));

        // process_match_events(result, data);
        //
        // fn process_match_events(result: &MatchResult, data: &mut SimulatorData) {
        //     for match_event in &result.details.as_ref().unwrap().events {
        //         match match_event {
        //             MatchEvent::MatchPlayed(player_id, is_start_squad, _minutes_played) => {
        //                 let mut player = data.player_mut(*player_id).unwrap();
        //
        //                 if *is_start_squad {
        //                     player.statistics.played += 1;
        //                 } else {
        //                     player.statistics.played_subs += 1;
        //                 }
        //             }
        //             MatchEvent::Goal(player_id) => {
        //                 let mut player = data.player_mut(*player_id).unwrap();
        //                 player.statistics.goals += 1;
        //             }
        //             MatchEvent::Assist(player_id) => {
        //                 let mut player = data.player_mut(*player_id).unwrap();
        //                 player.statistics.assists += 1;
        //             }
        //             MatchEvent::Injury(player_id) => {}
        //         }
        //     }
        // }
    }
}

pub struct LeagueMatch {
    pub id: String,

    pub league_id: u32,
    pub league_slug: String,

    pub date: NaiveDateTime,

    pub home_team_id: u32,
    pub away_team_id: u32,

    pub result: Option<LeagueMatchResultResult>,
}

pub struct LeagueMatchResultResult {
    pub home: TeamScore,
    pub away: TeamScore,
    pub details: Vec<GoalDetail>,
}

impl LeagueMatchResultResult {
    pub fn from_score(score: &Score) -> Self {
        LeagueMatchResultResult {
            home: TeamScore::from(&score.home_team),
            away: TeamScore::from(&score.away_team),
            details: score.detail().to_vec(),
        }
    }

}

impl From<ScheduleItem> for LeagueMatch {
    fn from(item: ScheduleItem) -> Self {
        let mut result = LeagueMatch {
            id: item.id.clone(),
            league_id: item.league_id,
            league_slug: item.league_slug,
            date: item.date,
            home_team_id: item.home_team_id,
            away_team_id: item.away_team_id,
            result: None,
        };

        if let Some(res) = item.result {
            result.result = Some(LeagueMatchResultResult::from_score(&res));
        }

        result
    }
}
