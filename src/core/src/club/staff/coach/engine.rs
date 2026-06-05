//! Coach decision engine — turns persistent memory + personality +
//! strategy into per-player assessments the selection / substitution
//! layers consume.
//!
//! Design choice: the engine is stateless. It borrows the head
//! coach's [`Staff`], the strategy chosen for the fixture, and the
//! profile derived from the staff. Per-player evidence
//! ([`CoachMemory`]) is borrowed from the staff's memory store. The
//! engine produces an assessment by combining all of that with the
//! player's objective state and the strategy weighting. The
//! adjustments are small by design — the existing scoring engine still
//! does the heavy lifting; the coach engine is the personality lens
//! that nudges the result by a fraction of a slot point.

use super::assessment::{CoachDecisionScore, CoachPlayerAssessment};
use super::memory::{CoachMemory, CoachMemoryFlags, CoachMemoryStore};
use super::reason::CoachDecisionReason;
use super::strategy::CoachStrategy;
use crate::club::staff::CoachProfile;
use crate::utils::DateUtils;
use crate::{Player, PlayerSquadStatus, Staff};
use chrono::NaiveDate;

/// Read-only inputs the engine needs to assess a player for selection.
/// Construct once per match-day and pass repeatedly into the engine's
/// per-player methods.
#[derive(Debug, Clone, Copy)]
pub struct CoachSelectionContext<'a> {
    pub date: NaiveDate,
    pub match_importance: f32,
    pub is_friendly: bool,
    pub is_cup: bool,
    pub is_derby: bool,
    pub is_continental: bool,
    /// Player's natural-role fit at the slot being considered (0..1).
    /// Passed in by the caller so the engine doesn't replicate the
    /// scoring engine's position-fit math.
    pub natural_role_fit: f32,
    /// True when this player is the team's recognised heir at a senior
    /// slot (drives SuccessionPlanning bias).
    pub is_succession_heir: &'a [u32],
}

impl<'a> CoachSelectionContext<'a> {
    pub fn is_succession_heir_of(&self, player_id: u32) -> bool {
        self.is_succession_heir.contains(&player_id)
    }
}

/// Read-only inputs for assessing a live in-match substitution.
#[derive(Debug, Clone, Copy)]
pub struct CoachLiveMatchContext {
    pub date: NaiveDate,
    pub match_minute: u32,
    pub goal_diff: i32,
    pub live_rating: f32,
    pub goals: u16,
    pub assists: u16,
    pub errors_leading_to_goal: u16,
    pub yellow_cards: u16,
    pub red_cards: u16,
    pub condition_pct: f32,
    pub is_starter: bool,
}

/// Stateless coach-decision engine. Borrows the head coach's memory
/// store, derived perception profile, and the fixture's strategy at
/// construction and exposes a small per-player API.
///
/// Construction is decoupled from [`Staff`] so callers that only
/// hold a snapshot of the coach's memory (the live match runner
/// carries a per-side snapshot rather than a full `Staff`) can build
/// an engine without pulling the whole staff record across the match
/// boundary. Squad selection uses [`Self::from_staff`] for convenience.
pub struct CoachDecisionEngine<'a> {
    pub memory: &'a CoachMemoryStore,
    pub strategy: CoachStrategy,
    pub profile: &'a CoachProfile,
}

impl<'a> CoachDecisionEngine<'a> {
    pub fn new(
        memory: &'a CoachMemoryStore,
        profile: &'a CoachProfile,
        strategy: CoachStrategy,
    ) -> Self {
        CoachDecisionEngine {
            memory,
            profile,
            strategy,
        }
    }

    /// Convenience constructor for callers that hold a full `Staff`
    /// reference. Equivalent to passing `&staff.coach_memory` to
    /// [`Self::new`].
    pub fn from_staff(
        staff: &'a Staff,
        profile: &'a CoachProfile,
        strategy: CoachStrategy,
    ) -> Self {
        Self::new(&staff.coach_memory, profile, strategy)
    }

    /// Borrow this coach's memory record for `player_id`, if any.
    pub fn memory_for(&self, player_id: u32) -> Option<&CoachMemory> {
        self.memory.get(player_id)
    }

