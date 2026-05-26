//! Home-advantage modulation: small deltas the caller applies to the
//! relevant systems based on crowd, venue, and derby intensity.
//!
//! Pure helpers; no engine state mutation.

use crate::r#match::engine::environment::MatchEnvironment;

/// Home-advantage modulation. A struct of small deltas the caller applies
/// to the relevant systems. Targets per spec:
///   home win 42–48% / draw 23–30% / away 27–34% in equal teams.
#[derive(Debug, Clone, Copy, Default)]
pub struct HomeAdvantageDeltas {
    pub home_confidence_bonus: f32,
    pub away_nervousness_bonus: f32,
    pub home_press_intensity_bonus: f32,
    pub away_communication_penalty: f32,
    pub referee_marginal_call_home_bias: f32,
}

/// Home-advantage decisions, grouped as associated functions.
pub struct HomeAdvantage;

impl HomeAdvantage {
    /// Pure function of environment + minute producing the per-system deltas.
    pub fn deltas(env: &MatchEnvironment) -> HomeAdvantageDeltas {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_advantage_zero_at_neutral_environment() {
        let env = MatchEnvironment {
            home_advantage: 0.0,
            crowd_intensity: 0.0,
            ..Default::default()
        };
        let d = HomeAdvantage::deltas(&env);
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
        let small_d = HomeAdvantage::deltas(&small);
        let big_d = HomeAdvantage::deltas(&big);
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
        let d = HomeAdvantage::deltas(&env);
        assert!(d.home_confidence_bonus <= 0.05);
        assert!(d.away_nervousness_bonus <= 0.04);
        assert!(d.home_press_intensity_bonus <= 0.04);
        assert!(d.referee_marginal_call_home_bias <= 0.03);
    }
}
