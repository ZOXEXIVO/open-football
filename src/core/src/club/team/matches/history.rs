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
}

impl MatchHistoryItem {
    pub fn new(date: NaiveDateTime, rival_team_id: u32, score: (TeamScore, TeamScore)) -> Self {
        MatchHistoryItem {
            date,
            rival_team_id,
            score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn item(us: u8, them: u8) -> MatchHistoryItem {
        let date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
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
        for _ in 0..4 { h.add(item(0, 2)); }
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
}
