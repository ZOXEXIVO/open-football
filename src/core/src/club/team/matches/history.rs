use crate::MatchTacticType;
use crate::PlayerPositionType;
use crate::r#match::TeamScore;
use chrono::NaiveDateTime;
use std::cmp::Ordering;

const DEFAULT_MATCH_LIST_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub struct MatchHistory {
    items: Vec<MatchHistoryItem>,
}

/// What a team has done in the season the league says it is in.
///
/// One struct rather than five accessors because every consumer wants all
/// of them at once off a single walk, and because keeping them together is
/// what stops one of them being computed season-scoped and the next
/// career-scoped.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SeasonRecord {
    /// League matches played this season.
    pub played: u8,
    /// Points per match this season; 0.0 before a ball is kicked.
    pub points_per_match: f32,
    /// Goals for minus goals against across the season.
    pub goal_difference: i16,
    pub recent_wins: u8,
    pub recent_draws: u8,
    pub recent_losses: u8,
    /// Goal difference across the form window only.
    pub recent_goal_difference: i16,
}

impl Default for MatchHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl MatchHistory {
    pub fn new() -> Self {
        MatchHistory {
            items: Vec::with_capacity(DEFAULT_MATCH_LIST_SIZE),
        }
    }

    pub fn add(&mut self, item: MatchHistoryItem) {
        self.items.push(item);
    }

    pub fn items(&self) -> &[MatchHistoryItem] {
        &self.items
    }

    /// Wins / draws / losses in the most recent `n` matches. `score.0` is
    /// this team; `score.1` is the opponent. Returns (0, 0, 0) if the team
    /// has no match history yet.
    pub fn recent_results(&self, n: usize) -> (u8, u8, u8) {
        Self::tally(&self.items, n)
    }

    /// The matches belonging to the season the league says the team is in.
    ///
    /// `MatchHistory` accumulates for the life of the team and is never
    /// truncated, so `items().len()` is a CAREER figure. The board judges a
    /// season, and the league is the only place that knows where the season
    /// started — it carries the count. The last `played` items are it.
    ///
    /// Reading the career total instead is what pinned every board in the
    /// world to [`SeasonPhase::RunIn`](crate::club::board::SeasonPhase) from
    /// its second season on: `played > total` for ever after, so the
    /// early-season sacking grace and the softer sporting scale never
    /// applied again.
    pub fn season_slice(&self, played: usize) -> &[MatchHistoryItem] {
        let from = self.items.len().saturating_sub(played);
        &self.items[from..]
    }

    /// Everything the board judges a season on, off one walk of the season
    /// slice. `recent` is the form window (the last `recent` matches *of
    /// this season*, so a season three games old has a three-game form
    /// reading rather than two of last season's).
    ///
    /// A `played` of 0 — pre-season, or a team the league has no count for
    /// — returns the neutral record: no form, no goals, no points.
    pub fn season_record(&self, played: usize, recent: usize) -> SeasonRecord {
        let season = self.season_slice(played);
        if season.is_empty() {
            return SeasonRecord::default();
        }

        let mut points = 0u32;
        let mut goal_difference = 0i16;
        for m in season {
            let (us, them) = (m.score.0.get() as i16, m.score.1.get() as i16);
            goal_difference += us - them;
            match us.cmp(&them) {
                Ordering::Greater => points += 3,
                Ordering::Equal => points += 1,
                Ordering::Less => {}
            }
        }

        let (recent_wins, recent_draws, recent_losses) = Self::tally(season, recent);
        let recent_goal_difference = season
            .iter()
            .rev()
            .take(recent)
            .map(|m| m.score.0.get() as i16 - m.score.1.get() as i16)
            .sum();

        SeasonRecord {
            played: season.len().min(u8::MAX as usize) as u8,
            points_per_match: points as f32 / season.len() as f32,
            goal_difference,
            recent_wins,
            recent_draws,
            recent_losses,
            recent_goal_difference,
        }
    }

    /// Wins / draws / losses across the most recent `n` of `items`.
    fn tally(items: &[MatchHistoryItem], n: usize) -> (u8, u8, u8) {
        let mut wins = 0u8;
        let mut draws = 0u8;
        let mut losses = 0u8;
        for m in items.iter().rev().take(n) {
            let us = m.score.0.get();
            let them = m.score.1.get();
            match us.cmp(&them) {
                Ordering::Greater => wins = wins.saturating_add(1),
                Ordering::Less => losses = losses.saturating_add(1),
                Ordering::Equal => draws = draws.saturating_add(1),
            }
        }
        (wins, draws, losses)
    }

