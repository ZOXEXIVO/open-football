//! Weekly Player of the Week selection and history.
//!
//! Each non-friendly league picks one winner per calendar week from the
//! match results played in the previous seven days. The pick is computed
//! Mondays from `League::matches` (the per-league `MatchStorage`) so
//! teams that drew, were rested, or skipped a fixture still contribute.
//!
//! ## Scoring
//! Per match the player appeared in:
//! ```text
//!   contribution = max(rating - 6.0, 0.0).min(4.0)
//!                + goals * 1.5
//!                + assists * 0.8
//!                + (motm ? 1.5 : 0.0)
//!                + (clean_sheet_for_back_line ? 0.5 : 0.0)
//!                + (team_won ? 0.3 : 0.0)
//! ```
//! Multiple appearances stack with a small multi-match bump
//! (`1 + 0.2 * (n - 1)`) so a player who featured twice in a week edges
//! out a one-match performance of equivalent quality.
//!
//! Tiebreak: total score → highest single-match rating → lower id.

use chrono::NaiveDate;
use std::collections::HashMap;

use crate::PlayerFieldPositionGroup;
use crate::r#match::MatchResult;
use crate::r#match::engine::result::PlayerMatchEndStats;

/// One historical award entry. The denormalised name/club fields let the
/// UI render past awards without re-resolving entities that may have moved
/// or retired since the week the award was given.
#[derive(Debug, Clone)]
pub struct PlayerOfTheWeekAward {
    pub week_end_date: NaiveDate,
    pub player_id: u32,
    pub player_name: String,
    pub player_slug: String,
    pub club_id: u32,
    pub club_name: String,
    pub club_slug: String,
    pub score: f32,
    pub goals: u8,
    pub assists: u8,
    pub matches_played: u8,
    pub average_rating: f32,
}

/// Per-league award archive. Bounded — we cap at three full seasons so the
/// in-memory cost stays predictable even on long saves.
#[derive(Debug, Clone, Default)]
pub struct PlayerOfTheWeekHistory {
    items: Vec<PlayerOfTheWeekAward>,
    last_award_week: Option<NaiveDate>,
}

const MAX_RETAINED_AWARDS: usize = 52 * 3;

impl PlayerOfTheWeekHistory {
    pub fn new() -> Self {
        PlayerOfTheWeekHistory {
            items: Vec::new(),
            last_award_week: None,
        }
    }

    /// Most recent award (highest `week_end_date`).
    pub fn latest(&self) -> Option<&PlayerOfTheWeekAward> {
        self.items.last()
    }

    /// All awards in chronological order (oldest → newest).
    pub fn items(&self) -> &[PlayerOfTheWeekAward] {
        &self.items
    }

    /// `Some(date)` if an award has already been recorded for the calendar
    /// week ending on `week_end_date`. Used by the simulator to avoid
    /// double-firing when the world ticks twice on the same Monday (e.g.
    /// across save / restart in a single in-game tick).
    pub fn has_award_for_week(&self, week_end_date: NaiveDate) -> bool {
        self.last_award_week == Some(week_end_date)
    }

    pub fn record(&mut self, award: PlayerOfTheWeekAward) {
        self.last_award_week = Some(award.week_end_date);
        self.items.push(award);
        let len = self.items.len();
        if len > MAX_RETAINED_AWARDS {
            let drop = len - MAX_RETAINED_AWARDS;
            self.items.drain(0..drop);
        }
    }
}

/// Aggregated weekly stats per candidate. Driver of the final score.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeeklyAggregate {
    pub matches_played: u8,
    pub goals: u8,
    pub assists: u8,
    pub motm_count: u8,
    pub team_wins: u8,
    pub clean_sheets_for_back_line: u8,
    pub rating_sum: f32,
    pub best_rating: f32,
    pub score: f32,
}

