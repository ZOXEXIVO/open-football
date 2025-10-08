use crate::context::GlobalContext;
use crate::league::LeagueTableResult;
use crate::r#match::MatchResult;
use std::cmp::Ordering;

#[derive(Debug)]
pub struct LeagueTable {
    pub rows: Vec<LeagueTableRow>,
}

impl LeagueTable {
    pub fn new(teams: &[u32]) -> Self {
        LeagueTable {
            rows: Self::generate_for_teams(teams),
        }
    }

    pub fn simulate(&mut self, ctx: &GlobalContext<'_>) -> LeagueTableResult {
        if self.rows.is_empty() {
            let league_ctx = ctx.league.as_ref().unwrap();
            self.rows = Self::generate_for_teams(league_ctx.team_ids);
        }

        LeagueTableResult {}
    }

    fn generate_for_teams(teams: &[u32]) -> Vec<LeagueTableRow> {
        let mut rows = Vec::with_capacity(teams.len());

        for team_id in teams {
            let table_row = LeagueTableRow {
                team_id: *team_id,
                played: 0,
                win: 0,
                draft: 0,
                lost: 0,
                goal_scored: 0,
                goal_concerned: 0,
                points: 0,
            };

            rows.push(table_row)
        }

        rows
    }

    #[inline]
    fn get_team_mut(&mut self, team_id: u32) -> &mut LeagueTableRow {
        self.rows.iter_mut().find(|c| c.team_id == team_id).unwrap()
    }

    fn winner(&mut self, team_id: u32, goal_scored: u8, goal_concerned: u8) {
        let team = self.get_team_mut(team_id);

        team.played += 1;
        team.win += 1;
        team.goal_scored += goal_scored as i32;
        team.goal_concerned += goal_concerned as i32;
        team.points += 3;
    }

    fn looser(&mut self, team_id: u32, goal_scored: u8, goal_concerned: u8) {
        let team = self.get_team_mut(team_id);

        team.played += 1;
        team.lost += 1;
        team.goal_scored += goal_scored as i32;
        team.goal_concerned += goal_concerned as i32;
    }

    fn draft(&mut self, team_id: u32, goal_scored: u8, goal_concerned: u8) {
        let team = self.get_team_mut(team_id);

        team.played += 1;
        team.draft += 1;
        team.goal_scored += goal_scored as i32;
        team.goal_concerned += goal_concerned as i32;
        team.points += 1;
    }

    pub fn update_from_results(&mut self, match_result: &[MatchResult]) {
        for result in match_result {
            match Ord::cmp(&result.score.home_team.get(), &result.score.away_team.get()) {
                Ordering::Equal => {
                    self.draft(
                        result.score.home_team.team_id,
                        result.score.home_team.get(),
                        result.score.away_team.get(),
                    );
                    self.draft(
                        result.score.away_team.team_id,
                        result.score.away_team.get(),
                        result.score.home_team.get(),
                    );
                }
                Ordering::Greater => {
                    self.winner(
                        result.score.home_team.team_id,
                        result.score.home_team.get(),
                        result.score.away_team.get(),
                    );
                    self.looser(
                        result.score.away_team.team_id,
                        result.score.away_team.get(),
                        result.score.home_team.get(),
                    );
                }
                Ordering::Less => {
                    self.looser(
                        result.score.home_team.team_id,
                        result.score.home_team.get(),
                        result.score.away_team.get(),
                    );
                    self.winner(
                        result.score.away_team.team_id,
                        result.score.away_team.get(),
                        result.score.home_team.get(),
                    );
                }
            }
        }

        self.rows.sort_by(|a, b| {
            match b.points.cmp(&a.points) {
                Ordering::Equal => match (b.goal_scored - b.goal_concerned).cmp(&(a.goal_scored - a.goal_concerned)) {
                    Ordering::Equal => b.goal_scored.cmp(&a.goal_scored),
                    other => other,
                },
                other => other,
            }
        });
    }

    pub fn get(&self) -> &[LeagueTableRow] {
        &self.rows
    }
}

#[derive(Debug)]
pub struct LeagueTableRow {
    pub team_id: u32,
    pub played: u8,
    pub win: u8,
    pub draft: u8,
    pub lost: u8,
    pub goal_scored: i32,
    pub goal_concerned: i32,
    pub points: u8,
}

impl LeagueTableRow {}

