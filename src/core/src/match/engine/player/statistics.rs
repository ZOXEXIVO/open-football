#[derive(Debug, Clone)]
pub struct MatchPlayerStatistics {
    pub items: Vec<MatchPlayerStatisticsItem>,
}

impl MatchPlayerStatistics {
    pub fn new() -> Self {
        MatchPlayerStatistics {
            items: Vec::with_capacity(5),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn add_goal(&mut self, match_second: u64, is_auto_goal: bool) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::Goal,
            match_second,
            is_auto_goal
        })
    }

    pub fn add_assist(&mut self, match_second: u64) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::Assist,
            match_second,
            is_auto_goal: false
        })
    }
}

impl Default for MatchPlayerStatistics {
    fn default() -> Self {
        MatchPlayerStatistics::new()
    }
}

#[derive(Debug, Copy, Clone)]
pub struct MatchPlayerStatisticsItem {
    pub stat_type: MatchStatisticType,
    pub match_second: u64,
    pub is_auto_goal: bool,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MatchStatisticType {
    Goal,
    Assist,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics_initialization() {
        let stats = MatchPlayerStatistics::new();
        assert!(stats.is_empty());
        assert!(stats.items.is_empty());
    }

    #[test]
    fn test_add_goal() {
        let mut stats = MatchPlayerStatistics::new();
        stats.add_goal(30, false);

        assert_eq!(stats.items.len(), 1);
        assert_eq!(stats.items[0].stat_type, MatchStatisticType::Goal);
        assert_eq!(stats.items[0].match_second, 30);
        assert!(!stats.is_empty());
    }

    #[test]
    fn test_add_assist() {
        let mut stats = MatchPlayerStatistics::new();
        stats.add_assist(45);

        assert_eq!(stats.items.len(), 1);
        assert_eq!(stats.items[0].stat_type, MatchStatisticType::Assist);
        assert_eq!(stats.items[0].match_second, 45);
        assert!(!stats.is_empty());
    }

    #[test]
    fn test_is_empty() {
        let stats = MatchPlayerStatistics::new();
        assert!(stats.is_empty());

        let mut stats_with_goal = MatchPlayerStatistics::new();
        stats_with_goal.add_goal(10, false);
        assert!(!stats_with_goal.is_empty());

        let mut stats_with_assist = MatchPlayerStatistics::new();
        stats_with_assist.add_assist(20);
        assert!(!stats_with_assist.is_empty());
    }
}
