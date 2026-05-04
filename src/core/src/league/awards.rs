//! Monthly / season / Team of the Week archives + selectors.
//!
//! `PlayerOfTheWeekHistory` and `PlayerOfTheWeekSelector` live in their
//! own module for backwards compatibility; the broader award archive
//! sits here.

use chrono::NaiveDate;
use std::collections::HashMap;

use crate::PlayerFieldPositionGroup;
use crate::r#match::MatchResult;
use crate::r#match::engine::result::PlayerMatchEndStats;

/// Position group breakdown the Team of the Week selector enforces.
const TOTW_GK: usize = 1;
const TOTW_DEF: usize = 4;
const TOTW_MID: usize = 4;
const TOTW_FWD: usize = 2;

/// Per-archive bounds. Weekly is owned by `PlayerOfTheWeekHistory`; the
/// rest are sized for ~3y of monthly awards and ~20 seasons.
pub const TOTW_MAX_RETAINED: usize = 52 * 3;
pub const MONTHLY_MAX_RETAINED: usize = 36;
pub const SEASON_MAX_RETAINED: usize = 20;

/// One spot in a team-of-the-week selection. Position group is preserved
/// so the UI can render the XI in a 1-4-3-3 layout without re-classifying.
#[derive(Debug, Clone)]
pub struct TeamOfTheWeekSlot {
    pub player_id: u32,
    pub player_name: String,
    pub player_slug: String,
    pub club_id: u32,
    pub club_name: String,
    pub club_slug: String,
    pub position_group: PlayerFieldPositionGroup,
    pub score: f32,
    pub matches_played: u8,
    pub goals: u8,
    pub assists: u8,
    pub average_rating: f32,
}

#[derive(Debug, Clone)]
pub struct TeamOfTheWeekAward {
    pub week_end_date: NaiveDate,
    pub slots: Vec<TeamOfTheWeekSlot>,
}

#[derive(Debug, Clone)]
pub struct MonthlyPlayerAward {
    pub month_end_date: NaiveDate,
    pub player_id: u32,
    pub player_name: String,
    pub player_slug: String,
    pub club_id: u32,
    pub club_name: String,
    pub club_slug: String,
    pub matches_played: u8,
    pub goals: u8,
    pub assists: u8,
    pub average_rating: f32,
    pub score: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SeasonAwardsSnapshot {
    pub season_end_date: NaiveDate,
    pub player_of_season: Option<u32>,
    pub young_player_of_season: Option<u32>,
    /// Best XI of the season — same positional quotas as Team of the Week.
    pub team_of_season: Vec<u32>,
    pub top_scorer: Option<u32>,
    pub top_assists: Option<u32>,
    pub golden_glove: Option<u32>,
    /// Champion / top-4 / relegated team ids, captured from final_table at
    /// snapshot time so the per-player team finish multiplier is stable.
    pub champion_team_id: Option<u32>,
    pub top_four_team_ids: Vec<u32>,
    pub relegated_team_ids: Vec<u32>,
}

/// Bounded archive of league award history beyond the weekly POW.
#[derive(Debug, Clone, Default)]
pub struct LeagueAwards {
    pub team_of_week: Vec<TeamOfTheWeekAward>,
    pub player_of_month: Vec<MonthlyPlayerAward>,
    pub young_player_of_month: Vec<MonthlyPlayerAward>,
    pub season_awards: Vec<SeasonAwardsSnapshot>,
    /// Set on season-end before stats are archived; consumed by the
    /// simulator-level `SeasonAwardsTick` to emit player events while
    /// the snapshot data is still meaningful.
    pub pending_season_awards: Option<SeasonAwardsSnapshot>,
    pub last_team_of_week: Option<NaiveDate>,
    pub last_monthly_award: Option<NaiveDate>,
}

impl LeagueAwards {
    pub fn record_team_of_week(&mut self, award: TeamOfTheWeekAward) {
        self.last_team_of_week = Some(award.week_end_date);
        self.team_of_week.push(award);
        if self.team_of_week.len() > TOTW_MAX_RETAINED {
            let drop = self.team_of_week.len() - TOTW_MAX_RETAINED;
            self.team_of_week.drain(0..drop);
        }
    }

