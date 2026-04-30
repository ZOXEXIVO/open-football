use crate::Club;
use crate::league::LeagueTable;
use crate::r#match::MatchResult;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LeagueStatistics {
    pub total_goals: u32,
    pub total_matches: u32,
    pub top_scorer: Option<(u32, u16)>,
    pub top_assists: Option<(u32, u16)>,
    pub clean_sheets: HashMap<u32, u16>,
    pub competitive_balance_index: f32,
    pub average_attendance: u32,
    pub highest_scoring_match: Option<(u32, u32, u8, u8)>,
    pub biggest_win: Option<(u32, u32, u8)>,
    pub longest_unbeaten_run: Option<(u32, u8)>,
}

impl LeagueStatistics {
    pub fn new() -> Self {
        LeagueStatistics {
            total_goals: 0,
            total_matches: 0,
            top_scorer: None,
            top_assists: None,
            clean_sheets: HashMap::new(),
            competitive_balance_index: 1.0,
            average_attendance: 0,
            highest_scoring_match: None,
            biggest_win: None,
            longest_unbeaten_run: None,
        }
    }

    pub fn process_match_result(&mut self, result: &MatchResult) {
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        self.total_goals += (home_goals + away_goals) as u32;
        self.total_matches += 1;

        let total_in_match = home_goals + away_goals;
        if let Some((_, _, _, current_high)) = self.highest_scoring_match {
            if total_in_match > current_high {
                self.highest_scoring_match = Some((
                    result.score.home_team.team_id,
                    result.score.away_team.team_id,
                    home_goals,
                    away_goals,
                ));
            }
        } else {
            self.highest_scoring_match = Some((
                result.score.home_team.team_id,
                result.score.away_team.team_id,
                home_goals,
                away_goals,
            ));
        }

        let goal_diff = (home_goals as i8 - away_goals as i8).abs() as u8;
        if goal_diff > 0 {
            if let Some((_, _, current_biggest)) = self.biggest_win {
                if goal_diff > current_biggest {
                    let (winner, loser) = if home_goals > away_goals {
                        (
                            result.score.home_team.team_id,
                            result.score.away_team.team_id,
                        )
                    } else {
                        (
                            result.score.away_team.team_id,
                            result.score.home_team.team_id,
                        )
                    };
                    self.biggest_win = Some((winner, loser, goal_diff));
                }
            } else {
                let (winner, loser) = if home_goals > away_goals {
                    (
                        result.score.home_team.team_id,
                        result.score.away_team.team_id,
                    )
                } else {
                    (
                        result.score.away_team.team_id,
                        result.score.home_team.team_id,
                    )
                };
                self.biggest_win = Some((winner, loser, goal_diff));
            }
        }
    }

    pub fn update_player_rankings(&mut self, clubs: &[Club]) {
        let mut scorer_stats: HashMap<u32, u16> = HashMap::new();
        let mut assist_stats: HashMap<u32, u16> = HashMap::new();

        for club in clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.statistics.goals > 0 {
                        scorer_stats.insert(player.id, player.statistics.goals);
                    }
                    if player.statistics.assists > 0 {
                        assist_stats.insert(player.id, player.statistics.assists);
                    }

                    if player.positions.is_goalkeeper() && player.statistics.played > 0 {
                        self.clean_sheets.insert(player.id, 0);
                    }
                }
            }
        }

        self.top_scorer = scorer_stats
            .iter()
            .max_by_key(|(_, goals)| *goals)
            .map(|(id, goals)| (*id, *goals));

        self.top_assists = assist_stats
            .iter()
            .max_by_key(|(_, assists)| *assists)
            .map(|(id, assists)| (*id, *assists));
    }

    pub fn update_competitive_balance(&mut self, table: &LeagueTable) {
        if table.rows.len() < 2 {
            self.competitive_balance_index = 1.0;
            return;
        }

        let mean_points =
            table.rows.iter().map(|r| r.points as f32).sum::<f32>() / table.rows.len() as f32;

        let variance = table
            .rows
            .iter()
            .map(|r| {
                let diff = r.points as f32 - mean_points;
                diff * diff
            })
            .sum::<f32>()
            / table.rows.len() as f32;

        let std_dev = variance.sqrt();
        self.competitive_balance_index = 1.0 / (1.0 + std_dev / 10.0);
    }

    pub fn archive_season_stats(&mut self) {
        debug!("📊 Season Statistics Archived:");
        debug!("  Total Goals: {}", self.total_goals);
        debug!("  Total Matches: {}", self.total_matches);
        debug!(
            "  Goals per Match: {:.2}",
            self.total_goals as f32 / self.total_matches.max(1) as f32
        );
        debug!(
            "  Competitive Balance: {:.2}",
            self.competitive_balance_index
        );

        if let Some((player_id, goals)) = self.top_scorer {
            debug!("  Top Scorer: Player {} with {} goals", player_id, goals);
        }

        self.total_goals = 0;
        self.total_matches = 0;
        self.top_scorer = None;
        self.top_assists = None;
        self.clean_sheets.clear();
        self.highest_scoring_match = None;
        self.biggest_win = None;
        self.longest_unbeaten_run = None;
    }
}
