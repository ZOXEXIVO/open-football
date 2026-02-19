use crate::league::{LeagueTableResult, ScheduleItem};
use crate::r#match::player::statistics::MatchStatisticType;
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
            &result.score,
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

        Self::process_match_events(result, data);
    }

    fn process_match_events(result: &MatchResult, data: &mut SimulatorData) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        // Mark players as played (main squad) or played_subs (substitutes)
        for player_id in &details.left_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played += 1;
            }
        }
        for player_id in &details.left_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played_subs += 1;
            }
        }
        for player_id in &details.right_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played += 1;
            }
        }
        for player_id in &details.right_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played_subs += 1;
            }
        }

        // Goals and assists from score details
        for detail in &result.score.details {
            match detail.stat_type {
                MatchStatisticType::Goal => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        player.statistics.goals += 1;
                    }
                }
                MatchStatisticType::Assist => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        player.statistics.assists += 1;
                    }
                }
            }
        }

        // Per-player stats (shots, passes, tackles, rating)
        let mut best_rating: f32 = 0.0;
        let mut best_player_id: Option<u32> = None;

        for (player_id, stats) in &details.player_stats {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.shots_on_target += stats.shots_on_target as f32;
                player.statistics.tackling += stats.tackles as f32;
                if stats.passes_attempted > 0 {
                    let match_pct = (stats.passes_completed as f32 / stats.passes_attempted as f32 * 100.0) as u8;
                    let games = player.statistics.played + player.statistics.played_subs;
                    if games <= 1 {
                        player.statistics.passes = match_pct;
                    } else {
                        let prev = player.statistics.passes as f32;
                        player.statistics.passes = ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8;
                    }
                }

                // Update running average rating
                let games = player.statistics.played + player.statistics.played_subs;
                if games <= 1 {
                    player.statistics.average_rating = stats.match_rating;
                } else {
                    let prev = player.statistics.average_rating;
                    player.statistics.average_rating =
                        (prev * (games - 1) as f32 + stats.match_rating) / games as f32;
                }

                // Track best rating for player of the match
                if stats.match_rating > best_rating {
                    best_rating = stats.match_rating;
                    best_player_id = Some(*player_id);
                }
            }
        }

        // Award player of the match
        if let Some(motm_id) = best_player_id {
            if let Some(player) = data.player_mut(motm_id) {
                player.statistics.player_of_the_match += 1;
            }
        }
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