    pub fn record_player_of_month(&mut self, award: MonthlyPlayerAward) {
        self.last_monthly_award = Some(award.month_end_date);
        self.player_of_month.push(award);
        if self.player_of_month.len() > MONTHLY_MAX_RETAINED {
            let drop = self.player_of_month.len() - MONTHLY_MAX_RETAINED;
            self.player_of_month.drain(0..drop);
        }
    }

    pub fn record_young_player_of_month(&mut self, award: MonthlyPlayerAward) {
        self.young_player_of_month.push(award);
        if self.young_player_of_month.len() > MONTHLY_MAX_RETAINED {
            let drop = self.young_player_of_month.len() - MONTHLY_MAX_RETAINED;
            self.young_player_of_month.drain(0..drop);
        }
    }

    pub fn record_season(&mut self, snapshot: SeasonAwardsSnapshot) {
        self.season_awards.push(snapshot);
        if self.season_awards.len() > SEASON_MAX_RETAINED {
            let drop = self.season_awards.len() - SEASON_MAX_RETAINED;
            self.season_awards.drain(0..drop);
        }
    }

    pub fn has_team_of_week_for(&self, week_end_date: NaiveDate) -> bool {
        self.last_team_of_week == Some(week_end_date)
    }

    pub fn has_monthly_award_for(&self, month_end_date: NaiveDate) -> bool {
        self.last_monthly_award == Some(month_end_date)
    }
}

/// Per-candidate aggregate over a window (week or month). Reused by
/// monthly and Team of the Week selectors — they share the same fold
/// shape and only differ in scoring weights.
#[derive(Debug, Clone, Copy, Default)]
pub struct CandidateAggregate {
    pub matches_played: u8,
    pub goals: u8,
    pub assists: u8,
    pub motm_count: u8,
    pub team_wins: u8,
    pub yellow_cards: u16,
    pub red_cards: u16,
    pub clean_sheets_for_back_line: u8,
    pub saves: u32,
    pub defensive_actions: u32,
    pub errors_leading_to_goal: u16,
    pub rating_sum: f32,
    pub best_rating: f32,
    pub primary_position: Option<PlayerFieldPositionGroup>,
}

impl CandidateAggregate {
    pub fn average_rating(&self) -> f32 {
        if self.matches_played > 0 {
            self.rating_sum / self.matches_played as f32
        } else {
            0.0
        }
    }
}

/// Stateless aggregation of per-player stats across a set of matches.
/// Friendlies and matches without `details` are skipped.
pub struct AwardAggregator;

impl AwardAggregator {
    pub fn aggregate<'a, I>(matches: I) -> HashMap<u32, CandidateAggregate>
    where
        I: IntoIterator<Item = &'a MatchResult>,
    {
        let mut out: HashMap<u32, CandidateAggregate> = HashMap::new();
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
                    Self::fold_one(agg, stats, is_motm, team_won, is_starter, conceded);
                }
            }
        }
        out
    }

    fn fold_one(
        agg: &mut CandidateAggregate,
        stats: &PlayerMatchEndStats,
        is_motm: bool,
        team_won: bool,
        is_starter: bool,
        team_goals_against: u8,
    ) {
        agg.matches_played = agg.matches_played.saturating_add(1);
        agg.goals = agg.goals.saturating_add(stats.goals as u8);
        agg.assists = agg.assists.saturating_add(stats.assists as u8);
        if is_motm {
            agg.motm_count = agg.motm_count.saturating_add(1);
        }
        if team_won {
            agg.team_wins = agg.team_wins.saturating_add(1);
        }
        agg.yellow_cards = agg.yellow_cards.saturating_add(stats.yellow_cards);
        agg.red_cards = agg.red_cards.saturating_add(stats.red_cards);
        let back_line = matches!(
            stats.position_group,
            PlayerFieldPositionGroup::Goalkeeper | PlayerFieldPositionGroup::Defender
        );
        if is_starter && back_line && team_goals_against == 0 {
            agg.clean_sheets_for_back_line = agg.clean_sheets_for_back_line.saturating_add(1);
        }
        if matches!(stats.position_group, PlayerFieldPositionGroup::Goalkeeper) {
            agg.saves = agg.saves.saturating_add(stats.saves as u32);
        }
        if matches!(
            stats.position_group,
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Midfielder
        ) {
            let actions = stats.tackles as u32
                + stats.interceptions as u32
                + stats.blocks as u32
                + stats.clearances as u32;
            agg.defensive_actions = agg.defensive_actions.saturating_add(actions);
        }
        agg.errors_leading_to_goal = agg
            .errors_leading_to_goal
            .saturating_add(stats.errors_leading_to_goal);
        agg.rating_sum += stats.match_rating;
        if stats.match_rating > agg.best_rating {
            agg.best_rating = stats.match_rating;
        }
        // First seen position wins; aggregation across position changes
        // is rare enough that this is good-enough.
        if agg.primary_position.is_none() {
            agg.primary_position = Some(stats.position_group);
        }
    }
}

