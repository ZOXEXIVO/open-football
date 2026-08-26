//! The status ladder — when a want becomes something he says, and when
//! saying it becomes demanding it.
//!
//! Every rung is decided from [`MindGoal::pressure`] against the two
//! bars the catalog gives each want. There are no other thresholds: no
//! "unhappy for 180 days", no per-detector cooldown, no separate
//! escalation rule per desire type. A goal climbs because it is strong,
//! urgent and unmet, and it descends because it stopped being those
//! things.
//!
//! Two rules give the ladder its shape, and both exist because a mind
//! that flips state weekly is not a mind:
//!
//! * **Hysteresis.** Climbing takes the full bar; falling back takes a
//!   clear drop below it. Without that, a goal sitting on its bar would
//!   voice and unvoice itself every review.
//! * **One rung per review.** He does not go from a private feeling to a
//!   formal transfer request in a single week. Anything that ought to
//!   move that fast does it by being fed hard enough to climb on
//!   consecutive reviews.

use super::catalog::GoalKind;
use super::goal::{GoalStatus, MindGoal};
use crate::club::mind::organs::memory::EpochDay;

/// How a goal's rung is decided.
pub struct Escalation;

/// What one goal's review changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusChange {
    pub kind: GoalKind,
    pub from: GoalStatus,
    pub to: GoalStatus,
}

impl Escalation {
    /// Pressure at which a want starts shaping decisions silently. Below
    /// this he feels it and nothing more.
    pub const ACTIVE_AT: f32 = 0.30;

    /// How far pressure must fall *below* a bar before the goal drops
    /// back down a rung. The gap is what stops a goal on its bar from
    /// oscillating every review.
    pub const HYSTERESIS: f32 = 0.08;

    /// Progress at or above which a goal counts as achieved.
    pub const SATISFIED_AT: f32 = 1.0;

    /// The rung this goal's pressure warrants, ignoring where it is now.
    fn warranted(goal: &MindGoal) -> GoalStatus {
        let spec = goal.kind.spec();
        let pressure = goal.pressure();

        if pressure >= spec.press_at {
            GoalStatus::Pressing
        } else if pressure >= spec.voice_at {
            GoalStatus::Voiced
        } else if pressure >= Self::ACTIVE_AT {
            GoalStatus::Active
        } else {
            GoalStatus::Latent
        }
    }

    /// The rung it may fall back to, applying hysteresis. A goal only
    /// descends once pressure is clearly below the bar it climbed.
    fn warranted_falling(goal: &MindGoal) -> GoalStatus {
        let spec = goal.kind.spec();
        let pressure = goal.pressure();
        let gap = Self::HYSTERESIS;

        if pressure >= spec.press_at - gap {
            GoalStatus::Pressing
        } else if pressure >= spec.voice_at - gap {
            GoalStatus::Voiced
        } else if pressure >= Self::ACTIVE_AT - gap {
            GoalStatus::Active
        } else {
            GoalStatus::Latent
        }
    }

    /// Review one goal and move it at most one rung. Returns the change
    /// if the status moved.
    ///
    /// Resolution beats escalation: a goal that has been achieved, has
    /// run out its deadline, or has faded to nothing stops here whatever
    /// its pressure says.
    pub fn review(goal: &mut MindGoal, today: EpochDay) -> Option<StatusChange> {
        let from = goal.status;
        if from.is_resolved() {
            return None;
        }

        // Waiting is itself a form of pressure. Accrued before the rung
        // is decided so a long-held want climbs on its own, without
        // anything outside having to notice it.
        goal.accrue_urgency(today);

        let to = Self::resolved_status(goal, today).unwrap_or_else(|| {
            let target = Self::warranted(goal);
            if target.rung() > from.rung() {
                // Climb exactly one rung.
                Self::next_rung_up(from)
            } else {
                // Falling is gated by hysteresis, and also one rung at a
                // time — a man does not go from a formal request back to
                // saying nothing in a week.
                let floor = Self::warranted_falling(goal);
                if floor.rung() < from.rung() {
                    Self::next_rung_down(from)
                } else {
                    from
                }
            }
        });

        if to == from {
            return None;
        }
        goal.status = to;
        Some(StatusChange {
            kind: goal.kind,
            from,
            to,
        })
    }

    /// Has this goal finished, one way or another?
    fn resolved_status(goal: &MindGoal, today: EpochDay) -> Option<GoalStatus> {
        if goal.progress() >= Self::SATISFIED_AT {
            return Some(GoalStatus::Satisfied);
        }
        // A date he gave himself, come and gone with the want unmet.
        if goal.deadline_passed(today) {
            return Some(GoalStatus::Frustrated);
        }
        if goal.is_spent() {
            return Some(GoalStatus::Abandoned);
        }
        if let Some(months) = goal.kind.spec().abandon_after_months {
            let held_months = goal.age_days(today) / 30;
            if held_months >= months && goal.progress() < 0.5 {
                return Some(GoalStatus::Abandoned);
            }
        }
        None
    }

