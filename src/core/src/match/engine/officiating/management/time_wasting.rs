//! Time-wasting by a leading team late in the match: how long a restart
//! gets dragged out, and the booking risk that accumulates.
//!
//! Pure helpers; no engine state mutation. Returned values are
//! millisecond deltas / probabilities the caller folds into the existing
//! stoppage-time and referee logic.

/// The kind of restart being dragged out.
#[derive(Debug, Clone, Copy)]
pub enum TimeWastingRestart {
    ThrowIn,
    GoalKick,
    Substitution,
    FreeKick,
}

/// Time-wasting decisions, grouped as associated functions.
pub struct TimeWasting;

impl TimeWasting {
    /// Compute how long a leading team can drag a restart. Positive values
    /// in milliseconds added to the natural stoppage. Returns 0 outside the
    /// 75th-minute leading-team window.
    pub fn delay_ms(
        score_diff: i32,
        match_minute: u32,
        restart_kind: TimeWastingRestart,
        team_aggression_0_20: f32,
    ) -> u64 {
        if score_diff <= 0 || match_minute < 75 {
            return 0;
        }
        let aggressiveness = (team_aggression_0_20 / 20.0).clamp(0.0, 1.0);
        let scale = 0.75 + (1.0 - aggressiveness) * 0.50; // calmer/older players waste more
        let base_ms = match restart_kind {
            TimeWastingRestart::ThrowIn => 9_000.0, // 5–18s window centre
            TimeWastingRestart::GoalKick => 14_000.0, // 8–24s
            TimeWastingRestart::Substitution => 28_000.0, // 20–35s
            TimeWastingRestart::FreeKick => 6_000.0,
        };
        (base_ms * scale) as u64
    }

    /// Probability of a yellow card for time-wasting given the cumulative
    /// delay so far this period and the referee's strictness.
    pub fn yellow_prob(
        cumulative_delay_ms: u64,
        referee_strictness_0_1: f32,
        repeated_offences: u32,
    ) -> f32 {
        // First warning at 45_000ms cumulative (per spec).
        if cumulative_delay_ms < 45_000 {
            return 0.0;
        }
        let strict = referee_strictness_0_1.clamp(0.0, 1.0);
        let repeats = (repeated_offences as f32).min(5.0);
        (strict * 0.18 + repeats * 0.12).clamp(0.05, 0.55)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_wasting_zero_when_not_leading() {
        let zero = TimeWasting::delay_ms(0, 80, TimeWastingRestart::ThrowIn, 12.0);
        assert_eq!(zero, 0);
        let losing = TimeWasting::delay_ms(-1, 85, TimeWastingRestart::ThrowIn, 12.0);
        assert_eq!(losing, 0);
    }

    #[test]
    fn time_wasting_zero_before_75th() {
        let p = TimeWasting::delay_ms(1, 60, TimeWastingRestart::ThrowIn, 12.0);
        assert_eq!(p, 0);
    }

    #[test]
    fn time_wasting_substitution_longest() {
        let throw = TimeWasting::delay_ms(1, 85, TimeWastingRestart::ThrowIn, 12.0);
        let goal_kick = TimeWasting::delay_ms(1, 85, TimeWastingRestart::GoalKick, 12.0);
        let sub = TimeWasting::delay_ms(1, 85, TimeWastingRestart::Substitution, 12.0);
        assert!(sub > goal_kick);
        assert!(goal_kick > throw);
    }

    #[test]
    fn time_wasting_yellow_kicks_in_after_threshold() {
        // Below threshold — no card.
        assert_eq!(TimeWasting::yellow_prob(20_000, 0.8, 0), 0.0);
        // Above threshold — non-zero (clamped at the 0.05 floor).
        assert!(TimeWasting::yellow_prob(60_000, 0.8, 1) > 0.0);
    }

    #[test]
    fn time_wasting_yellow_grows_with_strictness() {
        let lenient = TimeWasting::yellow_prob(60_000, 0.2, 0);
        let strict = TimeWasting::yellow_prob(60_000, 0.95, 0);
        assert!(strict > lenient);
    }
}
