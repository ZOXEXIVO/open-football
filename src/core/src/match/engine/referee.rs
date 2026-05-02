/// Referee profile and foul-call/card/advantage probability helpers.
///
/// Pure scoring; no RNG. Callers fold these into their own random rolls.
/// Inputs that come from match state (crowd, derby, match_temperature) are
/// supplied per-call so the referee profile itself stays stable across the match.

use crate::r#match::engine::environment::MatchEnvironment;

#[derive(Debug, Clone, Copy)]
pub struct RefereeProfile {
    /// 0..1 — how strict on contact in general.
    pub strictness: f32,
    /// 0..1 — willingness to let things go (paired against strictness).
    pub leniency: f32,
    /// 0..1 — how trigger-happy with cards once a foul is given.
    pub card_happiness: f32,
    /// 0..1 — base ability to spot fouls (lower = more missed calls).
    pub foul_detection: f32,
    /// 0..1 — patience for advantage (higher = longer window before whistle).
    pub advantage_patience: f32,
    /// 0..1 — how readily contact in the box becomes a penalty.
    pub penalty_strictness: f32,
    /// -0.08..+0.08 — nudge toward home (positive) or away team. Crowd
    /// intensity scales the *applied* magnitude.
    pub home_bias: f32,
}

impl Default for RefereeProfile {
    fn default() -> Self {
        RefereeProfile {
            strictness: 0.52,
            leniency: 0.48,
            card_happiness: 0.50,
            foul_detection: 0.58,
            advantage_patience: 0.55,
            penalty_strictness: 0.50,
            home_bias: 0.02,
        }
    }
}

/// Where on the pitch the contact happened — gates clamp ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactLocation {
    /// Open play, normal contact.
    Normal,
    /// Clearly a foul (e.g. Reckless or Violent severity).
    ClearFoul,
    /// Inside the defending team's penalty area.
    PenaltyBox,
}

#[derive(Debug, Clone, Copy)]
pub struct FoulCallContext {
    /// 0..1 — how severe the contact was (Normal=0.2, Reckless=0.6, Violent=0.9).
    pub contact_severity: f32,
    /// 0..1 — match temperature (recent fouls/cards/incidents on top of derby).
    pub match_temperature: f32,
    /// True if the fouled team is the home team (for bias direction).
    pub fouled_team_is_home: bool,
    pub location: ContactLocation,
}

impl RefereeProfile {
    pub fn clamp_inputs(&mut self) {
        self.strictness = self.strictness.clamp(0.0, 1.0);
        self.leniency = self.leniency.clamp(0.0, 1.0);
        self.card_happiness = self.card_happiness.clamp(0.0, 1.0);
        self.foul_detection = self.foul_detection.clamp(0.0, 1.0);
        self.advantage_patience = self.advantage_patience.clamp(0.0, 1.0);
        self.penalty_strictness = self.penalty_strictness.clamp(0.0, 1.0);
        self.home_bias = self.home_bias.clamp(-0.08, 0.08);
    }

    /// Probability that the referee blows the whistle for this contact.
    /// Returns a value clamped into the band appropriate for the location.
    /// Intended to be called *before* applying advantage; if advantage gets
    /// played, the call still resolves (foul/card recorded later) — this
    /// number only governs whether the whistle goes at all.
    pub fn foul_call_prob(&self, env: &MatchEnvironment, ctx: FoulCallContext) -> f32 {
        let bias_dir = if ctx.fouled_team_is_home { 1.0 } else { -1.0 };
        let crowd_pressure = env.crowd_intensity * env.home_advantage;
        let bias_term = self.home_bias * bias_dir * crowd_pressure;

        let pen_strict_bonus = if ctx.location == ContactLocation::PenaltyBox {
            (self.penalty_strictness - 0.5) * 0.20
        } else {
            0.0
        };

        let raw = 0.18
            + ctx.contact_severity * 0.22
            + self.strictness * 0.12
            + self.foul_detection * 0.10
            - self.leniency * 0.10
            + bias_term
            + ctx.match_temperature * 0.06
            + pen_strict_bonus;

        match ctx.location {
            ContactLocation::Normal => raw.clamp(0.10, 0.55),
            ContactLocation::ClearFoul => raw.clamp(0.70, 0.96),
            ContactLocation::PenaltyBox => raw.clamp(0.18, 0.90),
        }
    }

    /// Multiplier applied to the base card probability for a given foul.
    /// Caller picks the base (yellow vs red) from `FoulSeverity` and scales
    /// it by this. Always >= 0.
    pub fn card_modifier(&self, env: &MatchEnvironment) -> f32 {
        let m = 1.0
            + (self.card_happiness - 0.5) * 0.45
            + env.derby_intensity * 0.15
            + env.match_importance * 0.08;
        m.max(0.0)
    }

    /// How long the referee will let an advantage run, in engine ticks.
    /// 80–180 tick window from `advantage_patience` 0..1.
    pub fn advantage_window_ticks(&self) -> u32 {
        (80.0 + self.advantage_patience * 100.0) as u32
    }