/// Team of the Week selector. Picks 1 GK + 4 DEF + 3 MID + 3 FWD.
pub struct TeamOfTheWeekSelector;

impl TeamOfTheWeekSelector {
    /// Compute one player's contribution score from their aggregate.
    pub fn candidate_score(agg: &CandidateAggregate) -> f32 {
        let pos = agg.primary_position.unwrap_or(PlayerFieldPositionGroup::Midfielder);
        let avg = agg.average_rating();
        let rating_term = (avg - 6.0).max(0.0).min(4.0) * 2.0;

        let (goal_w, assist_w) = match pos {
            PlayerFieldPositionGroup::Forward => (1.4, 1.0),
            PlayerFieldPositionGroup::Midfielder => (1.8, 1.4),
            PlayerFieldPositionGroup::Defender => (2.5, 1.8),
            PlayerFieldPositionGroup::Goalkeeper => (2.5, 0.0),
        };
        let goal_term = agg.goals as f32 * goal_w;
        let assist_term = agg.assists as f32 * assist_w;

        let cs_term = match pos {
            PlayerFieldPositionGroup::Goalkeeper => agg.clean_sheets_for_back_line as f32 * 1.5,
            PlayerFieldPositionGroup::Defender => agg.clean_sheets_for_back_line as f32 * 1.0,
            _ => 0.0,
        };
        let saves_term = (agg.saves as f32 * 0.25).min(2.0);
        let def_term = match pos {
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Midfielder => {
                (agg.defensive_actions as f32 * 0.08).min(2.0)
            }
            _ => 0.0,
        };
        let error_penalty = agg.errors_leading_to_goal as f32 * -2.5;
        let red_penalty = agg.red_cards as f32 * -3.0;
        let win_bonus = agg.team_wins as f32 * 0.4;

        let raw = rating_term
            + goal_term
            + assist_term
            + cs_term
            + saves_term
            + def_term
            + error_penalty
            + red_penalty
            + win_bonus;

        let multi_match_bump = if agg.matches_played > 1 {
            (1.0 + 0.15 * (agg.matches_played as f32 - 1.0)).min(1.30)
        } else {
            1.0
        };

        raw * multi_match_bump
    }

