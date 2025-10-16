use crate::r#match::MatchResult;
use std::collections::HashMap;

#[derive(Debug)]
pub struct MatchStorage {
    results: HashMap<String, MatchResult>,
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
        }
    }

    pub fn push(&mut self, match_result: MatchResult) {
        self.results.insert(match_result.id.clone(), match_result);
    }

    pub fn get<M>(&self, match_id: M) -> Option<&MatchResult>
    where
        M: AsRef<str>,
    {
        self.results.get(match_id.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::{MatchResult, Score, TeamScore};

    #[test]
    fn test_match_storage_new() {
        let match_storage = MatchStorage::new();
        assert!(match_storage.results.is_empty());
    }

    #[test]
    fn test_match_storage_push() {
        let mut match_storage = MatchStorage::new();
        let match_result = MatchResult {
            id: "match_1".to_string(),
            league_slug: "slug".to_string(),
            // Fill in other fields as needed
            league_id: 0,
            details: None,
            score: Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: vec![],
            },
            home_team_id: 0,
            away_team_id: 0,
        };
        match_storage.push(match_result.clone());
        assert_eq!(match_storage.results.len(), 1);
        assert_eq!(
            match_storage.results.get(&match_result.id),
            Some(&match_result)
        );
    }

    #[test]
    fn test_match_storage_get() {
        let mut match_storage = MatchStorage::new();
        let match_result = MatchResult {
            id: "match_1".to_string(),
            league_slug: "slug".to_string(),
            league_id: 0,
            details: None,
            score: Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: vec![],
            },
            home_team_id: 0,
            away_team_id: 0,
        };

        match_storage.push(match_result.clone());

        assert_eq!(
            match_storage.get("match_1".to_string()),
            Some(&match_result)
        );
        assert_eq!(match_storage.get("nonexistent_id".to_string()), None);
    }
}
