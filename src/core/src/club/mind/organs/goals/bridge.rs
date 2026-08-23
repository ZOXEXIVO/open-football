//! The migration bridge between the legacy desire enums and the goal
//! stack.
//!
//! Phase 3 runs both systems side by side: `process_transfer_desire`
//! keeps computing `TransferRequestReason` exactly as it does today, and
//! every reason it finds also feeds the matching goal. Nothing downstream
//! changes yet — the old path is still the one that sets `Req`.
//!
//! What that buys is a real equivalence check before anything is
//! deleted. The transfer suites in this repo are calibrated; swapping
//! their input out in one step would invalidate a lot of them at once.
//! Running in parallel means the goal stack can be shown to reach the
//! same verdict on the same corpus first, and only then does
//! `TransferRequestReason` become a *view* over `GoalStack` rather than
//! a thing computed separately.
//!
//! The mapping is deliberately many-to-one in places. Three of the
//! legacy reasons are different evidence for the same want — a player
//! who has outgrown his club and a player who wants a new challenge are
//! both, underneath, players who want to step up — and collapsing them
//! is most of the point of the goal model.

use super::catalog::GoalKind;
use super::evidence::{GoalEvidence, GoalOrigin};
use crate::club::player::core::player::TransferRequestReason;

/// How one legacy reason maps onto a want.
#[derive(Debug, Clone, Copy)]
pub struct ReasonMapping {
    pub goal: GoalKind,
    pub origin: GoalOrigin,
    pub evidence: GoalEvidence,
    /// How hard this reason should push the goal each time it is seen.
    /// A reason that only fires once something is already serious pushes
    /// harder than an ambient one.
    pub weight: f32,
}

/// Translation between the legacy desire enums and [`GoalKind`].
pub struct GoalBridge;

impl GoalBridge {
    /// What a legacy transfer-request reason means as a want.
    pub fn from_transfer_request_reason(reason: TransferRequestReason) -> ReasonMapping {
        use GoalEvidence as E;
        use TransferRequestReason as R;

        let map =
            |goal: GoalKind, origin: GoalOrigin, atoms: &[u32], weight: f32| -> ReasonMapping {
                ReasonMapping {
                    goal,
                    origin,
                    evidence: GoalEvidence::of(atoms),
                    weight,
                }
            };

        match reason {
            // Character trouble is not a want. It reads as wanting out
            // because that is how it resolves, but the origin is the
            // player himself rather than anything done to him.
            R::PoorBehaviour => map(GoalKind::LeaveThisClub, GoalOrigin::Circumstance, &[], 0.55),
            // The terminus of everything unresolved. Fires only after
            // ~six months of held unhappiness, so it pushes hard.
            R::LongUnhappiness => map(GoalKind::LeaveThisClub, GoalOrigin::Grievance, &[], 0.85),
            R::AmbitionMismatch => map(
                GoalKind::StepUpToABiggerClub,
                GoalOrigin::Grievance,
                &[E::OUTGROWN_CLUB, E::HIGH_AMBITION],
                0.75,
            ),
            R::OutgrownClub => map(
                GoalKind::StepUpToABiggerClub,
                GoalOrigin::SelfDrive,
                &[E::OUTGROWN_CLUB, E::HIGH_AMBITION],
                0.70,
            ),
            R::NewChallenge => map(
                GoalKind::FindANewChallenge,
                GoalOrigin::SelfDrive,
                &[E::LONG_SERVICE, E::NOTHING_LEFT_TO_PROVE],
                0.60,
            ),
            R::SalaryUnresolved => map(
                GoalKind::BePaidWhatImWorth,
                GoalOrigin::Grievance,
                &[E::PAID_BELOW_HIS_PEERS, E::TERMS_REFUSED],
                0.80,
            ),
            R::ReturnHome => map(
                GoalKind::GoHome,
                GoalOrigin::Attachment,
                &[E::HOMESICK, E::ISOLATED_IN_THE_SQUAD],
                0.70,
            ),
            R::EuropeanAmbition => map(
                GoalKind::PlayContinentalFootball,
                GoalOrigin::SelfDrive,
                &[E::NO_CONTINENTAL_ROUTE, E::HIGH_AMBITION],
                0.70,
            ),
            R::CopaLibertadoresAmbition => map(
                GoalKind::PlayInLibertadores,
                GoalOrigin::SelfDrive,
                &[E::NO_CONTINENTAL_ROUTE, E::HIGH_AMBITION],
                0.70,
            ),
            R::RelegationEscape => map(
                GoalKind::KeepPlayingAtThisLevel,
                GoalOrigin::Circumstance,
                &[E::RELEGATED],
                0.80,
            ),
            R::WantsStrongerLeague => map(
                GoalKind::PlayInAStrongerLeague,
                GoalOrigin::SelfDrive,
                &[E::LEAGUE_IS_A_CEILING, E::HIGH_AMBITION],
                0.65,
            ),
            R::WantsFirstTeamFootball => map(
                GoalKind::PlayFirstTeamFootball,
                GoalOrigin::Survival,
                &[E::NO_FIRST_TEAM_FOOTBALL, E::PRIME_YEARS_PASSING],
                0.80,
            ),
        }
    }