/// Stateless scoring helpers for the weekly award. Grouped on a struct
/// rather than left as free functions so the caller pulls a single
/// namespace (`PlayerOfTheWeekSelector::aggregate(...)` /
/// `::pick_winner(...)`) and the unit tests have one entry-point cluster
/// to exercise.
pub struct PlayerOfTheWeekSelector;

impl PlayerOfTheWeekSelector {
    /// Score one player's contribution from a single match.
    fn score_one_match(
        stats: &PlayerMatchEndStats,
        is_motm: bool,
        team_won: bool,
        is_starter: bool,
        team_goals_against: u8,
    ) -> f32 {
        let rating_term = (stats.match_rating - 6.0).max(0.0).min(4.0);
        let goal_term = stats.goals as f32 * 1.5;
        let assist_term = stats.assists as f32 * 0.8;
        let motm_term = if is_motm { 1.5 } else { 0.0 };
        let win_term = if team_won { 0.3 } else { 0.0 };
        let back_line = matches!(
            stats.position_group,
            PlayerFieldPositionGroup::Goalkeeper | PlayerFieldPositionGroup::Defender
        );
        let cs_term = if is_starter && back_line && team_goals_against == 0 {
            0.5
        } else {
            0.0
        };
        rating_term + goal_term + assist_term + motm_term + win_term + cs_term
    }

    fn fold_one_appearance(
        agg: &mut WeeklyAggregate,
        stats: &PlayerMatchEndStats,
        is_motm: bool,
        team_won: bool,
        is_starter: bool,
        team_goals_against: u8,
    ) {
        let contribution = Self::score_one_match(
            stats,
            is_motm,
            team_won,
            is_starter,
            team_goals_against,
        );

        agg.matches_played = agg.matches_played.saturating_add(1);
        agg.goals = agg.goals.saturating_add(stats.goals as u8);
        agg.assists = agg.assists.saturating_add(stats.assists as u8);
        if is_motm {
            agg.motm_count = agg.motm_count.saturating_add(1);
        }
        if team_won {
            agg.team_wins = agg.team_wins.saturating_add(1);
        }
        let back_line = matches!(
            stats.position_group,
            PlayerFieldPositionGroup::Goalkeeper | PlayerFieldPositionGroup::Defender
        );
        if is_starter && back_line && team_goals_against == 0 {
            agg.clean_sheets_for_back_line = agg.clean_sheets_for_back_line.saturating_add(1);
        }
        agg.rating_sum += stats.match_rating;
        if stats.match_rating > agg.best_rating {
            agg.best_rating = stats.match_rating;
        }
        agg.score += contribution;
    }

    /// Aggregate candidate scores over a window of match results. Friendly
    /// matches and matches without `details` (no player stats — typically
    /// AI-batched) are skipped. Caller is responsible for filtering to the
    /// week-of-interest before calling.
    pub fn aggregate<'a, I>(matches: I) -> HashMap<u32, WeeklyAggregate>
    where
        I: IntoIterator<Item = &'a MatchResult>,
    {
        let mut out: HashMap<u32, WeeklyAggregate> = HashMap::new();
        for m in matches {
            if m.friendly {
                continue;
            }
            let Some(details) = m.details.as_ref() else {
                continue;
            };
            let home_id = m.home_team_id;
            let home_goals = m.score.home_team.get();
            let away_goals = m.score.away_team.get();
            let motm = details.player_of_the_match_id;

            for side in [&details.left_team_players, &details.right_team_players] {
                let is_home = side.team_id == home_id;
                let (scored, conceded) = if is_home {
                    (home_goals, away_goals)
                } else {
                    (away_goals, home_goals)
                };
                let team_won = scored > conceded;

                for (pid, is_starter) in side
                    .main
                    .iter()
                    .map(|id| (*id, true))
                    .chain(side.substitutes_used.iter().map(|id| (*id, false)))
                {
                    let Some(stats) = details.player_stats.get(&pid) else {
                        continue;
                    };
                    let is_motm = motm == Some(pid);
                    let agg = out.entry(pid).or_default();
                    Self::fold_one_appearance(
                        agg, stats, is_motm, team_won, is_starter, conceded,
                    );
                }
            }
        }

        // Apply multi-match bonus and finalise.
        for agg in out.values_mut() {
            if agg.matches_played > 1 {
                let bump = 1.0 + 0.2 * (agg.matches_played as f32 - 1.0);
                agg.score *= bump;
            }
        }
        out
    }

