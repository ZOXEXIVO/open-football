use crate::context::GlobalContext;
use crate::league::LeagueTableResult;
use crate::r#match::MatchResult;
use std::cmp::Ordering;

/// Configurable tie-break order for the league table. Each variant is a
/// concrete sort key derived from a row; they are compared in the order
/// the policy lists them. Default policy is FIFA's standard chain
/// (points → goal difference → goals scored → wins → team_id) — the
/// last key keeps sorts deterministic across re-runs without leaving the
/// outcome dependent on insertion order.
///
/// `HeadToHead` is reserved as a public hook: a future implementation
/// may carry the per-pair record on the table itself; for now the
/// comparator returns `Ordering::Equal` so the chain falls through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieBreakRule {
    Points,
    GoalDifference,
    GoalsScored,
    Wins,
    HeadToHead,
    TeamId,
}

#[derive(Debug, Clone)]
pub struct TieBreakPolicy {
    pub rules: Vec<TieBreakRule>,
}

impl TieBreakPolicy {
    pub fn fifa_default() -> Self {
        TieBreakPolicy {
            rules: vec![
                TieBreakRule::Points,
                TieBreakRule::GoalDifference,
                TieBreakRule::GoalsScored,
                TieBreakRule::Wins,
                TieBreakRule::TeamId,
            ],
        }
    }

