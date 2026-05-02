//! Game-management helpers: professional fouls, time-wasting,
//! stoppage-time accounting, home-advantage modulation.
//!
//! Pure helpers; no engine state mutation. Returned values are
//! probabilities or millisecond deltas the caller folds into the
//! existing dispatcher / referee logic.

use crate::r#match::engine::environment::MatchEnvironment;

/// Estimate of the post-foul attacking threat. Caller computes from the
/// match state; high values mean the foul prevented a likely chance.
#[derive(Debug, Clone, Copy)]
pub struct CounterAttackThreat {
    /// True if the carrier had a clear lane toward the goal.
    pub lane_open: bool,
    /// Defenders behind the ball, from the foul location.
    pub defenders_behind_ball: u8,
    /// Distance from foul spot to the goal being attacked, in field units.
    pub distance_to_goal_units: f32,
    /// Closest defender within fouling range (field units).
    pub fouler_distance_units: f32,
}

impl CounterAttackThreat {
    pub fn is_dogso_zone(&self) -> bool {
        self.distance_to_goal_units < 260.0
    }
}

/// Probability the defender commits a tactical (professional) foul to
/// stop a counter. Returns 0 outside the trigger conditions.
///
/// Trigger: `lane_open && defenders_behind_ball < 2 && distance < 260u
/// && fouler_distance < 8u`.
///
/// Probability:
///   aggression*0.14 + decisions*0.08 + team_desperation*0.10
///   + match_minute_late*0.06 - sportsmanship*0.10 - already_yellow*0.12
pub fn professional_foul_prob(
    threat: CounterAttackThreat,
    aggression_0_20: f32,
    decisions_0_20: f32,
    sportsmanship_0_20: f32,
    team_desperation_0_1: f32,
    match_minute: u32,
    already_yellow: bool,
) -> f32 {
    if !(threat.lane_open
        && threat.defenders_behind_ball < 2
        && threat.distance_to_goal_units < 260.0
        && threat.fouler_distance_units < 8.0)
    {
        return 0.0;
    }
    let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
    let late = if match_minute >= 75 { 1.0 } else if match_minute >= 60 { 0.5 } else { 0.0 };
    let yellow_bias = if already_yellow { 0.12 } else { 0.0 };

    let raw = n(aggression_0_20) * 0.14
        + n(decisions_0_20) * 0.08
        + team_desperation_0_1.clamp(0.0, 1.0) * 0.10
        + late * 0.06
        - n(sportsmanship_0_20) * 0.10
        - yellow_bias;
    raw.clamp(0.0, 0.65)
}

/// Card outcome for a professional foul.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfessionalFoulCard {
    Yellow,
    Red,
}

/// Decide whether the professional foul is yellow or red — DOGSO close to
/// goal can flip a small fraction to red. Returned as the *probability*
/// of red; caller rolls.
pub fn professional_foul_red_prob(threat: CounterAttackThreat) -> f32 {
    if !threat.is_dogso_zone() {
        return 0.03;
    }
    // Closer to goal + clear lane + no covering defender → DOGSO red.
    if threat.lane_open && threat.defenders_behind_ball == 0 {
        if threat.distance_to_goal_units < 100.0 {
            return 0.12;
        }
        return 0.07;
    }
    0.04
}

/// Compute how long a leading team can drag a restart. Positive values
/// in milliseconds added to the natural stoppage. Returns 0 outside the
/// 75th-minute leading-team window.
pub fn time_wasting_delay_ms(
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
        TimeWastingRestart::ThrowIn => 9_000.0,        // 5–18s window centre
        TimeWastingRestart::GoalKick => 14_000.0,      // 8–24s
        TimeWastingRestart::Substitution => 28_000.0,  // 20–35s
        TimeWastingRestart::FreeKick => 6_000.0,
    };
    (base_ms * scale) as u64
}