    /// Pick the 11 players, deterministic tiebreak: score → best rating →
    /// matches played → lower id. Weekly selection has no minimum-apps
    /// gate (a player who featured once last week is still eligible);
    /// season selection should call [`pick_with_min_apps`].
    pub fn pick(
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<(u32, PlayerFieldPositionGroup, f32, CandidateAggregate)> {
        Self::pick_with_min_apps(scores, 1)
    }

    /// Same as [`pick`] but each candidate must have at least `min_apps`
    /// matches played in the window. Used by the season Team of the
    /// Season so a player with one elite shift can't outrank a regular
    /// starter.
    pub fn pick_with_min_apps(
        scores: &HashMap<u32, CandidateAggregate>,
        min_apps: u8,
    ) -> Vec<(u32, PlayerFieldPositionGroup, f32, CandidateAggregate)> {
        let mut by_pos: HashMap<PlayerFieldPositionGroup, Vec<(u32, f32, CandidateAggregate)>> =
            HashMap::new();
        for (id, agg) in scores {
            if agg.matches_played < min_apps {
                continue;
            }
            let pos = agg
                .primary_position
                .unwrap_or(PlayerFieldPositionGroup::Midfielder);
            let s = Self::candidate_score(agg);
            if s <= 0.0 {
                continue;
            }
            by_pos.entry(pos).or_default().push((*id, s, *agg));
        }
        let cmp = |a: &(u32, f32, CandidateAggregate), b: &(u32, f32, CandidateAggregate)| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    b.2.best_rating
                        .partial_cmp(&a.2.best_rating)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.2.matches_played.cmp(&a.2.matches_played))
                .then(a.0.cmp(&b.0))
        };
        for v in by_pos.values_mut() {
            v.sort_by(cmp);
        }
        let mut out: Vec<(u32, PlayerFieldPositionGroup, f32, CandidateAggregate)> = Vec::new();
        let pick_n = |grp: PlayerFieldPositionGroup,
                      n: usize,
                      pool: Option<&Vec<(u32, f32, CandidateAggregate)>>,
                      out: &mut Vec<(u32, PlayerFieldPositionGroup, f32, CandidateAggregate)>| {
            if let Some(v) = pool {
                for (id, score, agg) in v.iter().take(n) {
                    out.push((*id, grp, *score, *agg));
                }
            }
        };
        pick_n(
            PlayerFieldPositionGroup::Goalkeeper,
            TOTW_GK,
            by_pos.get(&PlayerFieldPositionGroup::Goalkeeper),
            &mut out,
        );
        pick_n(
            PlayerFieldPositionGroup::Defender,
            TOTW_DEF,
            by_pos.get(&PlayerFieldPositionGroup::Defender),
            &mut out,
        );
        pick_n(
            PlayerFieldPositionGroup::Midfielder,
            TOTW_MID,
            by_pos.get(&PlayerFieldPositionGroup::Midfielder),
            &mut out,
        );
        pick_n(
            PlayerFieldPositionGroup::Forward,
            TOTW_FWD,
            by_pos.get(&PlayerFieldPositionGroup::Forward),
            &mut out,
        );
        out
    }
}

/// Monthly / season scoring helpers — same `CandidateAggregate` shape.
pub struct MonthlyAwardSelector;

impl MonthlyAwardSelector {
    pub fn score(agg: &CandidateAggregate, league_reputation: u16) -> f32 {
        let pos = agg
            .primary_position
            .unwrap_or(PlayerFieldPositionGroup::Midfielder);
        let avg = agg.average_rating();
        let avg_rating_score = (avg - 6.0).max(0.0) * 8.0;

        let (goal_w, assist_w) = match pos {
            PlayerFieldPositionGroup::Forward => (1.4, 1.1),
            PlayerFieldPositionGroup::Midfielder => (1.8, 1.5),
            PlayerFieldPositionGroup::Defender => (2.5, 1.8),
            PlayerFieldPositionGroup::Goalkeeper => (2.5, 0.0),
        };
        let goal_term = agg.goals as f32 * goal_w;
        let assist_term = agg.assists as f32 * assist_w;
        let motm_term = agg.motm_count as f32 * 2.0;
        let cs_term = match pos {
            PlayerFieldPositionGroup::Goalkeeper => agg.clean_sheets_for_back_line as f32 * 1.5,
            PlayerFieldPositionGroup::Defender => agg.clean_sheets_for_back_line as f32 * 0.8,
            _ => 0.0,
        };
        let discipline = agg.yellow_cards as f32 * -0.25 + agg.red_cards as f32 * -2.5;
        let team_wins = agg.team_wins as f32 * 0.4;

        let raw = avg_rating_score
            + goal_term
            + assist_term
            + motm_term
            + cs_term
            + discipline
            + team_wins;

        let league_rep_mul = 0.75 + (league_reputation as f32 / 10000.0) * 0.5;
        raw * league_rep_mul
    }

