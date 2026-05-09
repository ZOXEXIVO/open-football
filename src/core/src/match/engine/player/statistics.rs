use crate::r#match::engine::zones::{MatchZone, ZoneStats};

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
    /// Total miscontrols recorded by the first-touch resolver. The
    /// rating helper consumes this counter with a live coefficient;
    /// the live PRODUCER is intentionally deferred until receiver-state
    /// tracking can distinguish a clean reception from a mishit on
    /// claim. Defaults to zero — rating impact is zero until the
    /// producer lands.
    pub miscontrols: u16,
    /// First-touch resolutions in [HeavyTouchForward, HeavyTouchSideways].
    /// Same deferred-producer status as `miscontrols` — counter is read
    /// by the rating helper, written by the (not-yet-wired) first-touch
    /// resolver.
    pub heavy_touches: u16,
    /// Per-zone counters for zone-aware rating multipliers. Defaults
    /// to zero — engine call-sites that haven't been updated to record
    /// zone metadata still produce sensible (zone-neutral) ratings.
    pub zone_stats: ZoneStats,
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
            zone_stats: ZoneStats::default(),
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

    pub fn own_goals_count(&self) -> u16 {
        self.items
            .iter()
            .filter(|i| i.stat_type == MatchStatisticType::Goal && i.is_auto_goal)
            .count() as u16
    }

    /// Tag a tackle with the zone it occurred in. Caller still
    /// increments `tackles`; this helper only records the zone bucket.
    /// Six-yard and own-box counters are mutually exclusive — the
    /// six-yard band IS inside the own box, but the rating helper
    /// applies six-yard as the *replacement* bonus, not a stack on
    /// top of own-box. Crediting both would double-charge the same
    /// action's location bonus.
    pub fn note_tackle_zone(&mut self, zone: MatchZone) {
        if zone.is_own_six_yard() {
            self.zone_stats.tackles_own_six_yard =
                self.zone_stats.tackles_own_six_yard.saturating_add(1);
        } else if zone.is_own_box() {
            self.zone_stats.tackles_own_box = self.zone_stats.tackles_own_box.saturating_add(1);
        }
        if zone.is_final_third() {
            self.zone_stats.tackles_final_third =
                self.zone_stats.tackles_final_third.saturating_add(1);
        }
    }

    pub fn note_interception_zone(&mut self, zone: MatchZone) {
        if zone.is_own_six_yard() {
            self.zone_stats.interceptions_own_six_yard =
                self.zone_stats.interceptions_own_six_yard.saturating_add(1);
        } else if zone.is_own_box() {
            self.zone_stats.interceptions_own_box =
                self.zone_stats.interceptions_own_box.saturating_add(1);
        }
        if zone.is_middle_third() {
            self.zone_stats.interceptions_middle_third =
                self.zone_stats.interceptions_middle_third.saturating_add(1);
        }
    }

    pub fn note_block_zone(&mut self, zone: MatchZone) {
        if zone.is_own_six_yard() {
            self.zone_stats.blocks_own_six_yard =
                self.zone_stats.blocks_own_six_yard.saturating_add(1);
        } else if zone.is_own_box() {
            self.zone_stats.blocks_own_box = self.zone_stats.blocks_own_box.saturating_add(1);
        }
    }

    pub fn note_clearance_zone(&mut self, zone: MatchZone) {
        if zone.is_own_six_yard() {
            self.zone_stats.clearances_own_six_yard =
                self.zone_stats.clearances_own_six_yard.saturating_add(1);
        } else if zone.is_own_box() {
            self.zone_stats.clearances_own_box =
                self.zone_stats.clearances_own_box.saturating_add(1);
        }
    }

    pub fn note_pressure_won_zone(&mut self, zone: MatchZone) {
        if zone.is_final_third() {
            self.zone_stats.pressures_won_final_third =
                self.zone_stats.pressures_won_final_third.saturating_add(1);
        }
    }

    pub fn note_progressive_pass_into_final_third(&mut self) {
        self.zone_stats.progressive_passes_into_final_third = self
            .zone_stats
            .progressive_passes_into_final_third
            .saturating_add(1);
    }

    pub fn note_progressive_carry_into_final_third(&mut self) {
        self.zone_stats.progressive_carries_into_final_third = self
            .zone_stats
            .progressive_carries_into_final_third
            .saturating_add(1);
    }

    pub fn note_carry_into_box(&mut self) {
        self.zone_stats.carries_into_box = self.zone_stats.carries_into_box.saturating_add(1);
    }

    pub fn note_half_space_pass_into_box(&mut self) {
        self.zone_stats.half_space_passes_into_box =
            self.zone_stats.half_space_passes_into_box.saturating_add(1);
    }

    pub fn note_central_pass_into_box(&mut self) {
        self.zone_stats.central_passes_into_box =
            self.zone_stats.central_passes_into_box.saturating_add(1);
    }

    pub fn note_switch_of_play(&mut self) {
        self.zone_stats.switches_of_play = self.zone_stats.switches_of_play.saturating_add(1);
    }

    pub fn note_dangerous_turnover(&mut self, zone: MatchZone) {
        if zone.is_own_box() {
            self.zone_stats.dangerous_turnovers_own_box = self
                .zone_stats
                .dangerous_turnovers_own_box
                .saturating_add(1);
        } else if zone.is_own_third() {
            self.zone_stats.dangerous_turnovers_own_third = self
                .zone_stats
                .dangerous_turnovers_own_third
                .saturating_add(1);
        }
    }

    pub fn note_error_to_goal_own_box(&mut self) {
        self.zone_stats.errors_to_goal_own_box =
            self.zone_stats.errors_to_goal_own_box.saturating_add(1);
    }

    pub fn note_gk_command_action(&mut self) {
        self.zone_stats.gk_command_actions = self.zone_stats.gk_command_actions.saturating_add(1);
    }

    pub fn note_gk_failed_claim_to_shot(&mut self) {
        self.zone_stats.gk_failed_claims_to_shot =
            self.zone_stats.gk_failed_claims_to_shot.saturating_add(1);
    }

    pub fn note_gk_failed_claim_to_goal(&mut self) {
        self.zone_stats.gk_failed_claims_to_goal =
            self.zone_stats.gk_failed_claims_to_goal.saturating_add(1);
    }

    pub fn note_penalty_foul_conceded(&mut self) {
        self.zone_stats.penalty_fouls_conceded =
            self.zone_stats.penalty_fouls_conceded.saturating_add(1);
    }

    pub fn note_own_third_def_foul(&mut self) {
        self.zone_stats.own_third_def_fouls = self.zone_stats.own_third_def_fouls.saturating_add(1);
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

    #[test]
    fn six_yard_zone_increments_only_six_yard_counter() {
        // Mutual exclusivity: a tackle inside the six-yard band must
        // not also bump the own-box counter (the rating helper applies
        // six-yard as a *replacement* bonus, not a stack on top of
        // own-box).
        let mut stats = MatchPlayerStatistics::new();
        stats.note_tackle_zone(MatchZone::OwnSixYardBox);
        stats.note_interception_zone(MatchZone::OwnSixYardBox);
        stats.note_block_zone(MatchZone::OwnSixYardBox);
        stats.note_clearance_zone(MatchZone::OwnSixYardBox);
        assert_eq!(stats.zone_stats.tackles_own_six_yard, 1);
        assert_eq!(stats.zone_stats.tackles_own_box, 0);
        assert_eq!(stats.zone_stats.interceptions_own_six_yard, 1);
        assert_eq!(stats.zone_stats.interceptions_own_box, 0);
        assert_eq!(stats.zone_stats.blocks_own_six_yard, 1);
        assert_eq!(stats.zone_stats.blocks_own_box, 0);
        assert_eq!(stats.zone_stats.clearances_own_six_yard, 1);
        assert_eq!(stats.zone_stats.clearances_own_box, 0);
    }

    #[test]
    fn own_penalty_area_zone_increments_only_own_box_counter() {
        // The other half of mutual exclusivity: an own-penalty-area
        // (but outside the six-yard) action only credits the own-box
        // counter.
        let mut stats = MatchPlayerStatistics::new();
        stats.note_tackle_zone(MatchZone::OwnPenaltyArea);
        stats.note_interception_zone(MatchZone::OwnPenaltyArea);
        stats.note_block_zone(MatchZone::OwnPenaltyArea);
        stats.note_clearance_zone(MatchZone::OwnPenaltyArea);
        assert_eq!(stats.zone_stats.tackles_own_box, 1);
        assert_eq!(stats.zone_stats.tackles_own_six_yard, 0);
        assert_eq!(stats.zone_stats.interceptions_own_box, 1);
        assert_eq!(stats.zone_stats.interceptions_own_six_yard, 0);
        assert_eq!(stats.zone_stats.blocks_own_box, 1);
        assert_eq!(stats.zone_stats.blocks_own_six_yard, 0);
        assert_eq!(stats.zone_stats.clearances_own_box, 1);
        assert_eq!(stats.zone_stats.clearances_own_six_yard, 0);
    }

    #[test]
    fn final_third_tackle_also_credits_final_third_counter() {
        // Third-band counters are independent of the own-box / six-yard
        // mutual-exclusion rule: a tackle in the final third still
        // bumps `tackles_final_third` regardless.
        let mut stats = MatchPlayerStatistics::new();
        stats.note_tackle_zone(MatchZone::FinalThird);
        assert_eq!(stats.zone_stats.tackles_final_third, 1);
        assert_eq!(stats.zone_stats.tackles_own_box, 0);
        assert_eq!(stats.zone_stats.tackles_own_six_yard, 0);
    }

    #[test]
    fn middle_third_interception_records_middle_third_only() {
        let mut stats = MatchPlayerStatistics::new();
        stats.note_interception_zone(MatchZone::MiddleThird);
        assert_eq!(stats.zone_stats.interceptions_middle_third, 1);
        assert_eq!(stats.zone_stats.interceptions_own_box, 0);
        assert_eq!(stats.zone_stats.interceptions_own_six_yard, 0);
    }

    #[test]
    fn dangerous_turnover_own_box_excludes_own_third() {
        // An own-box giveaway only bumps the own-box counter, not the
        // own-third counter — the rating helper applies them as
        // independent layers.
        let mut stats = MatchPlayerStatistics::new();
        stats.note_dangerous_turnover(MatchZone::OwnPenaltyArea);
        assert_eq!(stats.zone_stats.dangerous_turnovers_own_box, 1);
        assert_eq!(stats.zone_stats.dangerous_turnovers_own_third, 0);
    }

    #[test]
    fn note_error_to_goal_own_box_increments_counter() {
        let mut stats = MatchPlayerStatistics::new();
        stats.note_error_to_goal_own_box();
        stats.note_error_to_goal_own_box();
        assert_eq!(stats.zone_stats.errors_to_goal_own_box, 2);
    }

    #[test]
    fn lateral_lane_helpers_increment_counters() {
        let mut stats = MatchPlayerStatistics::new();
        stats.note_half_space_pass_into_box();
        stats.note_half_space_pass_into_box();
        stats.note_central_pass_into_box();
        stats.note_switch_of_play();
        stats.note_switch_of_play();
        stats.note_switch_of_play();
        assert_eq!(stats.zone_stats.half_space_passes_into_box, 2);
        assert_eq!(stats.zone_stats.central_passes_into_box, 1);
        assert_eq!(stats.zone_stats.switches_of_play, 3);
    }

    #[test]
    fn pressure_helpers_track_separately() {
        let mut stats = MatchPlayerStatistics::new();
        stats.add_pressure();
        stats.add_pressure();
        stats.add_pressure();
        stats.add_successful_pressure();
        assert_eq!(stats.pressures, 3);
        assert_eq!(stats.successful_pressures, 1);
    }

    #[test]
    fn dribble_helpers_track_attempts_and_successes() {
        let mut stats = MatchPlayerStatistics::new();
        stats.add_successful_dribble();
        stats.add_successful_dribble();
        stats.add_failed_dribble();
        assert_eq!(stats.successful_dribbles, 2);
        assert_eq!(stats.attempted_dribbles, 3);
    }

    #[test]
    fn final_third_pressure_won_tags_zone_counter() {
        // Successful pressure won in the final third must populate
        // `pressures_won_final_third`. The interception handler reads
        // the player's zone and calls `note_pressure_won_zone(zone)`
        // — the rating helper's PRESSURE_FINAL_THIRD_BONUS depends on
        // this counter.
        let mut stats = MatchPlayerStatistics::new();
        stats.add_pressure();
        stats.add_successful_pressure();
        stats.note_pressure_won_zone(MatchZone::FinalThird);
        assert_eq!(stats.zone_stats.pressures_won_final_third, 1);
        // Pressures won in other zones don't credit the final-third
        // counter (the bonus is final-third-specific).
        stats.note_pressure_won_zone(MatchZone::MiddleThird);
        stats.note_pressure_won_zone(MatchZone::DefensiveThird);
        assert_eq!(stats.zone_stats.pressures_won_final_third, 1);
    }

    #[test]
    fn pressure_invariant_holds_when_only_credited_pressers_promoted() {
        // Contract enforced by `credit_pressures_on_pass` after the
        // cooldown-promotion fix: every `add_successful_pressure` is
        // preceded by an `add_pressure` on the same player. The
        // helper's responsibility is to populate `pressers_at_pass`
        // ONLY with players who got `add_pressure()`. Modeling that
        // contract here lets a future regression that bumps
        // successful_pressures from a non-credited cooldown shadow
        // surface as a coherence assertion failure.
        let mut stats = MatchPlayerStatistics::new();
        // Pass A: opponent within radius, no cooldown → credited.
        stats.add_pressure();
        // Pass A intercepted → promotion.
        stats.add_successful_pressure();
        // Pass B: same opponent within radius, but inside cooldown
        // window → NOT credited. With the fix in place, the helper
        // does NOT add this player to `pressers_at_pass`, so a Pass B
        // interception cannot promote them. Simulate that by simply
        // not calling `add_successful_pressure` here.
        // Pass C: cooldown elapsed, opponent credited again.
        stats.add_pressure();
        stats.add_successful_pressure();
        assert_eq!(stats.pressures, 2);
        assert_eq!(stats.successful_pressures, 2);
        // Coach press-success rate would be 1.0 — at the bound but
        // not over. Pre-fix behaviour could promote the cooldown
        // shadow on Pass B, producing successful_pressures=3 with
        // pressures=2 — ratio 1.5, which is what the fix prevents.
        assert!(
            stats.successful_pressures <= stats.pressures,
            "successful_pressures ({}) must not exceed pressures ({}) — \
             coach press-success rate would exceed 1.0",
            stats.successful_pressures,
            stats.pressures
        );
    }

    #[test]
    fn block_zone_credit_independent_of_interception() {
        // After the block-vs-intercept separation: block credit
        // travels through `add_block` + `note_block_zone(zone)` and
        // does NOT require `interceptions` to bump. A loose / corner
        // / safe / unlucky deflection now credits ONLY the block —
        // verify the zone bookkeeping remains independent.
        let mut stats = MatchPlayerStatistics::new();
        stats.add_block();
        stats.note_block_zone(MatchZone::OwnPenaltyArea);
        assert_eq!(stats.blocks, 1);
        assert_eq!(stats.zone_stats.blocks_own_box, 1);
        assert_eq!(
            stats.interceptions, 0,
            "deflection block must not inflate interceptions"
        );
    }

    #[test]
    fn controlled_block_credits_both_block_and_interception() {
        // The controlled-block path keeps emitting both `Blocked` and
        // `Intercepted` so the defender wins possession AND collects
        // a block stat. This test pins the dual-credit shape.
        let mut stats = MatchPlayerStatistics::new();
        stats.add_block();
        stats.note_block_zone(MatchZone::OwnPenaltyArea);
        // Controlled branch additionally credits an interception.
        stats.interceptions += 1;
        stats.note_interception_zone(MatchZone::OwnPenaltyArea);
        assert_eq!(stats.blocks, 1);
        assert_eq!(stats.interceptions, 1);
        assert_eq!(stats.zone_stats.blocks_own_box, 1);
        assert_eq!(stats.zone_stats.interceptions_own_box, 1);
    }
}
