#[derive(Debug, Clone)]
pub struct MatchPlayerStatistics {
    pub items: Vec<MatchPlayerStatisticsItem>,
    pub passes_attempted: u16,
    pub passes_completed: u16,
    pub tackles: u16,
    pub interceptions: u16,
    /// Shots stopped by this player (catches, dive-parries, punches,
    /// blocks). For goalkeepers the rating uses both `saves` and the
    /// derived save percentage (`saves / max(shots_faced, 1)`).
    pub saves: u16,
    /// Shots-on-target this player had to deal with — saved + conceded.
    /// Always incremented by the same code paths that increment `saves`,
    /// plus once per goal scored against the GK's team (so `shots_faced -
    /// saves` equals `goals_conceded` to a first approximation).
    pub shots_faced: u16,
    pub offsides: u16,
}

impl MatchPlayerStatistics {
    pub fn new() -> Self {
        MatchPlayerStatistics {
            items: Vec::with_capacity(5),
            passes_attempted: 0,
            passes_completed: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            offsides: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn goals_count(&self) -> u16 {
        self.items.iter()
            .filter(|i| i.stat_type == MatchStatisticType::Goal && !i.is_auto_goal)
            .count() as u16
    }

    pub fn assists_count(&self) -> u16 {
        self.items.iter()
            .filter(|i| i.stat_type == MatchStatisticType::Assist)
            .count() as u16
    }

    pub fn add_goal(&mut self, match_second: u64, is_auto_goal: bool) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::Goal,
            match_second,
            is_auto_goal,
        })
    }

    pub fn add_assist(&mut self, match_second: u64) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::Assist,
            match_second,
            is_auto_goal: false,
        })
    }

    pub fn add_foul(&mut self, match_second: u64) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::Foul,
            match_second,
            is_auto_goal: false,
        })
    }

    pub fn add_yellow_card(&mut self, match_second: u64) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::YellowCard,
            match_second,
            is_auto_goal: false,
        })
    }

    pub fn add_red_card(&mut self, match_second: u64) {
        self.items.push(MatchPlayerStatisticsItem {
            stat_type: MatchStatisticType::RedCard,
            match_second,
            is_auto_goal: false,
        })
    }

    pub fn yellow_cards_count(&self) -> u16 {
        self.items.iter()
            .filter(|i| i.stat_type == MatchStatisticType::YellowCard)
            .count() as u16
    }

    pub fn red_cards_count(&self) -> u16 {
        self.items.iter()
            .filter(|i| i.stat_type == MatchStatisticType::RedCard)
            .count() as u16
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
    YellowCard,
    RedCard,
    Foul,
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