    fn next_rung_up(from: GoalStatus) -> GoalStatus {
        match from {
            GoalStatus::Latent => GoalStatus::Active,
            GoalStatus::Active => GoalStatus::Voiced,
            GoalStatus::Voiced => GoalStatus::Pressing,
            other => other,
        }
    }

    fn next_rung_down(from: GoalStatus) -> GoalStatus {
        match from {
            GoalStatus::Pressing => GoalStatus::Voiced,
            GoalStatus::Voiced => GoalStatus::Active,
            GoalStatus::Active => GoalStatus::Latent,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::evidence::{GoalEvidence, GoalOrigin};
    use super::*;

    const TODAY: EpochDay = 10_000;

    fn goal_at(kind: GoalKind, strength: f32) -> MindGoal {
        goal_from(kind, GoalOrigin::SelfDrive, strength)
    }

    fn goal_from(kind: GoalKind, origin: GoalOrigin, strength: f32) -> MindGoal {
        let mut g = MindGoal::new(kind, origin, GoalEvidence::EMPTY, TODAY);
        g.set_strength(strength);
        g
    }

    /// Review repeatedly so a goal can climb as many rungs as its
    /// pressure warrants.
    fn settle(goal: &mut MindGoal, today: EpochDay) {
        for _ in 0..6 {
            Escalation::review(goal, today);
        }
    }

    #[test]
    fn a_weak_want_stays_private() {
        let mut g = goal_at(GoalKind::StepUpToABiggerClub, 0.15);
        settle(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Latent);
    }

    #[test]
    fn a_real_want_shapes_decisions_before_anyone_hears_about_it() {
        // The rung that does not exist in the current model for anything
        // but `big_stage_inclination`.
        let mut g = goal_at(GoalKind::StepUpToABiggerClub, 0.55);
        settle(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Active);
        assert!(g.status.shapes_decisions());
        assert!(!g.status.is_public(), "and he has said nothing");
    }

    #[test]
    fn a_strong_want_gets_said_out_loud() {
        let mut g = goal_at(GoalKind::StepUpToABiggerClub, 0.95);
        settle(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Voiced);
    }

    #[test]
    fn waiting_is_itself_a_form_of_pressure() {
        // Same want, same strength, two years apart. Nothing outside has
        // to notice for a long-held want to start pressing.
        let mut fresh = goal_at(GoalKind::StepUpToABiggerClub, 0.95);
        settle(&mut fresh, TODAY);
        assert_eq!(fresh.status, GoalStatus::Voiced);

        let mut long_held = goal_at(GoalKind::StepUpToABiggerClub, 0.95);
        settle(&mut long_held, TODAY + 730);
        assert_eq!(
            long_held.status,
            GoalStatus::Pressing,
            "two years of wanting the same thing is a different state"
        );
        assert!(long_held.urgency() > fresh.urgency());
    }

    #[test]
    fn a_deadline_presses_harder_than_the_years_before_it() {
        let mut g = goal_at(GoalKind::PlayFirstTeamFootball, 0.8);
        g.commit_until(TODAY + 180);

        g.accrue_urgency(TODAY + 10);
        let early = g.urgency();
        g.accrue_urgency(TODAY + 175);
        assert!(
            g.urgency() > early,
            "the last month before the date bites: {early} → {}",
            g.urgency()
        );
    }

    #[test]
    fn urgency_only_ever_rises() {
        let mut g = goal_at(GoalKind::GoHome, 0.5);
        g.set_urgency(0.9);
        g.accrue_urgency(TODAY + 1);
        assert!(
            g.urgency() >= 0.9,
            "an outside signal is never undone by the clock"
        );
    }

    #[test]
    fn an_overwhelming_grievance_becomes_a_demand() {
        let mut g = goal_from(GoalKind::LeaveThisClub, GoalOrigin::Grievance, 1.0);
        settle(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Pressing);
    }

    #[test]
    fn ambition_alone_never_produces_a_demand_on_its_own() {
        // A real property of the model, not an accident of the numbers:
        // wanting more, however badly, does not make a man hand in a
        // transfer request the same season. It takes a grievance, or it
        // takes time — and both routes are exercised above.
        let mut g = goal_at(GoalKind::LeaveThisClub, 1.0);
        settle(&mut g, TODAY);
        assert_eq!(
            g.status,
            GoalStatus::Voiced,
            "he says it, loudly, and stops short of demanding"
        );
    }

    #[test]
    fn he_never_goes_from_a_feeling_to_a_demand_in_one_review() {
        let mut g = goal_at(GoalKind::LeaveThisClub, 1.0);
        let change = Escalation::review(&mut g, TODAY).unwrap();
        assert_eq!(change.from, GoalStatus::Latent);
        assert_eq!(
            change.to,
            GoalStatus::Active,
            "one rung per review, however strongly he feels it"
        );
    }

    #[test]
    fn a_goal_sitting_on_its_bar_does_not_oscillate() {
        // Hysteresis, asserted: park pressure exactly on `voice_at` and
        // review many times. Without the gap this flaps every review.
        let spec = GoalKind::StepUpToABiggerClub.spec();
        let mut g = goal_at(GoalKind::StepUpToABiggerClub, 0.0);
        // Solve for the strength that lands pressure on the bar.
        g.set_strength(spec.voice_at / (0.65 * GoalOrigin::SelfDrive.escalation_bias()));
        settle(&mut g, TODAY);
        let settled = g.status;

        let mut flips = 0;
        for _ in 0..20 {
            if Escalation::review(&mut g, TODAY).is_some() {
                flips += 1;
            }
        }
        assert_eq!(flips, 0, "a settled goal stays settled");
        assert_eq!(g.status, settled);
    }

    #[test]
    fn a_want_that_fades_climbs_back_down_one_rung_at_a_time() {
        let mut g = goal_from(GoalKind::LeaveThisClub, GoalOrigin::Grievance, 1.0);
        settle(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Pressing);

        g.set_strength(0.10);
        let change = Escalation::review(&mut g, TODAY).unwrap();
        assert_eq!(
            change.to,
            GoalStatus::Voiced,
            "a request is not withdrawn silently in a week"
        );

        settle(&mut g, TODAY);
        assert_eq!(
            g.status,
            GoalStatus::Latent,
            "it climbs all the way back down to a private feeling"
        );

        // And once there is nothing left of it at all, he lets it go.
        g.set_strength(0.02);
        Escalation::review(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Abandoned);
    }

    #[test]
    fn getting_what_he_wanted_satisfies_it() {
        let mut g = goal_at(GoalKind::WinBackMyPlace, 0.9);
        settle(&mut g, TODAY);
        assert!(g.status.is_public());

        g.advance(1.0);
        let change = Escalation::review(&mut g, TODAY).unwrap();
        assert_eq!(change.to, GoalStatus::Satisfied);
        assert!(g.status.is_resolved());
    }

    #[test]
    fn a_deadline_come_and_gone_frustrates_it() {
        // The behaviour the current model cannot express at all: he gave
        // it until January, January came, nothing changed.
        let mut g = goal_at(GoalKind::PlayFirstTeamFootball, 0.6);
        g.commit_until(TODAY + 120);
        settle(&mut g, TODAY);
        assert!(g.is_live(), "he is waiting, not agitating");

        let change = Escalation::review(&mut g, TODAY + 121).unwrap();
        assert_eq!(change.to, GoalStatus::Frustrated);
    }

    #[test]
    fn he_holds_the_line_until_the_date_he_set() {
        let mut g = goal_at(GoalKind::PlayFirstTeamFootball, 0.6);
        g.commit_until(TODAY + 120);

        for week in 0..17u16 {
            Escalation::review(&mut g, TODAY + week * 7);
        }
        assert!(
            g.is_live(),
            "four months of coherent waiting — not fifty-two re-rolls"
        );
    }

    #[test]
    fn satisfaction_beats_escalation() {
        let mut g = goal_at(GoalKind::LeaveThisClub, 1.0);
        settle(&mut g, TODAY);
        g.advance(1.0);
        Escalation::review(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Satisfied);
    }

    #[test]
    fn a_want_he_never_gets_anywhere_with_is_eventually_let_go() {
        // `FindANewChallenge` abandons after 36 months of no progress.
        let mut g = goal_at(GoalKind::FindANewChallenge, 0.8);
        settle(&mut g, TODAY);
        assert!(g.is_live());

        Escalation::review(&mut g, TODAY + 30 * 37);
        assert_eq!(g.status, GoalStatus::Abandoned);
    }

    #[test]
    fn a_resolved_goal_is_never_reviewed_again() {
        let mut g = goal_at(GoalKind::WinATrophy, 0.9);
        g.advance(1.0);
        Escalation::review(&mut g, TODAY);
        assert_eq!(g.status, GoalStatus::Satisfied);
        assert!(Escalation::review(&mut g, TODAY + 5_000).is_none());
    }
}