    /// Compare two rows: `a < b` means `a` ranks higher (sorts first).
    /// All non-id keys descend (more is better); team_id ascends so the
    /// numerically smaller id wins the otherwise-tied bucket.
    pub fn compare(&self, a: &LeagueTableRow, b: &LeagueTableRow) -> Ordering {
        for rule in &self.rules {
            let ord = match rule {
                TieBreakRule::Points => b.effective_points().cmp(&a.effective_points()),
                TieBreakRule::GoalDifference => b.goal_difference().cmp(&a.goal_difference()),
                TieBreakRule::GoalsScored => b.goal_scored.cmp(&a.goal_scored),
                TieBreakRule::Wins => b.win.cmp(&a.win),
                TieBreakRule::HeadToHead => Ordering::Equal,
                TieBreakRule::TeamId => a.team_id.cmp(&b.team_id),
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }
}

impl Default for TieBreakPolicy {
    fn default() -> Self {
        Self::fifa_default()
    }
}

#[derive(Debug, Clone)]
pub struct LeagueTable {
    pub rows: Vec<LeagueTableRow>,
    pub tie_break: TieBreakPolicy,
}

impl LeagueTable {
    pub fn new(teams: &[u32]) -> Self {
        LeagueTable {
            rows: Self::generate_for_teams(teams),
            tie_break: TieBreakPolicy::fifa_default(),
        }
    }

    pub fn with_policy(teams: &[u32], policy: TieBreakPolicy) -> Self {
        LeagueTable {
            rows: Self::generate_for_teams(teams),
            tie_break: policy,
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
                points_deduction: 0,
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

    /// Apply a one-shot points deduction to a team. Tracked separately
    /// from earned points so the deduction is auditable and idempotent
    /// when an FFP / disciplinary case re-fires its sanction. Returns
    /// the deduction actually recorded (saturating at u8::MAX).
    pub fn apply_points_deduction(&mut self, team_id: u32, amount: u8) -> u8 {
        let Some(row) = self.rows.iter_mut().find(|r| r.team_id == team_id) else {
            return 0;
        };
        let new_total = row.points_deduction.saturating_add(amount);
        let actual = new_total - row.points_deduction;
        row.points_deduction = new_total;
        self.resort();
        actual
    }

    /// Re-sort the rows in place under the active tie-break policy.
    /// Public for callers (e.g. regulations.rs) that adjust state on a
    /// row directly and want to refresh the standings.
    pub fn resort(&mut self) {
        let policy = self.tie_break.clone();
        self.rows.sort_by(|a, b| policy.compare(a, b));
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

        self.resort();
    }

    pub fn get(&self) -> &[LeagueTableRow] {
        &self.rows
    }
}

#[derive(Debug, Clone)]
pub struct LeagueTableRow {
    pub team_id: u32,
    pub played: u8,
    pub win: u8,
    pub draft: u8,
    pub lost: u8,
    pub goal_scored: i32,
    pub goal_concerned: i32,
    /// Earned points only — match results never subtract from this.
    /// The actual standings figure used in sorting is
    /// `effective_points()`, which subtracts `points_deduction`.
    pub points: u8,
    /// Cumulative points deducted by the league (FFP, disciplinary,
    /// administrative sanctions). Tracked separately so the same case
    /// is not re-applied on every matchday tick and so the UI can
    /// surface the original earned figure alongside the penalty.
    pub points_deduction: u8,
}

impl LeagueTableRow {
    /// Standings-effective points after deductions, never negative.
    pub fn effective_points(&self) -> u8 {
        self.points.saturating_sub(self.points_deduction)
    }

    pub fn goal_difference(&self) -> i32 {
        self.goal_scored - self.goal_concerned
    }
}

impl Default for LeagueTable {
    fn default() -> Self {
        LeagueTable {
            rows: Vec::new(),
            tie_break: TieBreakPolicy::fifa_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::{Score, TeamScore};

    fn make_row(
        team_id: u32,
        played: u8,
        win: u8,
        draft: u8,
        lost: u8,
        gs: i32,
        gc: i32,
        points: u8,
    ) -> LeagueTableRow {
        LeagueTableRow {
            team_id,
            played,
            win,
            draft,
            lost,
            goal_scored: gs,
            goal_concerned: gc,
            points,
            points_deduction: 0,
        }
    }

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
                home_shootout: 0,
                away_shootout: 0,
            },
            details: None,
            friendly: false,
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
                home_shootout: 0,
                away_shootout: 0,
            },
            details: None,
            friendly: false,
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
                home_shootout: 0,
                away_shootout: 0,
            },
            details: None,
            friendly: false,
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

    #[test]
    fn tie_break_orders_by_goal_difference_when_points_equal() {
        let policy = TieBreakPolicy::fifa_default();
        // a: 10pts, +5 GD. b: 10pts, +3 GD. a should rank higher.
        let a = make_row(1, 10, 3, 1, 6, 12, 7, 10);
        let b = make_row(2, 10, 3, 1, 6, 8, 5, 10);
        assert_eq!(policy.compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn tie_break_orders_by_goals_scored_when_gd_equal() {
        let policy = TieBreakPolicy::fifa_default();
        // Both 10pts +3 GD. a scored 15 vs b scored 8 → a higher.
        let a = make_row(1, 10, 3, 1, 6, 15, 12, 10);
        let b = make_row(2, 10, 3, 1, 6, 8, 5, 10);
        assert_eq!(policy.compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn tie_break_orders_by_wins_when_gs_equal() {
        let policy = TieBreakPolicy::fifa_default();
        // Same points / GD / GS but a has more wins (and balanced draws/losses).
        let a = make_row(1, 10, 4, 0, 6, 10, 7, 12);
        let b = make_row(2, 10, 3, 3, 4, 10, 7, 12);
        assert_eq!(policy.compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn tie_break_orders_by_team_id_when_all_else_equal() {
        let policy = TieBreakPolicy::fifa_default();
        let a = make_row(1, 10, 3, 1, 6, 10, 7, 10);
        let b = make_row(2, 10, 3, 1, 6, 10, 7, 10);
        // Lower team_id ranks higher.
        assert_eq!(policy.compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn point_deduction_is_separate_and_idempotent_via_effective_points() {
        let teams = vec![1u32, 2, 3];
        let mut table = LeagueTable::new(&teams);
        // Set up: team 1 has 30 earned points; team 2 has 25; team 3 has 20.
        for (id, p) in [(1u32, 30u8), (2, 25), (3, 20)] {
            let row = table.rows.iter_mut().find(|r| r.team_id == id).unwrap();
            row.points = p;
            row.played = 10;
        }
        table.resort();
        // Initial standing: team 1 first.
        assert_eq!(table.rows[0].team_id, 1);
        // Apply -10 deduction to team 1 → effective = 20, drops to last.
        let applied = table.apply_points_deduction(1, 10);
        assert_eq!(applied, 10);
        let row1 = table.rows.iter().find(|r| r.team_id == 1).unwrap();
        // Earned points untouched.
        assert_eq!(row1.points, 30);
        assert_eq!(row1.points_deduction, 10);
        assert_eq!(row1.effective_points(), 20);
        // Standings: team 2 (25) first, then team 1 / team 3 tied on 20 — id breaks the tie.
        assert_eq!(table.rows[0].team_id, 2);
        assert_eq!(table.rows[1].team_id, 1);
        assert_eq!(table.rows[2].team_id, 3);
    }
}