    /// Build a full assessment of `player` for a starting-XI / bench
    /// decision. Combines form pressure, trust signals, role fit, and
    /// strategy weighting into a single read.
    pub fn assess_player_for_selection(
        &self,
        player: &Player,
        ctx: &CoachSelectionContext<'_>,
    ) -> CoachPlayerAssessment {
        let memory = self.memory_for(player.id);
        let mut reasons: Vec<CoachDecisionReason> = Vec::new();

        let form_confidence = AssessmentMath::form_confidence(memory, self.profile);
        AssessmentMath::push_form_reasons(memory, self.profile, &mut reasons);

        let trust_score = AssessmentMath::trust_score(memory, self.profile);
        AssessmentMath::push_trust_reasons(memory, &mut reasons);

        let risk_confidence = AssessmentMath::risk_confidence(memory);
        AssessmentMath::push_risk_reasons(memory, &mut reasons);

        let role_fit_score = AssessmentMath::role_fit_score(memory, ctx.natural_role_fit);
        if role_fit_score < 0.35 {
            reasons.push(CoachDecisionReason::RoleMismatch);
        }

        let development_priority = AssessmentMath::development_priority(player, ctx, self.profile);
        if development_priority >= 0.6 {
            reasons.push(CoachDecisionReason::DevelopmentPathway);
        }
        if ctx.is_succession_heir_of(player.id) {
            reasons.push(CoachDecisionReason::SuccessionPlanning);
        }

        // Big-match flags are read directly off the flags bit-set so a
        // "BIG_MATCH_PROVEN" player gets the small big-match lift even
        // when his recent form has dipped.
        if let Some(mem) = memory {
            if mem.flags.contains(CoachMemoryFlags::BIG_MATCH_PROVEN)
                && (ctx.is_cup || ctx.is_continental || ctx.is_derby)
            {
                reasons.push(CoachDecisionReason::BigMatchReliability);
            }
            if mem.flags.contains(CoachMemoryFlags::BIG_MATCH_FAILED)
                && (ctx.is_cup || ctx.is_continental || ctx.is_derby)
            {
                reasons.push(CoachDecisionReason::BigMatchFailure);
            }
        }

        // Compose the strategy-weighted selection confidence. The
        // strategy nudges the relative weights; the underlying signals
        // come from memory + objective state.
        let weights = StrategyWeights::for_strategy(self.strategy, self.profile);
        let selection_confidence = (form_confidence * weights.form
            + trust_score * weights.trust
            + risk_confidence * weights.risk
            + role_fit_score * weights.role
            + AssessmentMath::big_match_signal(memory, ctx) * weights.big_match
            + development_priority * weights.development)
            .clamp(0.0, 1.0);

        let drop_risk = (1.0 - selection_confidence).clamp(0.0, 1.0);

        // Bench preference is a softened version of start preference —
        // a player the coach has lost trust in still gets named in the
        // 18 if no alternative exists.
        let bench_preference =
            (selection_confidence * 0.7 + 0.15 + development_priority * 0.10).clamp(0.0, 1.0);

        CoachPlayerAssessment {
            selection_confidence,
            form_confidence,
            risk_confidence,
            trust_score,
            role_fit_score,
            development_priority,
            drop_risk,
            start_preference: selection_confidence,
            bench_preference,
            sub_off_urgency: 0.0,
            sub_in_preference: 0.0,
            reasons,
        }
    }

    /// Compact selection-slot adjustment to fold into a slot score.
    /// Internally calls [`assess_player_for_selection`] and packs the
    /// result into a small score with reasons.
    pub fn score_starting_slot(
        &self,
        player: &Player,
        ctx: &CoachSelectionContext<'_>,
    ) -> CoachDecisionScore {
        let assessment = self.assess_player_for_selection(player, ctx);
        // Adjustment is in slot-score units. Selection slot scores
        // typically range over a few units — half a unit nudge is
        // enough to swing close calls without dominating raw quality.
        let raw_adjustment = assessment.selection_adjustment();
        let scaled = (raw_adjustment * AssessmentMath::SELECTION_SCALE).clamp(
            -AssessmentMath::SELECTION_SCALE,
            AssessmentMath::SELECTION_SCALE,
        );
        CoachDecisionScore {
            adjustment: scaled,
            reasons: assessment.reasons,
        }
    }