    /// Fraction of the last `n` matches that were wins, or 0.5 when the
    /// team has no recent data — used as a neutral default in form-driven
    /// systems (attendance, board evaluation) so early-season ticks don't
    /// register as a losing streak.
    pub fn recent_wins_ratio(&self, n: usize) -> f32 {
        let (wins, draws, losses) = self.recent_results(n);
        let total = wins + draws + losses;
        if total == 0 {
            0.5
        } else {
            wins as f32 / total as f32
        }
    }
}

#[derive(Debug, Clone)]
pub struct MatchHistoryItem {
    pub date: NaiveDateTime,
    pub rival_team_id: u32,
    pub score: (TeamScore, TeamScore),
    /// The shape this team STARTED the match in (the team's pre-match
    /// plan as captured at kickoff). Different from `tactic_used` only
    /// when the in-match coach changed shape mid-match.
    pub tactic_started: Option<MatchTacticType>,
    /// The shape this team actually finished the match in. May differ
    /// from the starting plan when the in-match coach switched to a
    /// chase / protect / counter shape via
    /// `evaluate_situational_shape`. Lets the web tactics view show
    /// the planned formation alongside what the manager really used.
    pub tactic_used: Option<MatchTacticType>,
    /// Sim-minute at which the FIRST shape change fired (for either
    /// side — the engine records a single minute per match). `None`
    /// when neither side changed shape, which is the common case for
    /// stable scorelines.
    pub tactic_change_minute: Option<u8>,
    /// The starting XI this team fielded at kickoff, each player
    /// paired with the tactical slot they were deployed in. Lets the
    /// web tactics view render the real last-match lineup on the
    /// pitch instead of recomputing "best available" every render.
    /// Empty for legacy items predating the recording (and for paths
    /// that don't go through the squad selector, e.g. dev_match).
    pub starting_eleven: Vec<(u32, PlayerPositionType)>,
    /// Which end of the fixture this team was. Together with the date
    /// and the two team ids this reconstructs the match record's id
    /// (`{date}_{home}_{away}`), which is how the newspaper's scorelines
    /// link back to the match page.
    pub is_home: bool,
}

impl MatchHistoryItem {
    pub fn new(date: NaiveDateTime, rival_team_id: u32, score: (TeamScore, TeamScore)) -> Self {
        MatchHistoryItem {
            date,
            rival_team_id,
            score,
            tactic_started: None,
            tactic_used: None,
            tactic_change_minute: None,
            starting_eleven: Vec::new(),
            is_home: false,
        }
    }

    pub fn with_venue(mut self, is_home: bool) -> Self {
        self.is_home = is_home;
        self
    }

    pub fn with_tactic(mut self, tactic: Option<MatchTacticType>) -> Self {
        self.tactic_used = tactic;
        self
    }

    pub fn with_starting_eleven(mut self, starting_eleven: Vec<(u32, PlayerPositionType)>) -> Self {
        self.starting_eleven = starting_eleven;
        self
    }

    /// Combined tactical summary: starting shape + final shape +
    /// optional first-shape-change minute. When `started` and `final_`
    /// match, the team kept its plan; otherwise the coach shifted.
    pub fn with_tactic_summary(
        mut self,
        started: Option<MatchTacticType>,
        final_: Option<MatchTacticType>,
        change_minute: Option<u8>,
    ) -> Self {
        self.tactic_started = started;
        self.tactic_used = final_;
        // Only stamp the change minute when the shape actually
        // shifted — otherwise a "minute X" label on a kept-plan row
        // would be misleading.
        if started != final_ {
            self.tactic_change_minute = change_minute;
        } else {
            self.tactic_change_minute = None;
        }
        self
    }