    /// The legacy reason a want corresponds to, where one exists.
    ///
    /// The inverse direction, for the phase where
    /// `TransferRequestReason` becomes a view over the stack rather than
    /// a separately-computed set. `None` for the many goals that have no
    /// legacy equivalent — the model is deliberately wider than the enum
    /// it replaces.
    pub fn to_transfer_request_reason(goal: GoalKind) -> Option<TransferRequestReason> {
        use TransferRequestReason as R;

        match goal {
            GoalKind::LeaveThisClub => Some(R::LongUnhappiness),
            GoalKind::StepUpToABiggerClub => Some(R::OutgrownClub),
            GoalKind::FindANewChallenge => Some(R::NewChallenge),
            GoalKind::BePaidWhatImWorth => Some(R::SalaryUnresolved),
            GoalKind::GoHome => Some(R::ReturnHome),
            GoalKind::PlayContinentalFootball => Some(R::EuropeanAmbition),
            GoalKind::PlayInLibertadores => Some(R::CopaLibertadoresAmbition),
            GoalKind::KeepPlayingAtThisLevel => Some(R::RelegationEscape),
            GoalKind::PlayInAStrongerLeague => Some(R::WantsStrongerLeague),
            GoalKind::PlayFirstTeamFootball => Some(R::WantsFirstTeamFootball),
            _ => None,
        }
    }

    /// Every legacy reason, for the coverage audit.
    pub const ALL_REASONS: &'static [TransferRequestReason] = &[
        TransferRequestReason::PoorBehaviour,
        TransferRequestReason::LongUnhappiness,
        TransferRequestReason::AmbitionMismatch,
        TransferRequestReason::OutgrownClub,
        TransferRequestReason::NewChallenge,
        TransferRequestReason::SalaryUnresolved,
        TransferRequestReason::ReturnHome,
        TransferRequestReason::EuropeanAmbition,
        TransferRequestReason::CopaLibertadoresAmbition,
        TransferRequestReason::RelegationEscape,
        TransferRequestReason::WantsStrongerLeague,
        TransferRequestReason::WantsFirstTeamFootball,
    ];
}

#[cfg(test)]
mod tests {
    use super::super::stack::GoalStack;
    use super::*;
    use crate::club::mind::organs::memory::EpochDay;

    const TODAY: EpochDay = 10_000;

    #[test]
    fn every_legacy_reason_maps_to_a_real_want() {
        for reason in GoalBridge::ALL_REASONS {
            let mapping = GoalBridge::from_transfer_request_reason(*reason);
            assert_ne!(
                mapping.goal,
                GoalKind::None,
                "{reason:?} has no goal behind it"
            );
            assert!(
                (0.0..=1.0).contains(&mapping.weight),
                "{reason:?} weight out of band"
            );
        }
    }

