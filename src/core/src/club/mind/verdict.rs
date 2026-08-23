//! What a faculty gives back.
//!
//! Two answers, shared by both minds. [`MoodContribution`] is a
//! faculty's read of how its owner *feels* about the slice of life it
//! speaks for. [`ReasonSet`] is its opinion on a decision, as named and
//! weighted reasons rather than a bare number — so every verdict can be
//! rendered, argued with, and tested.
//!
//! Both are `Copy` and bounded. A verdict that allocated would put a
//! heap pointer inside every `Player` and every `Staff`.

use super::organs::goals::{GoalDomain, GoalKind};
use std::cmp::Ordering;

/// One faculty's contribution to how its owner feels.
///
/// Signed, on the same rough scale as the existing `HappinessFactors`
/// (−10..+10 per axis), plus a confidence saying how much this faculty
/// actually knows. A faculty with nothing to go on returns
/// [`MoodContribution::silent`] rather than a confident zero — the
/// difference matters when the contributions are combined, and it is
/// the thing a flat `morale = 50` cannot express.
#[derive(Debug, Clone, Copy)]
pub struct MoodContribution {
    pub domain: GoalDomain,
    /// Signed contribution, roughly −10..+10.
    pub value: f32,
    /// 0..1 — how much this faculty has to go on. Zero means "no view",
    /// which is not the same as "no problem".
    pub confidence: f32,
}

impl MoodContribution {
    /// This faculty has nothing to say.
    pub fn silent(domain: GoalDomain) -> Self {
        MoodContribution {
            domain,
            value: 0.0,
            confidence: 0.0,
        }
    }

    pub fn new(domain: GoalDomain, value: f32, confidence: f32) -> Self {
        MoodContribution {
            domain,
            value: value.clamp(-10.0, 10.0),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    /// The contribution actually applied, once confidence is taken into
    /// account.
    #[inline]
    pub fn weighted(&self) -> f32 {
        self.value * self.confidence
    }

    #[inline]
    pub fn is_silent(&self) -> bool {
        self.confidence <= 0.0
    }
}

/// A decision someone is being asked about.
///
/// One enum for both minds. A player and a manager face genuinely
/// different questions, but they face them through the same `weigh`
/// call, and a shared option type is what lets the deliberation layer
/// stay one layer rather than two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MindOption {
    // ── A player ────────────────────────────────────────────────
    /// Would he join this club?
    JoinClub(u32),
    /// Would he sign these terms?
    SignContract,
    /// Would he go out on loan?
    AcceptLoan(u32),
    /// Should he ask to leave?
    RequestTransfer,
    /// Should he stay and fight for his place?
    StayAndFight,
    /// Is it time to stop?
    Retire,

    // ── A manager ───────────────────────────────────────────────
    /// Would he take this job?
    TakeTheJob(u32),
    /// Does he want this player signed?
    SignThisPlayer(u32),
    /// Would he let this player go?
    SellThisPlayer(u32),
    /// Is this player out of the side?
    DropThisPlayer(u32),
    /// Should he change how the team plays?
    ChangeTheSystem,
    /// Should he walk?
    Resign,
}

/// One named, weighted reason a faculty gives for or against an option.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeightedReason {
    pub goal: GoalKind,
    /// −1..+1. Positive argues for the option.
    pub weight: f32,
}

/// A faculty's answer to a `weigh` call. Bounded so the whole verdict
/// stays `Copy`; six reasons is more than any renderer will print.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReasonSet {
    reasons: [WeightedReason; 6],
    len: u8,
}

impl ReasonSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, goal: GoalKind, weight: f32) {
        if self.len as usize >= self.reasons.len() {
            return;
        }
        self.reasons[self.len as usize] = WeightedReason {
            goal,
            weight: weight.clamp(-1.0, 1.0),
        };
        self.len += 1;
    }

    #[inline]
    pub fn as_slice(&self) -> &[WeightedReason] {
        &self.reasons[..self.len as usize]
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Net argument, positive for and negative against.
    pub fn net(&self) -> f32 {
        self.as_slice()
            .iter()
            .map(|r| r.weight)
            .sum::<f32>()
            .clamp(-1.0, 1.0)
    }

    /// Fold another faculty's reasons in, keeping as many as fit. The
    /// fan-out order decides who gets heard when a decision draws more
    /// than six arguments; every faculty still contributes to
    /// [`Self::net`] through its own set before the merge.
    pub fn absorb(&mut self, other: &ReasonSet) {
        for reason in other.as_slice() {
            self.push(reason.goal, reason.weight);
        }
    }

    /// The single loudest argument, either way.
    pub fn strongest(&self) -> Option<WeightedReason> {
        self.as_slice().iter().copied().max_by(|a, b| {
            a.weight
                .abs()
                .partial_cmp(&b.weight.abs())
                .unwrap_or(Ordering::Equal)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_not_the_same_as_contentment() {
        let nothing_to_say = MoodContribution::silent(GoalDomain::Financial);
        let genuinely_fine = MoodContribution::new(GoalDomain::Financial, 0.0, 1.0);

        assert!(nothing_to_say.is_silent());
        assert!(!genuinely_fine.is_silent());
        assert_eq!(nothing_to_say.weighted(), genuinely_fine.weighted());
    }

    #[test]
    fn confidence_scales_what_actually_lands() {
        let sure = MoodContribution::new(GoalDomain::Social, -6.0, 1.0);
        let unsure = MoodContribution::new(GoalDomain::Social, -6.0, 0.25);
        assert_eq!(sure.weighted(), -6.0);
        assert_eq!(unsure.weighted(), -1.5);
    }

    #[test]
    fn contributions_stay_in_band() {
        let extreme = MoodContribution::new(GoalDomain::Career, 500.0, 5.0);
        assert_eq!(extreme.value, 10.0);
        assert_eq!(extreme.confidence, 1.0);
    }

    #[test]
    fn a_reason_set_is_bounded_and_nets_out() {
        let mut reasons = ReasonSet::new();
        assert!(reasons.is_empty());

        reasons.push(GoalKind::StepUpToABiggerClub, 0.6);
        reasons.push(GoalKind::StayAtThisClub, -0.3);
        assert_eq!(reasons.as_slice().len(), 2);
        assert!((reasons.net() - 0.3).abs() < 1e-5);

        for _ in 0..20 {
            reasons.push(GoalKind::GoHome, 1.0);
        }
        assert_eq!(reasons.as_slice().len(), 6, "the set is bounded");
        assert!(reasons.net() <= 1.0);
    }

    #[test]
    fn the_strongest_reason_is_the_loudest_either_way() {
        let mut reasons = ReasonSet::new();
        reasons.push(GoalKind::KeepThisJob, 0.4);
        reasons.push(GoalKind::ProveThemWrong, -0.9);
        reasons.push(GoalKind::GetABiggerJob, 0.5);

        let loudest = reasons.strongest().expect("three reasons were pushed");
        assert_eq!(loudest.goal, GoalKind::ProveThemWrong);
    }

    #[test]
    fn absorbing_merges_up_to_the_bound() {
        let mut mine = ReasonSet::new();
        mine.push(GoalKind::KeepThisJob, 0.5);

        let mut theirs = ReasonSet::new();
        theirs.push(GoalKind::BeBackedInTheMarket, -0.2);
        theirs.push(GoalKind::WinSomethingHere, 0.3);

        mine.absorb(&theirs);
        assert_eq!(mine.as_slice().len(), 3);
    }
}