    /// True if the team's final shape differed from what they kicked
    /// off with — the canonical "did the manager actually shift?"
    /// signal consumed by the web view and tests.
    pub fn shape_changed(&self) -> bool {
        match (self.tactic_started, self.tactic_used) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn item(us: u8, them: u8) -> MatchHistoryItem {
        let date = NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let our = TeamScore::new_with_score(1, us);
        let their = TeamScore::new_with_score(2, them);
        MatchHistoryItem::new(date, 2, (our, their))
    }

    /// A career's worth of results, then three games of a new season. The
    /// board must read the three.
    #[test]
    fn the_season_record_ignores_the_career_behind_it() {
        let mut history = MatchHistory::new();
        // Two seasons of routine wins — 73 of them.
        for _ in 0..73 {
            history.add(item(2, 0));
        }
        // …then a new campaign: win, draw, loss.
        history.add(item(1, 0));
        history.add(item(1, 1));
        history.add(item(0, 2));

        let record = history.season_record(3, 5);
        assert_eq!(record.played, 3, "the season is three games old");
        // 3 + 1 + 0 points over 3 matches.
        assert!(
            (record.points_per_match - 4.0 / 3.0).abs() < 1e-5,
            "points came from the career, not the season: {}",
            record.points_per_match
        );
        assert_eq!(record.goal_difference, -1);
        assert_eq!(record.recent_wins, 1);
        assert_eq!(record.recent_draws, 1);
        assert_eq!(record.recent_losses, 1);
        assert_eq!(record.recent_goal_difference, -1);
        // The career reading, for contrast: 76 games, nearly all won.
        assert_eq!(history.items().len(), 76);
    }

    /// Pre-season. Nobody has played anybody.
    #[test]
    fn no_matches_this_season_is_a_neutral_record() {
        let mut history = MatchHistory::new();
        for _ in 0..40 {
            history.add(item(3, 0));
        }
        assert_eq!(history.season_record(0, 5), SeasonRecord::default());
    }

    /// A league count larger than the history (a team whose fixtures the
    /// club has not recorded) takes what there is rather than panicking.
    #[test]
    fn a_count_past_the_history_is_clamped() {
        let mut history = MatchHistory::new();
        history.add(item(1, 0));
        let record = history.season_record(38, 5);
        assert_eq!(record.played, 1);
        assert_eq!(record.recent_wins, 1);
    }

    /// The form window is the last N of THIS season, never a couple of
    /// games borrowed from the last one.
    #[test]
    fn the_form_window_stays_inside_the_season() {
        let mut history = MatchHistory::new();
        for _ in 0..10 {
            history.add(item(4, 0)); // last season: emphatic wins
        }
        history.add(item(0, 1)); // this season: two defeats
        history.add(item(0, 1));

        let record = history.season_record(2, 5);
        assert_eq!(record.recent_wins, 0, "last season's wins leaked in");
        assert_eq!(record.recent_losses, 2);
        assert_eq!(record.recent_goal_difference, -2);
    }

    #[test]
    fn empty_history_gives_neutral_ratio_and_zero_counts() {
        let h = MatchHistory::new();
        assert_eq!(h.recent_results(5), (0, 0, 0));
        assert_eq!(h.recent_wins_ratio(5), 0.5);
    }

    #[test]
    fn counts_are_scoped_to_the_last_n_matches() {
        let mut h = MatchHistory::new();
        // Older matches (4 losses) shouldn't bleed into recent window
        for _ in 0..4 {
            h.add(item(0, 2));
        }
        // Recent 3 matches: 2 wins, 1 draw
        h.add(item(2, 1));
        h.add(item(1, 1));
        h.add(item(3, 0));
        assert_eq!(h.recent_results(3), (2, 1, 0));
    }

    #[test]
    fn wins_ratio_divides_by_actual_count_when_history_is_short() {
        let mut h = MatchHistory::new();
        h.add(item(1, 0)); // win
        h.add(item(0, 1)); // loss
        // Only 2 matches even though we asked for 5
        assert!((h.recent_wins_ratio(5) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn shape_changed_reflects_starting_vs_final() {
        let date = NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let kept = MatchHistoryItem::new(
            date,
            99,
            (
                TeamScore::new_with_score(1, 2),
                TeamScore::new_with_score(2, 1),
            ),
        )
        .with_tactic_summary(
            Some(MatchTacticType::T4231),
            Some(MatchTacticType::T4231),
            Some(70),
        );
        assert!(!kept.shape_changed());
        // Plan kept → no minute stamp even when one was offered.
        assert!(kept.tactic_change_minute.is_none());

        let shifted = MatchHistoryItem::new(
            date,
            99,
            (
                TeamScore::new_with_score(1, 0),
                TeamScore::new_with_score(2, 1),
            ),
        )
        .with_tactic_summary(
            Some(MatchTacticType::T442),
            Some(MatchTacticType::T433),
            Some(72),
        );
        assert!(shifted.shape_changed());
        assert_eq!(shifted.tactic_change_minute, Some(72));
        assert_eq!(shifted.tactic_started, Some(MatchTacticType::T442));
        assert_eq!(shifted.tactic_used, Some(MatchTacticType::T433));
    }
}
