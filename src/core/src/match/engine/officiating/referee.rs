/// Referee profile and foul-call/card/advantage probability helpers.
///
/// Pure scoring; no RNG. Callers fold these into their own random rolls.
/// Inputs that come from match state (crowd, derby, match_temperature) are
/// supplied per-call so the referee profile itself stays stable across the match.
use crate::r#match::MatchContext;
use crate::r#match::engine::environment::MatchEnvironment;
use crate::r#match::engine::flow::context::rng::MatchRng;
use crate::r#match::engine::result::DeadTime;

#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// A man brought down on purpose to stop a break: in plain sight, and
    /// almost never missed.
    pub deliberate: bool,
}

impl RefereeProfile {
    /// How long a booking holds the restart: the whistle, the name, the
    /// card, and for a red the walk off.
    pub const BOOKING_PAUSE_MS: u64 = 20_000;
    pub const SENDING_OFF_PAUSE_MS: u64 = 40_000;

    /// The share of a dead second this referee adds back as stoppage time.
    ///
    /// Law 7 lists what is allowed for — substitutions, injuries, time
    /// wasting, disciplinary sanctions, celebrations — and the ordinary
    /// wait for a throw-in or a goal kick is not on it. How fully the list
    /// is honoured is the referee's own strictness.
    pub fn add_back(&self, why: DeadTime) -> f32 {
        match why {
            DeadTime::Restart => 0.0,
            DeadTime::Celebration
            | DeadTime::Substitution
            | DeadTime::Treatment
            | DeadTime::Booking
            | DeadTime::Delay => 0.8 + 0.2 * self.strictness,
        }
    }

    /// How far one referee's temperament sits from the baseline, either
    /// way, on each of the 0..1 traits.
    const SPREAD: f32 = 0.12;
    /// …and how far his lean toward the home side does.
    const BIAS_SPREAD: f32 = 0.02;
    /// Keeps the referee's draw apart from the weather's on the same seed.
    const STREAM: u64 = 0x5EF_E4EE;

    /// The referee appointed to a fixture: `baseline`'s temperament with
    /// his own lean on each trait, drawn from `seed` so the same fixture
    /// always gets the same man.
    pub fn draw(seed: u64, baseline: RefereeProfile) -> Self {
        let rng = MatchRng::from_seed(seed ^ Self::STREAM);
        let mut referee = RefereeProfile {
            strictness: rng.jitter(baseline.strictness, Self::SPREAD),
            leniency: rng.jitter(baseline.leniency, Self::SPREAD),
            card_happiness: rng.jitter(baseline.card_happiness, Self::SPREAD),
            foul_detection: rng.jitter(baseline.foul_detection, Self::SPREAD),
            advantage_patience: rng.jitter(baseline.advantage_patience, Self::SPREAD),
            penalty_strictness: rng.jitter(baseline.penalty_strictness, Self::SPREAD),
            home_bias: rng.jitter(baseline.home_bias, Self::BIAS_SPREAD),
        };
        referee.clamp_inputs();
        referee
    }

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
        // A/B control — see `MatchContext::home_flat`. The referee is the
        // third declared home-advantage channel and has to go quiet with
        // the other two, or the decomposition is not one.
        let crowd_pressure = if MatchContext::home_flat() {
            0.0
        } else {
            env.crowd_intensity * env.home_advantage
        };
        let bias_term = self.home_bias * bias_dir * crowd_pressure;

        let pen_strict_bonus = if ctx.location == ContactLocation::PenaltyBox {
            (self.penalty_strictness - 0.5) * 0.20
        } else {
            0.0
        };

        // Base 0.18 → 0.40 (2026-06 discipline recalibration): by the
        // time this gate runs, the duel model has already decided that
        // genuine foul contact OCCURRED — the referee question is
        // marginal-call judgment, not detection from scratch. At 0.18
        // the raw score for normal contact landed at ~0.28-0.35, so
        // ~⅔ of real fouls were waved away and the engine ran at ~7
        // fouls/team vs the real ~12 (starving free kicks and the
        // persistent-infringement card pipeline). At 0.40 a typical
        // normal foul resolves around 0.50-0.55 with the
        // strict/lenient/crowd spread still swinging it inside the
        // band.
        let raw = 0.40
            + ctx.contact_severity * 0.22
            + self.strictness * 0.12
            + self.foul_detection * 0.10
            - self.leniency * 0.10
            + bias_term
            + ctx.match_temperature * 0.06
            + pen_strict_bonus;

        if ctx.deliberate {
            return (raw + 0.35).clamp(0.88, 0.99);
        }
        match ctx.location {
            // Normal-contact band lifted [0.10, 0.55] → [0.25, 0.75]
            // alongside the raw-base lift above.
            ContactLocation::Normal => raw.clamp(0.25, 0.75),
            ContactLocation::ClearFoul => raw.clamp(0.70, 0.96),
            // Box ceiling 0.90 → 0.50: real referees are demonstrably
            // more reluctant to whistle in the penalty area — the same
            // contact that draws a free kick at halfway is waved on in
            // the box unless it's clear. Counterweights the raw-base
            // lift so the penalty rate stays in the real ~0.27/match
            // band instead of scaling with the foul-count fix.
            ContactLocation::PenaltyBox => raw.clamp(0.12, 0.50),
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
            deliberate: false,
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
        // Band [0.25, 0.75] per the 2026-06 discipline recalibration —
        // genuine contact is mostly called; marginal-call spread lives
        // inside the band.
        let env = MatchEnvironment::default();
        let r = RefereeProfile::default();
        for severity in [0.0, 0.3, 0.6, 1.0] {
            let p = r.foul_call_prob(&env, ctx(severity, ContactLocation::Normal));
            assert!((0.25..=0.75).contains(&p), "severity {severity} -> {p}");
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
        // Severity 0.5 → 0.0: after the raw-base lift (0.18 → 0.40) a
        // mid-severity box contact saturates the new 0.50 box ceiling
        // for BOTH profiles. At zero severity the strict-vs-soft spread
        // is visible inside the band, which is the property this test
        // exists to protect.
        let env = MatchEnvironment::default();
        let strict = RefereeProfile {
            penalty_strictness: 0.95,
            ..Default::default()
        };
        let soft = RefereeProfile {
            penalty_strictness: 0.10,
            ..Default::default()
        };
        let c = ctx(0.0, ContactLocation::PenaltyBox);
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
        let home_fouled = r.foul_call_prob(
            &big_crowd,
            FoulCallContext {
                contact_severity: 0.5,
                match_temperature: 0.0,
                fouled_team_is_home: true,
                location: ContactLocation::Normal,
                deliberate: false,
            },
        );
        let away_fouled = r.foul_call_prob(
            &big_crowd,
            FoulCallContext {
                contact_severity: 0.5,
                match_temperature: 0.0,
                fouled_team_is_home: false,
                location: ContactLocation::Normal,
                deliberate: false,
            },
        );
        assert!(home_fouled > away_fouled);
    }
}