    /// Best player in `scores` who passed the minimum-appearances gate.
    /// `tie_break_player_id` is required for the deterministic tiebreak
    /// (lower id wins after score and best-rating).
    pub fn pick_best(
        scores: &HashMap<u32, CandidateAggregate>,
        league_reputation: u16,
        min_apps: u8,
        eligibility: impl Fn(u32) -> bool,
    ) -> Option<(u32, CandidateAggregate, f32)> {
        scores
            .iter()
            .filter(|(_, a)| a.matches_played >= min_apps)
            .filter(|(id, _)| eligibility(**id))
            .map(|(id, a)| (*id, *a, Self::score(a, league_reputation)))
            .filter(|(_, _, s)| *s > 0.0)
            .max_by(|(la, aa, sa), (lb, ab, sb)| {
                sa.partial_cmp(sb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(
                        aa.best_rating
                            .partial_cmp(&ab.best_rating)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                    .then(aa.matches_played.cmp(&ab.matches_played))
                    .then(lb.cmp(la))
            })
    }
}

/// Season-long scoring (same shape, season weights / multipliers).
pub struct SeasonAwardSelector;

impl SeasonAwardSelector {
    pub fn score(
        agg: &CandidateAggregate,
        league_reputation: u16,
        team_finish_mul: f32,
        weekly_awards: u8,
    ) -> f32 {
        let pos = agg
            .primary_position
            .unwrap_or(PlayerFieldPositionGroup::Midfielder);
        let avg = agg.average_rating();
        let avg_rating_score = (avg - 6.0).max(0.0) * 18.0;

        let (goal_w, assist_w) = match pos {
            PlayerFieldPositionGroup::Forward => (1.4, 1.1),
            PlayerFieldPositionGroup::Midfielder => (1.8, 1.5),
            PlayerFieldPositionGroup::Defender => (2.5, 1.8),
            PlayerFieldPositionGroup::Goalkeeper => (2.5, 0.0),
        };
        let goal_term = agg.goals as f32 * goal_w;
        let assist_term = agg.assists as f32 * assist_w;
        let motm_term = agg.motm_count as f32 * 3.0;
        let weekly_term = weekly_awards as f32 * 2.0;
        let cs_term = match pos {
            PlayerFieldPositionGroup::Goalkeeper => agg.clean_sheets_for_back_line as f32 * 1.5,
            PlayerFieldPositionGroup::Defender => agg.clean_sheets_for_back_line as f32 * 0.8,
            _ => 0.0,
        };
        let discipline = agg.yellow_cards as f32 * -0.15 + agg.red_cards as f32 * -2.0;

        let raw =
            avg_rating_score + goal_term + assist_term + motm_term + weekly_term + cs_term + discipline;

        let league_rep_mul = 0.70 + (league_reputation as f32 / 10000.0) * 0.6;
        raw * league_rep_mul * team_finish_mul.max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agg(
        pos: PlayerFieldPositionGroup,
        played: u8,
        goals: u8,
        assists: u8,
        avg_rating: f32,
        wins: u8,
    ) -> CandidateAggregate {
        let mut a = CandidateAggregate::default();
        a.matches_played = played;
        a.goals = goals;
        a.assists = assists;
        a.team_wins = wins;
        a.rating_sum = avg_rating * played as f32;
        a.best_rating = avg_rating;
        a.primary_position = Some(pos);
        a
    }

    #[test]
    fn totw_pick_respects_quotas() {
        let mut scores: HashMap<u32, CandidateAggregate> = HashMap::new();
        // 3 GKs (only 1 selected)
        for i in 1..=3u32 {
            scores.insert(
                i,
                make_agg(PlayerFieldPositionGroup::Goalkeeper, 1, 0, 0, 8.0 - i as f32 * 0.1, 1),
            );
            // GKs need a clean sheet to show up in score; boost manually:
            let agg = scores.get_mut(&i).unwrap();
            agg.clean_sheets_for_back_line = 1;
        }
        // 8 defenders → 4 selected
        for i in 10..=17u32 {
            let mut a = make_agg(PlayerFieldPositionGroup::Defender, 1, 0, 0, 7.5, 1);
            a.clean_sheets_for_back_line = 1;
            scores.insert(i, a);
        }
        // 6 mids → 4 selected
        for i in 20..=25u32 {
            scores.insert(
                i,
                make_agg(PlayerFieldPositionGroup::Midfielder, 1, 1, 0, 7.6, 1),
            );
        }
        // 5 fwds → 2 selected
        for i in 30..=34u32 {
            scores.insert(
                i,
                make_agg(PlayerFieldPositionGroup::Forward, 1, 1, 0, 7.6, 1),
            );
        }
        let team = TeamOfTheWeekSelector::pick(&scores);
        let gk_count = team
            .iter()
            .filter(|(_, p, ..)| matches!(p, PlayerFieldPositionGroup::Goalkeeper))
            .count();
        let def_count = team
            .iter()
            .filter(|(_, p, ..)| matches!(p, PlayerFieldPositionGroup::Defender))
            .count();
        let mid_count = team
            .iter()
            .filter(|(_, p, ..)| matches!(p, PlayerFieldPositionGroup::Midfielder))
            .count();
        let fwd_count = team
            .iter()
            .filter(|(_, p, ..)| matches!(p, PlayerFieldPositionGroup::Forward))
            .count();
        assert_eq!(gk_count, 1);
        assert_eq!(def_count, 4);
        assert_eq!(mid_count, 4);
        assert_eq!(fwd_count, 2);
        assert_eq!(team.len(), 11);
    }

    #[test]
    fn monthly_min_apps_gate_filters() {
        let mut scores: HashMap<u32, CandidateAggregate> = HashMap::new();
        scores.insert(
            1,
            make_agg(PlayerFieldPositionGroup::Forward, 2, 5, 2, 9.0, 2),
        );
        scores.insert(
            2,
            make_agg(PlayerFieldPositionGroup::Forward, 3, 1, 0, 7.2, 1),
        );
        let pick = MonthlyAwardSelector::pick_best(&scores, 8000, 3, |_| true).map(|(id, _, _)| id);
        assert_eq!(pick, Some(2), "id 1 has only 2 apps and is gated out");
    }

    #[test]
    fn tots_min_apps_excludes_one_match_wonder() {
        let mut scores: HashMap<u32, CandidateAggregate> = HashMap::new();
        // One-match elite player: rating 10.0, hat-trick — would win
        // if Team of the Week selector were used directly.
        scores.insert(
            1,
            make_agg(PlayerFieldPositionGroup::Forward, 1, 3, 0, 10.0, 1),
        );
        // Regular starter: 28 apps, more modest per-match output.
        scores.insert(
            2,
            make_agg(PlayerFieldPositionGroup::Forward, 28, 18, 6, 7.4, 18),
        );

        let weekly = TeamOfTheWeekSelector::pick(&scores);
        let weekly_fwd: Vec<u32> = weekly
            .iter()
            .filter(|(_, p, ..)| matches!(p, PlayerFieldPositionGroup::Forward))
            .map(|(id, ..)| *id)
            .collect();
        assert!(
            weekly_fwd.contains(&1),
            "weekly view: one-match wonder should rank ahead by raw score"
        );

        let season = TeamOfTheWeekSelector::pick_with_min_apps(&scores, 10);
        let season_fwd: Vec<u32> = season
            .iter()
            .filter(|(_, p, ..)| matches!(p, PlayerFieldPositionGroup::Forward))
            .map(|(id, ..)| *id)
            .collect();
        assert_eq!(season_fwd, vec![2]);
    }

    #[test]
    fn monthly_eligibility_predicate_filters() {
        let mut scores: HashMap<u32, CandidateAggregate> = HashMap::new();
        scores.insert(
            1,
            make_agg(PlayerFieldPositionGroup::Forward, 4, 5, 2, 9.0, 3),
        );
        scores.insert(
            2,
            make_agg(PlayerFieldPositionGroup::Forward, 4, 1, 0, 7.0, 2),
        );
        // Eligibility excludes id 1; id 2 should win.
        let pick =
            MonthlyAwardSelector::pick_best(&scores, 8000, 3, |id| id != 1).map(|(id, _, _)| id);
        assert_eq!(pick, Some(2));
    }
}