    /// Should the referee play advantage *now*? `attack_value` is the caller's
    /// estimate of the post-foul attack quality (0..1, where 0.45+ is roughly
    /// "controlled possession into useful space").
    pub fn should_play_advantage(
        &self,
        attack_value: f32,
        possession_retained: bool,
        severity: f32,
    ) -> bool {
        // Violent fouls always stop play regardless of advantage.
        if severity >= 0.85 {
            return false;
        }
        possession_retained && attack_value >= 0.45
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::engine::environment::MatchEnvironment;

    fn ctx(severity: f32, location: ContactLocation) -> FoulCallContext {
        FoulCallContext {
            contact_severity: severity,
            match_temperature: 0.2,
            fouled_team_is_home: false,
            location,
        }
    }

    #[test]
    fn strict_referee_calls_more_fouls_than_lenient() {
        let env = MatchEnvironment::default();
        let strict = RefereeProfile {
            strictness: 0.85,
            leniency: 0.15,
            ..Default::default()
        };
        let lenient = RefereeProfile {
            strictness: 0.20,
            leniency: 0.80,
            ..Default::default()
        };
        let c = ctx(0.5, ContactLocation::Normal);
        assert!(strict.foul_call_prob(&env, c) > lenient.foul_call_prob(&env, c));
    }

    #[test]
    fn normal_contact_call_prob_stays_in_band() {
        let env = MatchEnvironment::default();
        let r = RefereeProfile::default();
        for severity in [0.0, 0.3, 0.6, 1.0] {
            let p = r.foul_call_prob(&env, ctx(severity, ContactLocation::Normal));
            assert!((0.10..=0.55).contains(&p), "severity {severity} -> {p}");
        }
    }

    #[test]
    fn clear_foul_call_prob_stays_high() {
        let env = MatchEnvironment::default();
        let r = RefereeProfile::default();
        let p = r.foul_call_prob(&env, ctx(0.9, ContactLocation::ClearFoul));
        assert!((0.70..=0.96).contains(&p));
    }

    #[test]
    fn penalty_box_strictness_increases_call_prob() {
        let env = MatchEnvironment::default();
        let strict = RefereeProfile {
            penalty_strictness: 0.95,
            ..Default::default()
        };
        let soft = RefereeProfile {
            penalty_strictness: 0.10,
            ..Default::default()
        };
        let c = ctx(0.5, ContactLocation::PenaltyBox);
        assert!(strict.foul_call_prob(&env, c) > soft.foul_call_prob(&env, c));
    }

    #[test]
    fn card_modifier_increases_with_card_happy_ref_and_derby() {
        let calm_env = MatchEnvironment::default();
        let derby_env = MatchEnvironment {
            derby_intensity: 1.0,
            match_importance: 1.0,
            ..Default::default()
        };
        let card_happy = RefereeProfile {
            card_happiness: 1.0,
            ..Default::default()
        };
        let baseline = RefereeProfile::default();
        assert!(card_happy.card_modifier(&calm_env) > baseline.card_modifier(&calm_env));
        assert!(baseline.card_modifier(&derby_env) > baseline.card_modifier(&calm_env));
    }

    #[test]
    fn advantage_window_grows_with_patience() {
        let patient = RefereeProfile {
            advantage_patience: 1.0,
            ..Default::default()
        };
        let impatient = RefereeProfile {
            advantage_patience: 0.0,
            ..Default::default()
        };
        assert!(patient.advantage_window_ticks() > impatient.advantage_window_ticks());
        assert!(patient.advantage_window_ticks() <= 180);
        assert!(impatient.advantage_window_ticks() >= 80);
    }

    #[test]
    fn violent_foul_never_gets_advantage() {
        let r = RefereeProfile::default();
        assert!(!r.should_play_advantage(0.9, true, 0.9));
    }

    #[test]
    fn advantage_requires_possession_and_quality() {
        let r = RefereeProfile::default();
        assert!(r.should_play_advantage(0.6, true, 0.3));
        assert!(!r.should_play_advantage(0.6, false, 0.3));
        assert!(!r.should_play_advantage(0.30, true, 0.3));
    }

    #[test]
    fn home_bias_nudges_toward_home_team_with_crowd() {
        let big_crowd = MatchEnvironment {
            crowd_intensity: 1.0,
            home_advantage: 1.0,
            ..Default::default()
        };
        let r = RefereeProfile {
            home_bias: 0.08,
            ..Default::default()
        };
        let home_fouled =
            r.foul_call_prob(&big_crowd, FoulCallContext {
                contact_severity: 0.5,
                match_temperature: 0.0,
                fouled_team_is_home: true,
                location: ContactLocation::Normal,
            });
        let away_fouled =
            r.foul_call_prob(&big_crowd, FoulCallContext {
                contact_severity: 0.5,
                match_temperature: 0.0,
                fouled_team_is_home: false,
                location: ContactLocation::Normal,
            });
        assert!(home_fouled > away_fouled);
    }
}
