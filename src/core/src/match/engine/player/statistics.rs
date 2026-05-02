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

    // ── Modern football stats (Section 7) ───────────────────────────────
    /// Carries that move the ball ≥ 25u toward opp goal outside final third
    /// or ≥ 12u inside final third.
    pub progressive_carries: u16,
    /// Cumulative pitch-units carried under control. Useful for "metres
    /// progressed" rate calculations.
    pub carry_distance: u32,
    pub successful_dribbles: u16,
    pub attempted_dribbles: u16,
    /// Completed pass directly followed by a teammate's shot.
    pub key_passes: u16,
    /// Pass that progresses the ball significantly toward opp goal.
    pub progressive_passes: u16,
    /// Completed passes that finish inside the opposition penalty box.
    pub passes_into_box: u16,
    pub crosses_attempted: u16,
    pub crosses_completed: u16,
    /// Times the player applied close-range pressure on an opponent in
    /// possession (within 8u for sustained ticks).
    pub pressures: u16,
    /// Pressures that produced a turnover, miscontrol, or panic-back-pass
    /// from the opponent within the response window.
    pub successful_pressures: u16,
    pub blocks: u16,
    pub clearances: u16,
    /// Errors (miscontrol, bad pass, lost tackle) that an opponent
    /// converted into a shot within the response window.
    pub errors_leading_to_shot: u16,
    /// Subset of `errors_leading_to_shot` that resulted in a goal.
    pub errors_leading_to_goal: u16,
    /// Sum of xG of all shots in possessions this player participated in
    /// — i.e., touched the ball during build-up. Used for chain rating.
    pub xg_chain: f32,
    /// Sum of xG of shots in possessions this player participated in,
    /// excluding shots and assists themselves (pure build-up).
    pub xg_buildup: f32,
    /// (GK) Post-shot xG faced minus goals conceded — positive = above
    /// expectation save performance.
    pub xg_prevented: f32,
    /// Total miscontrols / heavy touches recorded by the first-touch
    /// resolver.
    pub miscontrols: u16,
    /// First-touch resolutions in [HeavyTouchForward, HeavyTouchSideways].
    pub heavy_touches: u16,
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
            progressive_carries: 0,
            carry_distance: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            key_passes: 0,
            progressive_passes: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            pressures: 0,
            successful_pressures: 0,
            blocks: 0,
            clearances: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            xg_prevented: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
        }
    }

    /// Tally a successful dribble (1v1 win).
    pub fn add_successful_dribble(&mut self) {
        self.attempted_dribbles = self.attempted_dribbles.saturating_add(1);
        self.successful_dribbles = self.successful_dribbles.saturating_add(1);
    }

    /// Tally a failed dribble (1v1 loss).
    pub fn add_failed_dribble(&mut self) {
        self.attempted_dribbles = self.attempted_dribbles.saturating_add(1);
    }

    pub fn add_key_pass(&mut self) {
        self.key_passes = self.key_passes.saturating_add(1);
    }

    pub fn add_progressive_pass(&mut self) {
        self.progressive_passes = self.progressive_passes.saturating_add(1);
    }

    pub fn add_progressive_carry(&mut self, distance_units: u32) {
        self.progressive_carries = self.progressive_carries.saturating_add(1);
        self.carry_distance = self.carry_distance.saturating_add(distance_units);
    }

    pub fn add_pressure(&mut self) {
        self.pressures = self.pressures.saturating_add(1);
    }

    pub fn add_successful_pressure(&mut self) {
        self.successful_pressures = self.successful_pressures.saturating_add(1);
    }

    pub fn add_block(&mut self) {
        self.blocks = self.blocks.saturating_add(1);
    }

    pub fn add_clearance(&mut self) {
        self.clearances = self.clearances.saturating_add(1);
    }

    pub fn add_error_leading_to_shot(&mut self) {
        self.errors_leading_to_shot = self.errors_leading_to_shot.saturating_add(1);
    }

    pub fn add_error_leading_to_goal(&mut self) {
        self.errors_leading_to_goal = self.errors_leading_to_goal.saturating_add(1);
    }

    pub fn add_miscontrol(&mut self) {
        self.miscontrols = self.miscontrols.saturating_add(1);
    }

    pub fn add_heavy_touch(&mut self) {
        self.heavy_touches = self.heavy_touches.saturating_add(1);
    }

    pub fn add_cross_attempt(&mut self) {
        self.crosses_attempted = self.crosses_attempted.saturating_add(1);
    }

    pub fn add_cross_completed(&mut self) {
        self.crosses_completed = self.crosses_completed.saturating_add(1);
    }

    pub fn add_pass_into_box(&mut self) {
        self.passes_into_box = self.passes_into_box.saturating_add(1);
    }

    pub fn record_xg_chain(&mut self, xg: f32) {
        self.xg_chain += xg;
    }

    pub fn record_xg_buildup(&mut self, xg: f32) {
        self.xg_buildup += xg;
    }

    /// Record xG-prevented contribution: positive if a shot expected to
    /// be a goal (post-shot xG) was saved; negative otherwise.
    pub fn record_xg_prevented(&mut self, delta: f32) {
        self.xg_prevented += delta;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn goals_count(&self) -> u16 {
        self.items
            .iter()
            .filter(|i| i.stat_type == MatchStatisticType::Goal && !i.is_auto_goal)
            .count() as u16
    }

    pub fn assists_count(&self) -> u16 {
        self.items
            .iter()
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
        self.items
            .iter()
            .filter(|i| i.stat_type == MatchStatisticType::YellowCard)
            .count() as u16
    }

    pub fn red_cards_count(&self) -> u16 {
        self.items
            .iter()
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
