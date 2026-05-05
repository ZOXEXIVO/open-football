use crate::MatchTacticType;
use crate::r#match::TeamScore;
use chrono::NaiveDateTime;

const DEFAULT_MATCH_LIST_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub struct MatchHistory {
    items: Vec<MatchHistoryItem>,
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
        let mut wins = 0u8;
        let mut draws = 0u8;
        let mut losses = 0u8;
        for m in self.items.iter().rev().take(n) {
            let us = m.score.0.get();
            let them = m.score.1.get();
            match us.cmp(&them) {
                std::cmp::Ordering::Greater => wins = wins.saturating_add(1),
                std::cmp::Ordering::Less => losses = losses.saturating_add(1),
                std::cmp::Ordering::Equal => draws = draws.saturating_add(1),
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
        }
    }

    pub fn with_tactic(mut self, tactic: Option<MatchTacticType>) -> Self {
        self.tactic_used = tactic;
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