    #[test]
    fn the_twelve_reasons_collapse_to_fewer_wants() {
        // The point of the model: several legacy reasons are different
        // evidence for the same underlying want.
        let mut goals: Vec<GoalKind> = GoalBridge::ALL_REASONS
            .iter()
            .map(|r| GoalBridge::from_transfer_request_reason(*r).goal)
            .collect();
        goals.sort_by_key(|g| g.bit());
        let before = goals.len();
        goals.dedup();
        assert!(
            goals.len() < before,
            "twelve reasons should not need twelve separate wants"
        );
    }

    #[test]
    fn every_reason_that_points_out_maps_to_a_goal_that_points_out() {
        // Except the wage grievance, which is genuinely neutral — being
        // underpaid is fixed by a new contract as readily as by a move.
        for reason in GoalBridge::ALL_REASONS {
            let mapping = GoalBridge::from_transfer_request_reason(*reason);
            if *reason == TransferRequestReason::SalaryUnresolved {
                continue;
            }
            assert!(
                mapping.goal.points_away(),
                "{reason:?} maps to {:?}, which does not point out of the club",
                mapping.goal
            );
        }
    }

    #[test]
    fn the_round_trip_is_stable_where_it_exists() {
        for reason in GoalBridge::ALL_REASONS {
            let goal = GoalBridge::from_transfer_request_reason(*reason).goal;
            let Some(back) = GoalBridge::to_transfer_request_reason(goal) else {
                continue;
            };
            // Not necessarily the same reason — several collapse onto one
            // want — but the round trip must land on a reason that maps
            // back to the same want.
            assert_eq!(
                GoalBridge::from_transfer_request_reason(back).goal,
                goal,
                "{reason:?} → {goal:?} → {back:?} does not return to {goal:?}"
            );
        }
    }

    #[test]
    fn feeding_the_legacy_reasons_produces_a_pressing_want() {
        // The parallel-run property: the same signal that sets `Req`
        // today, fed weekly, reaches a formal demand in the goal stack.
        let mut stack = GoalStack::new();
        let mapping =
            GoalBridge::from_transfer_request_reason(TransferRequestReason::LongUnhappiness);

        let mut day = TODAY;
        for _ in 0..16 {
            day += 7;
            stack.pursue(
                mapping.goal,
                mapping.origin,
                mapping.evidence,
                mapping.weight,
                day,
            );
            stack.review(day);
        }

        assert!(
            stack.is_pressing(),
            "sustained unhappiness must reach a formal demand"
        );
        assert_eq!(
            GoalBridge::to_transfer_request_reason(mapping.goal),
            Some(TransferRequestReason::LongUnhappiness)
        );
    }

    #[test]
    fn a_reason_that_stops_firing_lets_the_want_fade() {
        // The other half of the parallel run: today a reason vanishing
        // clears `Req` immediately. Here it fades — which is the
        // behaviour change the model is for, and it must be a *fade*
        // rather than a cliff.
        let mut stack = GoalStack::new();
        let mapping =
            GoalBridge::from_transfer_request_reason(TransferRequestReason::LongUnhappiness);

        let mut day = TODAY;
        for _ in 0..16 {
            day += 7;
            stack.pursue(
                mapping.goal,
                mapping.origin,
                mapping.evidence,
                mapping.weight,
                day,
            );
            stack.review(day);
        }
        let pressing = stack.pressure_of(mapping.goal);

        // The grievance is resolved and nothing feeds it any more.
        for _ in 0..40 {
            day += 7;
            stack.review(day);
        }
        let after_a_season = stack.pressure_of(mapping.goal);
        assert!(
            after_a_season < pressing,
            "it does subside: {pressing} → {after_a_season}"
        );
        assert!(
            stack.is_pressing(),
            "but a formal demand is not quietly withdrawn inside a season — \
             the hysteresis band is what stops a goal on its bar from flapping"
        );

        // Long enough, and it clears the band and comes down.
        for _ in 0..40 {
            day += 7;
            stack.review(day);
        }
        assert!(stack.pressure_of(mapping.goal) < after_a_season);
        assert!(!stack.is_pressing(), "eventually he stops asking");
    }
}