impl Default for LeagueTable {
    fn default() -> Self {
        LeagueTable { rows: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::{Score, TeamScore};

    #[test]
    fn table_draft() {
        let first_team_id = 1;
        let second_team_id = 2;

        let teams = vec![first_team_id, second_team_id];

        let mut table = LeagueTable::new(&teams);

        let match_results = vec![MatchResult {
            league_id: 0,
            id: "123".to_string(),
            league_slug: "slug".to_string(),
            home_team_id: 1,
            away_team_id: 2,
            score: Score {
                home_team: TeamScore::new_with_score(1, 3),
                away_team: TeamScore::new_with_score(2, 3),
                details: vec![],
            },
            details: None,
        }];

        table.update_from_results(&match_results);

        let returned_table = table.get();

        let home = &returned_table[0];

        assert_eq!(1, home.played);
        assert_eq!(1, home.draft);
        assert_eq!(0, home.win);
        assert_eq!(0, home.lost);
        assert_eq!(3, home.goal_scored);
        assert_eq!(3, home.goal_concerned);
        assert_eq!(1, home.points);

        let away = &returned_table[0];

        assert_eq!(1, away.played);
        assert_eq!(1, away.draft);
        assert_eq!(0, away.win);
        assert_eq!(0, away.lost);
        assert_eq!(3, away.goal_scored);
        assert_eq!(3, away.goal_concerned);
        assert_eq!(1, away.points);
    }

    #[test]
    fn table_winner() {
        let first_team_id = 1;
        let second_team_id = 2;

        let teams = vec![first_team_id, second_team_id];

        let mut table = LeagueTable::new(&teams);

        let home_team_id = 1;
        let away_team_id = 2;

        let match_results = vec![MatchResult {
            league_id: 0,
            id: "123".to_string(),
            league_slug: "slug".to_string(),
            home_team_id,
            away_team_id,
            score: Score {
                home_team: TeamScore::new_with_score(1, 3),
                away_team: TeamScore::new_with_score(2, 0),
                details: vec![],
            },
            details: None,
        }];

        table.update_from_results(&match_results);

        let returned_table = table.get();

        let home = returned_table
            .iter()
            .find(|c| c.team_id == home_team_id)
            .unwrap();

        assert_eq!(1, home.team_id);
        assert_eq!(1, home.played);
        assert_eq!(0, home.draft);
        assert_eq!(1, home.win);
        assert_eq!(0, home.lost);
        assert_eq!(3, home.goal_scored);
        assert_eq!(0, home.goal_concerned);
        assert_eq!(3, home.points);

        let away = returned_table
            .iter()
            .find(|c| c.team_id == away_team_id)
            .unwrap();

        assert_eq!(2, away.team_id);
        assert_eq!(1, away.played);
        assert_eq!(0, away.draft);
        assert_eq!(0, away.win);
        assert_eq!(1, away.lost);
        assert_eq!(0, away.goal_scored);
        assert_eq!(3, away.goal_concerned);
        assert_eq!(0, away.points);
    }

    #[test]
    fn table_looser() {
        let first_team_id = 1;
        let second_team_id = 2;

        let teams = vec![first_team_id, second_team_id];

        let mut table = LeagueTable::new(&teams);

        let home_team_id = 1;
        let away_team_id = 2;

        let match_results = vec![MatchResult {
            league_id: 0,
            id: "123".to_string(),
            league_slug: "slug".to_string(),
            home_team_id,
            away_team_id,
            score: Score {
                home_team: TeamScore::new_with_score(1, 0),
                away_team: TeamScore::new_with_score(2, 3),
                details: vec![],
            },
            details: None,
        }];

        table.update_from_results(&match_results);

        let returned_table = table.get();

        let home = returned_table
            .iter()
            .find(|c| c.team_id == home_team_id)
            .unwrap();

        assert_eq!(1, home.team_id);
        assert_eq!(1, home.played);
        assert_eq!(0, home.draft);
        assert_eq!(0, home.win);
        assert_eq!(1, home.lost);
        assert_eq!(0, home.goal_scored);
        assert_eq!(3, home.goal_concerned);
        assert_eq!(0, home.points);

        let away = returned_table
            .iter()
            .find(|c| c.team_id == away_team_id)
            .unwrap();

        assert_eq!(2, away.team_id);
        assert_eq!(1, away.played);
        assert_eq!(0, away.draft);
        assert_eq!(1, away.win);
        assert_eq!(0, away.lost);
        assert_eq!(3, away.goal_scored);
        assert_eq!(0, away.goal_concerned);
        assert_eq!(3, away.points);
    }
}
