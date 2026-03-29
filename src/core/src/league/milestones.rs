use crate::league::{LeagueStatistics, LeagueTable};
use chrono::{Datelike, NaiveDate};
use log::debug;

#[derive(Debug, Clone)]
pub struct LeagueMilestones {
    pub all_time_records: AllTimeRecords,
    pub season_milestones: Vec<Milestone>,
    pub historic_champions: Vec<(u16, u32)>,
}

impl LeagueMilestones {
    pub fn new() -> Self {
        LeagueMilestones {
            all_time_records: AllTimeRecords::default(),
            season_milestones: Vec::new(),
            historic_champions: Vec::new(),
        }
    }

    pub fn check_records(&mut self, stats: &LeagueStatistics, table: &LeagueTable) {
        if let Some(leader) = table.rows.first() {
            if leader.points > self.all_time_records.most_points_in_season.1 {
                debug!("📊 NEW RECORD! Team {} has {} points!", leader.team_id, leader.points);
                self.all_time_records.most_points_in_season = (leader.team_id, leader.points);
            }

            if leader.goal_scored > self.all_time_records.most_goals_in_season.1 {
                debug!("⚽ NEW RECORD! Team {} has scored {} goals!",
                      leader.team_id, leader.goal_scored);
                self.all_time_records.most_goals_in_season =
                    (leader.team_id, leader.goal_scored);
            }
        }

        if let Some((player_id, goals)) = stats.top_scorer {
            if goals > self.all_time_records.most_goals_by_player.1 {
                debug!("🎯 NEW RECORD! Player {} has scored {} goals!", player_id, goals);
                self.all_time_records.most_goals_by_player = (player_id, goals);
            }
        }
    }

    pub fn check_season_milestones(&mut self, matches_played: u8, table: &LeagueTable) {
        let total_matches = 38;
        let matches_remaining = total_matches - matches_played;

        if table.rows.len() >= 2 {
            let leader = &table.rows[0];
            let second = &table.rows[1];
            let max_possible_points_second = second.points + (matches_remaining * 3);

            if leader.points > max_possible_points_second {
                let milestone = Milestone {
                    milestone_type: MilestoneType::TitleWon,
                    team_id: leader.team_id,
                    description: format!("Title won with {} matches to spare!", matches_remaining),
                    matches_played,
                };
                self.season_milestones.push(milestone);
                debug!("🏆 {} wins the title with {} matches remaining!",
                      leader.team_id, matches_remaining);
            }
        }

        for row in &table.rows {
            if row.lost == 0 && matches_played >= 10 {
                let milestone = Milestone {
                    milestone_type: MilestoneType::UnbeatenRun,
                    team_id: row.team_id,
                    description: format!("Unbeaten in {} matches", matches_played),
                    matches_played,
                };

                if !self.season_milestones.iter().any(|m|
                    m.milestone_type == MilestoneType::UnbeatenRun &&
                        m.team_id == row.team_id
                ) {
                    self.season_milestones.push(milestone);
                    debug!("💪 Team {} is unbeaten after {} matches!", row.team_id, matches_played);
                }
            }
        }
    }

    pub fn record_champion(&mut self, team_id: u32, date: NaiveDate) {
        let year = date.year() as u16;
        self.historic_champions.push((year, team_id));

        let consecutive_titles = self.historic_champions.iter()
            .rev()
            .take_while(|(_, id)| *id == team_id)
            .count();

        if consecutive_titles >= 3 {
            debug!("👑 Dynasty! Team {} wins {} consecutive titles!",
                  team_id, consecutive_titles);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AllTimeRecords {
    pub most_points_in_season: (u32, u8),
    pub most_goals_in_season: (u32, i32),
    pub fewest_goals_conceded: (u32, i32),
    pub most_goals_by_player: (u32, u16),
    pub longest_winning_streak: (u32, u8),
    pub longest_unbeaten_streak: (u32, u8),
}

#[derive(Debug, Clone)]
pub struct Milestone {
    pub milestone_type: MilestoneType,
    pub team_id: u32,
    pub description: String,
    pub matches_played: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MilestoneType {
    TitleWon,
    RelegationConfirmed,
    UnbeatenRun,
    WinningStreak,
    GoalRecord,
    PointsRecord,
}