#[derive(Debug, Clone, Copy)]
pub enum TimeWastingRestart {
    ThrowIn,
    GoalKick,
    Substitution,
    FreeKick,
}

/// Probability of a yellow card for time-wasting given the cumulative
/// delay so far this period and the referee's strictness.
pub fn time_wasting_yellow_prob(
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

/// Stoppage-time additions for various match incidents. All in
/// milliseconds; spec reference values:
///   goal: 30–55s
///   substitution: 25–40s
///   injury: 45–120s
#[derive(Debug, Clone, Copy)]
pub enum StoppageEvent {
    Goal,
    Substitution,
    InjuryShort,
    InjuryLong,
    TimeWastingFoul,
}

pub fn stoppage_for(event: StoppageEvent) -> u64 {
    match event {
        StoppageEvent::Goal => 42_000,
        StoppageEvent::Substitution => 32_000,
        StoppageEvent::InjuryShort => 60_000,
        StoppageEvent::InjuryLong => 105_000,
        StoppageEvent::TimeWastingFoul => 15_000,
    }
}

/// Home-advantage modulation. Returns a struct of small deltas the
/// caller applies to the relevant systems. Pure function of environment
/// + minute. Targets per spec:
///   home win 42–48% / draw 23–30% / away 27–34% in equal teams.
#[derive(Debug, Clone, Copy, Default)]
pub struct HomeAdvantageDeltas {
    pub home_confidence_bonus: f32,
    pub away_nervousness_bonus: f32,
    pub home_press_intensity_bonus: f32,
    pub away_communication_penalty: f32,
    pub referee_marginal_call_home_bias: f32,
}

pub fn home_advantage_deltas(env: &MatchEnvironment) -> HomeAdvantageDeltas {
    let crowd = env.crowd_intensity.clamp(0.0, 1.0);
    let ha = env.home_advantage.clamp(0.0, 1.0);
    let derby = env.derby_intensity.clamp(0.0, 1.0);
    let big_crowd = (crowd - 0.5).max(0.0); // 0..0.5

    HomeAdvantageDeltas {
        home_confidence_bonus: 0.04 * ha,
        away_nervousness_bonus: 0.03 * (ha + derby).min(1.0),
        home_press_intensity_bonus: 0.03 * ha,
        away_communication_penalty: if crowd > 0.75 { -0.02 * ha } else { 0.0 },
        referee_marginal_call_home_bias: 0.04 * big_crowd * ha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_threat() -> CounterAttackThreat {
        CounterAttackThreat {
            lane_open: true,
            defenders_behind_ball: 1,
            distance_to_goal_units: 220.0,
            fouler_distance_units: 5.0,
        }
    }

    #[test]
    fn pro_foul_zero_outside_trigger() {
        let mut t = open_threat();
        t.lane_open = false;
        let p = professional_foul_prob(t, 14.0, 12.0, 12.0, 0.6, 80, false);
        assert_eq!(p, 0.0);
    }

    #[test]
    fn pro_foul_late_match_more_likely() {
        let early = professional_foul_prob(open_threat(), 14.0, 12.0, 12.0, 0.6, 30, false);
        let late = professional_foul_prob(open_threat(), 14.0, 12.0, 12.0, 0.6, 85, false);
        assert!(late > early);
    }

    #[test]
    fn pro_foul_already_yellow_reduces_prob() {
        let no_card = professional_foul_prob(open_threat(), 14.0, 12.0, 12.0, 0.5, 80, false);
        let on_yellow = professional_foul_prob(open_threat(), 14.0, 12.0, 12.0, 0.5, 80, true);
        assert!(on_yellow < no_card);
    }

    #[test]
    fn dogso_close_to_goal_red_prob_higher() {
        let mut t = open_threat();
        t.distance_to_goal_units = 80.0;
        t.defenders_behind_ball = 0;
        let close = professional_foul_red_prob(t);

        let mut far = open_threat();
        far.distance_to_goal_units = 250.0;
        let far_p = professional_foul_red_prob(far);

        assert!(close > far_p);
    }

    #[test]
    fn time_wasting_zero_when_not_leading() {
        let zero =
            time_wasting_delay_ms(0, 80, TimeWastingRestart::ThrowIn, 12.0);
        assert_eq!(zero, 0);
        let losing = time_wasting_delay_ms(-1, 85, TimeWastingRestart::ThrowIn, 12.0);
        assert_eq!(losing, 0);
    }

    #[test]
    fn time_wasting_zero_before_75th() {
        let p = time_wasting_delay_ms(1, 60, TimeWastingRestart::ThrowIn, 12.0);
        assert_eq!(p, 0);
    }

    #[test]
    fn time_wasting_substitution_longest() {
        let throw = time_wasting_delay_ms(1, 85, TimeWastingRestart::ThrowIn, 12.0);
        let goal_kick = time_wasting_delay_ms(1, 85, TimeWastingRestart::GoalKick, 12.0);
        let sub = time_wasting_delay_ms(1, 85, TimeWastingRestart::Substitution, 12.0);
        assert!(sub > goal_kick);
        assert!(goal_kick > throw);
    }

    #[test]
    fn time_wasting_yellow_kicks_in_after_threshold() {
        // Below threshold — no card.
        assert_eq!(time_wasting_yellow_prob(20_000, 0.8, 0), 0.0);
        // Above threshold — non-zero (clamped at the 0.05 floor).
        assert!(time_wasting_yellow_prob(60_000, 0.8, 1) > 0.0);
    }

    #[test]
    fn time_wasting_yellow_grows_with_strictness() {
        let lenient = time_wasting_yellow_prob(60_000, 0.2, 0);
        let strict = time_wasting_yellow_prob(60_000, 0.95, 0);
        assert!(strict > lenient);
    }

    #[test]
    fn stoppage_event_mapping() {
        assert!(stoppage_for(StoppageEvent::InjuryLong) > stoppage_for(StoppageEvent::InjuryShort));
        assert!(
            stoppage_for(StoppageEvent::Substitution) < stoppage_for(StoppageEvent::InjuryShort)
        );
    }

    #[test]
    fn home_advantage_zero_at_neutral_environment() {
        let env = MatchEnvironment {
            home_advantage: 0.0,
            crowd_intensity: 0.0,
            ..Default::default()
        };
        let d = home_advantage_deltas(&env);
        assert_eq!(d.home_confidence_bonus, 0.0);
        assert_eq!(d.referee_marginal_call_home_bias, 0.0);
    }

    #[test]
    fn home_advantage_grows_with_crowd() {
        let small = MatchEnvironment {
            home_advantage: 1.0,
            crowd_intensity: 0.5,
            ..Default::default()
        };
        let big = MatchEnvironment {
            home_advantage: 1.0,
            crowd_intensity: 1.0,
            ..Default::default()
        };
        let small_d = home_advantage_deltas(&small);
        let big_d = home_advantage_deltas(&big);
        assert!(big_d.referee_marginal_call_home_bias > small_d.referee_marginal_call_home_bias);
        assert!(big_d.away_communication_penalty < 0.0); // big crowd activates penalty
        assert_eq!(small_d.away_communication_penalty, 0.0);
    }

    #[test]
    fn home_advantage_modest_in_size() {
        // Sanity: even max-everything environment shouldn't produce
        // arcade-tier deltas.
        let env = MatchEnvironment {
            home_advantage: 1.0,
            crowd_intensity: 1.0,
            derby_intensity: 1.0,
            ..Default::default()
        };
        let d = home_advantage_deltas(&env);
        assert!(d.home_confidence_bonus <= 0.05);
        assert!(d.away_nervousness_bonus <= 0.04);
        assert!(d.home_press_intensity_bonus <= 0.04);
        assert!(d.referee_marginal_call_home_bias <= 0.03);
    }
}