    /// Bench-role adjustment counterpart to [`score_starting_slot`].
    /// Smaller magnitude — the bench-role scorer already has its own
    /// fit logic; the coach lens is a finishing touch.
    pub fn score_bench_role(
        &self,
        player: &Player,
        ctx: &CoachSelectionContext<'_>,
    ) -> CoachDecisionScore {
        let assessment = self.assess_player_for_selection(player, ctx);
        let scaled = (assessment.bench_adjustment() * AssessmentMath::BENCH_SCALE).clamp(
            -AssessmentMath::BENCH_SCALE,
            AssessmentMath::BENCH_SCALE,
        );
        CoachDecisionScore {
            adjustment: scaled,
            reasons: assessment.reasons,
        }
    }

    /// Live in-match assessment. The substitution layer reads
    /// `sub_off_urgency` and `sub_in_preference`; the rest of the
    /// assessment is filled in so the caller can attach reasons.
    ///
    /// Uses the full live context — match minute, goal_diff,
    /// condition, is_starter — alongside memory-derived signals so
    /// the coach's read reflects the actual fixture state, not just
    /// the player's rating.
    pub fn assess_live_substitution(
        &self,
        player_id: u32,
        live: &CoachLiveMatchContext,
    ) -> CoachPlayerAssessment {
        let memory = self.memory_for(player_id);
        let mut reasons: Vec<CoachDecisionReason> = Vec::new();

        // Sub-off urgency components.
        let live_perf_gap = ((6.2 - live.live_rating) / 2.0).clamp(0.0, 1.0);
        if live_perf_gap > 0.25 {
            reasons.push(CoachDecisionReason::LiveMatchUnderperformance);
        }
        if live.errors_leading_to_goal > 0 {
            reasons.push(CoachDecisionReason::CostlyError);
        }
        if live.yellow_cards > 0 {
            reasons.push(CoachDecisionReason::CardRisk);
        }

        // Trust signal — scaled by the coach's reaction weight so a
        // negativity-biased coach feels distrust harder.
        let trust_reaction = self.profile.trust_reaction_weight();
        let trust_signal_against = if let Some(m) = memory {
            (1.0 - m.tactical_trust).clamp(0.0, 1.0) * 0.5 * trust_reaction
        } else {
            0.0
        };

        let star_protection = if live.goals + live.assists >= 1 || live.live_rating >= 7.5 {
            reasons.push(CoachDecisionReason::ProtectingStar);
            0.45
        } else if live.live_rating >= 7.0 {
            reasons.push(CoachDecisionReason::ProtectingStar);
            0.20
        } else {
            0.0
        };

        let big_match_protection = if let Some(m) = memory {
            if m.flags.contains(CoachMemoryFlags::BIG_MATCH_PROVEN) {
                0.10
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Game-state amplifiers — the coach's read of the fixture
        // shapes how urgent each signal feels.
        let late_game_pressure = LiveContextMath::late_game_pressure(live.match_minute);
        let losing_pressure = LiveContextMath::losing_pressure(live.goal_diff);
        let fatigue_pressure = LiveContextMath::fatigue_pressure(live.condition_pct);
        let bench_warm_dampener = if live.is_starter { 1.0 } else { 0.4 };

        let raw_urgency = (live_perf_gap * 0.45
            + (live.errors_leading_to_goal as f32).min(2.0) * 0.35
            + trust_signal_against
            + fatigue_pressure * 0.20
            + losing_pressure * live_perf_gap * 0.15
            + late_game_pressure * (live_perf_gap + fatigue_pressure) * 0.10
            - star_protection
            - big_match_protection)
            * bench_warm_dampener;
        let raw_urgency = raw_urgency.clamp(0.0, 1.0);

        let sub_off_urgency = raw_urgency;

        // Sub-in preference reads memory-derived trust + role fit + dev.
        let development_priority = 0.0; // Live-match dev: tracked at the slot caller layer.
        let sub_in_preference = if let Some(m) = memory {
            (m.tactical_trust * 0.4
                + m.big_match_trust * 0.2
                + m.role_fit_confidence * 0.2
                + m.recent_high_rating_count.min(4) as f32 * 0.05
                + late_game_pressure * m.training_trust * 0.05)
                .clamp(0.0, 1.0)
        } else {
            0.55
        };

        // Trust reasons mirrored.
        if let Some(m) = memory {
            if m.tactical_trust < 0.35 {
                reasons.push(CoachDecisionReason::LowTacticalTrust);
            } else if m.tactical_trust >= 0.7 {
                reasons.push(CoachDecisionReason::HighTacticalTrust);
            }
        }

        CoachPlayerAssessment {
            selection_confidence: 1.0 - sub_off_urgency,
            form_confidence: AssessmentMath::form_confidence(memory, self.profile),
            risk_confidence: 1.0 - live_perf_gap,
            trust_score: AssessmentMath::trust_score(memory, self.profile),
            role_fit_score: memory.map(|m| m.role_fit_confidence).unwrap_or(0.5),
            development_priority,
            drop_risk: sub_off_urgency,
            start_preference: 1.0 - sub_off_urgency,
            bench_preference: sub_in_preference,
            sub_off_urgency,
            sub_in_preference,
            reasons,
        }
    }

    /// Compact sub-off adjustment usable by the substitution scorer.
    /// Returns a small additive nudge on the existing sub_off_score.
    pub fn sub_off_adjustment(&self, player_id: u32, live: &CoachLiveMatchContext) -> f32 {
        let a = self.assess_live_substitution(player_id, live);
        // Tighten the live nudge: memory steers the close call,
        // existing scorer makes the final decision.
        ((a.sub_off_urgency - 0.5) * 2.0 * AssessmentMath::LIVE_SCALE).clamp(
            -AssessmentMath::LIVE_SCALE,
            AssessmentMath::LIVE_SCALE,
        )
    }

    /// Compact sub-in adjustment.
    pub fn sub_in_adjustment(&self, player_id: u32, live: &CoachLiveMatchContext) -> f32 {
        let a = self.assess_live_substitution(player_id, live);
        ((a.sub_in_preference - 0.5) * 2.0 * AssessmentMath::LIVE_SCALE).clamp(
            -AssessmentMath::LIVE_SCALE,
            AssessmentMath::LIVE_SCALE,
        )
    }
}

/// Per-strategy weighting of the assessment dimensions. Higher values
/// for a dimension mean the strategy cares about that signal more.
/// Sum need not equal 1.0 — the assessment clamps the final number to
/// [0, 1] so a strategy with stronger weights produces a tighter
/// reading than a flat one.
#[derive(Debug, Clone, Copy)]
struct StrategyWeights {
    form: f32,
    trust: f32,
    risk: f32,
    role: f32,
    big_match: f32,
    development: f32,
}

impl StrategyWeights {
    fn for_strategy(strategy: CoachStrategy, profile: &CoachProfile) -> Self {
        let base = match strategy {
            CoachStrategy::WinNow => StrategyWeights {
                form: 0.25,
                trust: 0.20,
                risk: 0.15,
                role: 0.15,
                big_match: 0.15,
                development: 0.10,
            },
            CoachStrategy::RotateForLoad => StrategyWeights {
                form: 0.15,
                trust: 0.15,
                risk: 0.20,
                role: 0.20,
                big_match: 0.10,
                development: 0.20,
            },
            CoachStrategy::DevelopYouth => StrategyWeights {
                form: 0.10,
                trust: 0.10,
                risk: 0.10,
                role: 0.20,
                big_match: 0.05,
                development: 0.45,
            },
            CoachStrategy::ProtectLead => StrategyWeights {
                form: 0.15,
                trust: 0.30,
                risk: 0.25,
                role: 0.15,
                big_match: 0.10,
                development: 0.05,
            },
            CoachStrategy::ChaseGame => StrategyWeights {
                form: 0.30,
                trust: 0.15,
                risk: 0.10,
                role: 0.15,
                big_match: 0.20,
                development: 0.10,
            },
            CoachStrategy::RebuildSquad => StrategyWeights {
                form: 0.10,
                trust: 0.15,
                risk: 0.10,
                role: 0.20,
                big_match: 0.05,
                development: 0.40,
            },
            CoachStrategy::TrustCore => StrategyWeights {
                form: 0.15,
                trust: 0.40,
                risk: 0.20,
                role: 0.15,
                big_match: 0.10,
                development: 0.00,
            },
            CoachStrategy::CupOpportunity => StrategyWeights {
                form: 0.10,
                trust: 0.10,
                risk: 0.15,
                role: 0.20,
                big_match: 0.10,
                development: 0.35,
            },
            CoachStrategy::SuccessionPlanning => StrategyWeights {
                form: 0.15,
                trust: 0.20,
                risk: 0.15,
                role: 0.20,
                big_match: 0.10,
                development: 0.20,
            },
        };
        // Personality tilt: a tactical coach reads role/trust harder; a
        // negativity-biased coach hammers risk.
        let role_shift = (profile.tactical_blindness - 0.4) * -0.10;
        let risk_shift = (profile.negativity_bias - 0.5) * 0.10;
        StrategyWeights {
            form: base.form,
            trust: (base.trust - role_shift).clamp(0.0, 1.0),
            risk: (base.risk + risk_shift).clamp(0.0, 1.0),
            role: (base.role + role_shift).clamp(0.0, 1.0),
            big_match: base.big_match,
            development: base.development,
        }
    }
}

/// Stateless namespace for the dimension formulas. Keeping the math
/// here lets the engine read as orchestration and the formulas stay
/// in one place for tuning / tests.
///
/// Adjustment-scale notes — the coach lens is **additive** on top of
/// the existing scoring engine. The existing engine already reads:
///   * `player.load.form_rating` (an EMA of raw effective ratings)
///   * `training_impression` (visible effort vs actual performance)
///   * `coach_relationship` (Staff–Player relations)
/// The coach memory layer reads a *different* signal — the head
/// coach's personality-shaped *interpretation* of recent ratings vs
/// long-form baseline. The two share evidence but are computed
/// through different lenses, so there is intentional overlap. The
/// scales below are deliberately conservative so the coach lens nudges
/// close calls (a fraction of a slot point) instead of stacking a
/// second form penalty on top of the existing one. Raising
/// `SELECTION_SCALE` above ~0.7 risks the engine bench-rotating a
/// player twice on the same evidence.
struct AssessmentMath;

impl AssessmentMath {
    const SELECTION_SCALE: f32 = 0.55;
    const BENCH_SCALE: f32 = 0.30;
    const LIVE_SCALE: f32 = 0.30;

    fn form_confidence(memory: Option<&CoachMemory>, profile: &CoachProfile) -> f32 {
        let Some(m) = memory else { return 0.5 };
        if !m.is_well_observed() {
            return 0.5;
        }
        // form_lift raises confidence; form_pressure lowers it. The
        // coach's `form_reaction_weight` scales how strongly form
        // evidence moves the confidence; `one_bad_game_dampener`
        // further softens the pressure side so a high-judging,
        // high-man-management coach doesn't snap to a low confidence
        // on a single bad sample.
        let lift = m.form_lift();
        let pressure = m.form_pressure();
        let reaction = profile.form_reaction_weight();
        let dampener = profile.one_bad_game_dampener();
        let net =
            (lift * reaction - pressure * dampener * reaction).clamp(-1.0, 1.0);
        (0.5 + net * 0.5).clamp(0.0, 1.0)
    }

    fn trust_score(memory: Option<&CoachMemory>, profile: &CoachProfile) -> f32 {
        let Some(m) = memory else { return 0.5 };
        // Composite trust: tactical (50%) + big_match (25%) + training (25%).
        // Pulled toward 0.5 by the inverse of the coach's trust reaction —
        // a low-attitude, high-man-management coach reads the same trust
        // signal as a smaller selection move than an authoritarian one.
        let raw = m.tactical_trust * 0.5 + m.big_match_trust * 0.25 + m.training_trust * 0.25;
        let reaction = profile.trust_reaction_weight();
        // raw moves around 0.5; reaction in [0.3, 1.3] scales how far.
        (0.5 + (raw - 0.5) * reaction).clamp(0.0, 1.0)
    }

    fn risk_confidence(memory: Option<&CoachMemory>) -> f32 {
        let Some(m) = memory else { return 0.5 };
        let mut score: f32 = 1.0;
        if m.flags.contains(CoachMemoryFlags::STICKY_DOUBT) {
            score -= 0.30;
        }
        if m.flags.contains(CoachMemoryFlags::EARLY_HOOK_RECENT) {
            score -= 0.10;
        }
        score.clamp(0.0, 1.0)
    }

    fn role_fit_score(memory: Option<&CoachMemory>, natural_role_fit: f32) -> f32 {
        let base = natural_role_fit.clamp(0.0, 1.0);
        if let Some(m) = memory {
            // Coach's role_fit_confidence pulls the read toward what
            // they have seen — a player who keeps performing out of
            // position can edge a small lift.
            (base * 0.6 + m.role_fit_confidence * 0.4).clamp(0.0, 1.0)
        } else {
            base
        }
    }

    fn big_match_signal(memory: Option<&CoachMemory>, ctx: &CoachSelectionContext<'_>) -> f32 {
        if !(ctx.is_cup || ctx.is_derby || ctx.is_continental) {
            return 0.5;
        }
        let Some(m) = memory else { return 0.5 };
        m.big_match_trust
    }

    fn development_priority(
        player: &Player,
        ctx: &CoachSelectionContext<'_>,
        profile: &CoachProfile,
    ) -> f32 {
        let age = DateUtils::age(player.birth_date, ctx.date);
        let age_bucket = if age <= 19 {
            1.0
        } else if age <= 22 {
            0.7
        } else if age <= 25 {
            0.3
        } else {
            0.0
        };
        let status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
        let status_bump = match status {
            PlayerSquadStatus::HotProspectForTheFuture => 0.25,
            PlayerSquadStatus::DecentYoungster => 0.10,
            PlayerSquadStatus::FirstTeamSquadRotation => 0.05,
            _ => 0.0,
        };
        (age_bucket + status_bump) * profile.development_patience()
    }

    fn push_form_reasons(
        memory: Option<&CoachMemory>,
        profile: &CoachProfile,
        reasons: &mut Vec<CoachDecisionReason>,
    ) {
        let Some(m) = memory else { return };
        if !m.is_well_observed() {
            return;
        }
        let pressure = m.form_pressure();
        let lift = m.form_lift();
        // A "sustained" drop is poor_match_streak >= 3 OR
        // recent_low_rating_count >= 3.
        let sustained =
            m.poor_match_streak >= 3 || m.recent_low_rating_count >= 3;
        if sustained {
            reasons.push(CoachDecisionReason::SustainedPoorForm);
        } else if pressure > 0.15 {
            // High-recency / low-judging coach surfaces this earlier.
            let threshold: f32 = 0.15 * profile.one_bad_game_dampener();
            if pressure > threshold.max(0.05) {
                reasons.push(CoachDecisionReason::PoorRecentForm);
            }
        }
        if lift > 0.20 {
            reasons.push(CoachDecisionReason::StrongRecentForm);
        }
    }

    fn push_trust_reasons(
        memory: Option<&CoachMemory>,
        reasons: &mut Vec<CoachDecisionReason>,
    ) {
        let Some(m) = memory else { return };
        if m.tactical_trust < 0.35 {
            reasons.push(CoachDecisionReason::LowTacticalTrust);
        } else if m.tactical_trust >= 0.70 {
            reasons.push(CoachDecisionReason::HighTacticalTrust);
        }
        if m.training_trust >= 0.70 {
            reasons.push(CoachDecisionReason::TrainingLevel);
        }
    }

    fn push_risk_reasons(memory: Option<&CoachMemory>, reasons: &mut Vec<CoachDecisionReason>) {
        let Some(m) = memory else { return };
        if m.flags.contains(CoachMemoryFlags::STICKY_DOUBT) {
            reasons.push(CoachDecisionReason::StickyDoubt);
        }
    }
}

/// Stateless namespace for the live-match context amplifiers. Keeps
/// the per-signal formulas in one place so the live assessment reads
/// as orchestration and the curves stay easy to tune.
struct LiveContextMath;

impl LiveContextMath {
    /// Late-game pressure in [0.0, 1.0]. Zero before the 60th minute,
    /// climbs to ~1.0 by the 85th. Amplifies form-pressure and fatigue
    /// in the closing stages — a 6.0 rating at minute 80 is more
    /// urgent than the same rating at minute 50.
    fn late_game_pressure(match_minute: u32) -> f32 {
        if match_minute <= 60 {
            0.0
        } else if match_minute >= 85 {
            1.0
        } else {
            ((match_minute - 60) as f32 / 25.0).clamp(0.0, 1.0)
        }
    }

    /// Losing pressure in [0.0, 1.0]. Trailing by one is mild
    /// pressure; trailing by 2+ is strong. Used as a multiplier on
    /// form-pressure so a bad showing in a losing game is more urgent
    /// than the same showing in a draw.
    fn losing_pressure(goal_diff: i32) -> f32 {
        if goal_diff >= 0 {
            0.0
        } else if goal_diff <= -2 {
            1.0
        } else {
            0.5
        }
    }

    /// Fatigue pressure in [0.0, 1.0] derived from condition. Below
    /// 50% condition starts to register; below 30% is a strong sub-
    /// off signal.
    fn fatigue_pressure(condition_pct: f32) -> f32 {
        let c = condition_pct.clamp(0.0, 1.0);
        if c >= 0.6 {
            0.0
        } else if c <= 0.25 {
            1.0
        } else {
            ((0.6 - c) / 0.35).clamp(0.0, 1.0)
        }
    }
}
