use crate::r#match::MatchResult;
use chrono::NaiveDate;
use std::collections::{BTreeMap, HashMap};

/// Default retention window — three completed seasons. Long enough for any
/// realistic UI lookup (historical results, head-to-head, player career
/// recaps within the current save era) while keeping the HashMap bounded
/// on multi-decade saves.
pub const DEFAULT_RETENTION_DAYS: i64 = 365 * 3 + 1;

#[derive(Debug, Clone)]
pub struct MatchStorage {
    results: HashMap<String, MatchResult>,
    /// Secondary index: date → match ids recorded that day. Used to drop
    /// old entries without walking the main HashMap.
    by_date: BTreeMap<NaiveDate, Vec<String>>,
    retention_days: i64,
}

impl Default for MatchStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MatchStorage {
    pub fn new() -> Self {
        MatchStorage {
            results: HashMap::new(),
            by_date: BTreeMap::new(),
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }

    pub fn with_retention_days(mut self, days: i64) -> Self {
        self.retention_days = days.max(30);
        self
    }

    /// Insert a match result tagged with the sim date it was played on.
    /// Older `push` sites that don't have a date handy should pass the
    /// current simulation date; undated inserts would defeat rotation.
    pub fn push(&mut self, match_result: MatchResult, date: NaiveDate) {
        let id = match_result.id.clone();
        self.results.insert(id.clone(), match_result);
        self.by_date.entry(date).or_default().push(id);
    }

    pub fn get<M>(&self, match_id: M) -> Option<&MatchResult>
    where
        M: AsRef<str>,
    {
        self.results.get(match_id.as_ref())
    }

    pub fn len(&self) -> usize {
        self.results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Drop every match recorded before `today − retention_days`. O(K log N)
    /// in the number of evicted dates; cheap to call on season boundaries.
    pub fn trim(&mut self, today: NaiveDate) {
        let cutoff = today - chrono::Duration::days(self.retention_days);
        let evict_dates: Vec<NaiveDate> =
            self.by_date.range(..cutoff).map(|(d, _)| *d).collect();
        for date in evict_dates {
            if let Some(ids) = self.by_date.remove(&date) {
                for id in ids {
                    self.results.remove(&id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::{MatchResult, Score, TeamScore};

    fn mk(id: &str) -> MatchResult {
        MatchResult {
            id: id.to_string(),
            league_slug: "slug".to_string(),
            league_id: 0,
            details: None,
            score: Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: vec![],
                home_shootout: 0,
                away_shootout: 0,
            },
            home_team_id: 0,
            away_team_id: 0,
            friendly: false,
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn test_match_storage_new() {
        let match_storage = MatchStorage::new();
        assert!(match_storage.is_empty());
    }

    #[test]
    fn test_match_storage_push() {
        let mut match_storage = MatchStorage::new();
        let match_result = mk("match_1");
        match_storage.push(match_result.clone(), day(2024, 1, 1));
        assert_eq!(match_storage.len(), 1);
        assert_eq!(match_storage.get("match_1"), Some(&match_result));
    }

    #[test]
    fn test_match_storage_get() {
        let mut match_storage = MatchStorage::new();
        let match_result = mk("match_1");
        match_storage.push(match_result.clone(), day(2024, 1, 1));

        assert_eq!(
            match_storage.get("match_1".to_string()),
            Some(&match_result)
        );
        assert_eq!(match_storage.get("nonexistent_id".to_string()), None);
    }

    #[test]
    fn trim_drops_old_matches() {
        let mut s = MatchStorage::new().with_retention_days(365);
        s.push(mk("old"), day(2020, 1, 1));
        s.push(mk("recent"), day(2024, 6, 1));
        s.trim(day(2024, 12, 31));
        assert!(s.get("old").is_none());
        assert!(s.get("recent").is_some());
    }

    #[test]
    fn trim_uses_retention_window() {
        let mut s = MatchStorage::new().with_retention_days(60);
        s.push(mk("m1"), day(2024, 1, 1));   // 74 days before 2024-03-15
        s.push(mk("m2"), day(2024, 3, 1));   // 14 days before 2024-03-15
        s.trim(day(2024, 3, 15));
        assert!(s.get("m1").is_none());
        assert!(s.get("m2").is_some());
    }
}