    /// Pick the top scorer. Ties broken by best single-match rating, then
    /// lowest player id (deterministic across ticks).
    pub fn pick_winner(
        scores: &HashMap<u32, WeeklyAggregate>,
    ) -> Option<(u32, WeeklyAggregate)> {
        scores
            .iter()
            .filter(|(_, a)| a.matches_played > 0 && a.score > 0.0)
            .max_by(|(la, a), (lb, b)| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(
                        a.best_rating
                            .partial_cmp(&b.best_rating)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                    .then(lb.cmp(la))
            })
            .map(|(id, a)| (*id, *a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;
    use crate::r#match::engine::result::{FieldSquad, MatchResultRaw, PlayerMatchEndStats};
    use crate::r#match::{MatchResult, Score, TeamScore};

    fn empty_stats() -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            shots_on_target: 0,
            shots_total: 0,
            passes_attempted: 0,
            passes_completed: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            goals: 0,
            assists: 0,
            match_rating: 6.0,
            xg: 0.0,
            position_group: PlayerFieldPositionGroup::Midfielder,
            fouls: 0,
            yellow_cards: 0,
            red_cards: 0,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
        }
    }

    fn build_match(
        id: &str,
        home_team: u32,
        away_team: u32,
        home_goals: u8,
        away_goals: u8,
        starters_home: &[u32],
        starters_away: &[u32],
        per_player_stats: Vec<(u32, PlayerMatchEndStats)>,
        motm: Option<u32>,
    ) -> MatchResult {
        let mut details = MatchResultRaw::with_match_time(90 * 60 * 1000);
        details.left_team_players = FieldSquad {
            team_id: home_team,
            main: starters_home.to_vec(),
            substitutes: vec![],
            substitutes_used: vec![],
        };
        details.right_team_players = FieldSquad {
            team_id: away_team,
            main: starters_away.to_vec(),
            substitutes: vec![],
            substitutes_used: vec![],
        };
        for (pid, stats) in per_player_stats {
            details.player_stats.insert(pid, stats);
        }
        details.player_of_the_match_id = motm;

        MatchResult {
            id: id.to_string(),
            league_id: 1,
            league_slug: "league".to_string(),
            home_team_id: home_team,
            away_team_id: away_team,
            details: Some(details),
            score: Score {
                home_team: TeamScore::new_with_score(home_team, home_goals),
                away_team: TeamScore::new_with_score(away_team, away_goals),
                details: vec![],
                home_shootout: 0,
                away_shootout: 0,
            },
            friendly: false,
        }
    }

    #[test]
    fn high_rating_scorer_wins_over_assister() {
        let mut sa = empty_stats();
        sa.match_rating = 7.5;
        sa.goals = 2;
        sa.position_group = PlayerFieldPositionGroup::Forward;

        let mut sb = empty_stats();
        sb.match_rating = 7.0;
        sb.assists = 1;
        sb.position_group = PlayerFieldPositionGroup::Midfielder;

        let m = build_match(
            "m1",
            10,
            20,
            2,
            0,
            &[1, 2],
            &[],
            vec![(1, sa), (2, sb)],
            Some(1),
        );

        let agg = PlayerOfTheWeekSelector::aggregate([&m]);
        let (winner, _) = PlayerOfTheWeekSelector::pick_winner(&agg).expect("a winner");
        assert_eq!(winner, 1);
    }

    #[test]
    fn friendly_matches_are_skipped() {
        let mut s = empty_stats();
        s.match_rating = 9.5;
        s.goals = 5;
        s.position_group = PlayerFieldPositionGroup::Forward;

        let mut m = build_match(
            "f1",
            10,
            20,
            5,
            0,
            &[1],
            &[],
            vec![(1, s)],
            Some(1),
        );
        m.friendly = true;

        let agg = PlayerOfTheWeekSelector::aggregate([&m]);
        assert!(PlayerOfTheWeekSelector::pick_winner(&agg).is_none());
    }

    #[test]
    fn multi_match_player_beats_single_match_equivalent() {
        let mut s_solo = empty_stats();
        s_solo.match_rating = 8.5;
        s_solo.goals = 1;
        s_solo.position_group = PlayerFieldPositionGroup::Forward;

        let mut s_a = empty_stats();
        s_a.match_rating = 7.8;
        s_a.goals = 1;
        s_a.position_group = PlayerFieldPositionGroup::Forward;
        let s_b = s_a.clone();

        let m1 = build_match("m1", 10, 20, 1, 0, &[1], &[], vec![(1, s_solo)], Some(1));
        let m2 = build_match("m2", 30, 40, 1, 0, &[2], &[], vec![(2, s_a)], Some(2));
        let m3 = build_match("m3", 30, 40, 1, 0, &[2], &[], vec![(2, s_b)], Some(2));

        let agg = PlayerOfTheWeekSelector::aggregate([&m1, &m2, &m3]);
        let (winner, _) = PlayerOfTheWeekSelector::pick_winner(&agg).expect("a winner");
        assert_eq!(winner, 2);
    }

    #[test]
    fn defender_clean_sheet_lifts_score() {
        let mut def = empty_stats();
        def.match_rating = 7.4;
        def.position_group = PlayerFieldPositionGroup::Defender;

        let mut mid = empty_stats();
        mid.match_rating = 7.6;
        mid.position_group = PlayerFieldPositionGroup::Midfielder;

        // Same team that won 1-0; both were starters. The defender's
        // clean-sheet bonus should bring them above the marginally
        // higher-rated midfielder on the losing side of the tie.
        let m = build_match(
            "m",
            10,
            20,
            1,
            0,
            &[1, 2],
            &[],
            vec![(1, def), (2, mid)],
            None,
        );
        let agg = PlayerOfTheWeekSelector::aggregate([&m]);
        let (winner, _) = PlayerOfTheWeekSelector::pick_winner(&agg).expect("a winner");
        assert_eq!(winner, 1);
    }

    #[test]
    fn history_records_and_caps_retention() {
        let mut h = PlayerOfTheWeekHistory::new();
        for i in 0..(MAX_RETAINED_AWARDS + 5) {
            h.record(PlayerOfTheWeekAward {
                week_end_date: NaiveDate::from_ymd_opt(2026, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                player_id: i as u32,
                player_name: "n".into(),
                player_slug: "s".into(),
                club_id: 0,
                club_name: "c".into(),
                club_slug: "cs".into(),
                score: 1.0,
                goals: 0,
                assists: 0,
                matches_played: 1,
                average_rating: 7.0,
            });
        }
        assert_eq!(h.items().len(), MAX_RETAINED_AWARDS);
    }

    #[test]
    fn has_award_for_week_blocks_double_fire() {
        let mut h = PlayerOfTheWeekHistory::new();
        let week = NaiveDate::from_ymd_opt(2026, 4, 6).unwrap();
        assert!(!h.has_award_for_week(week));
        h.record(PlayerOfTheWeekAward {
            week_end_date: week,
            player_id: 1,
            player_name: "n".into(),
            player_slug: "s".into(),
            club_id: 0,
            club_name: "c".into(),
            club_slug: "cs".into(),
            score: 1.0,
            goals: 0,
            assists: 0,
            matches_played: 1,
            average_rating: 7.0,
        });
        assert!(h.has_award_for_week(week));
    }
}
