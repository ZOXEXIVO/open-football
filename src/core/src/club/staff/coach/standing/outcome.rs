//! What a standing actually does.
//!
//! The ladder decides; this says what the decision is worth to the layers
//! that consume it. Two of them matter and they want different things:
//! selection wants a signed shift it can fold into a preference, and the
//! squad plan wants a bound on what role the coach is willing to give the
//! man. Both are held here so the mapping is in one readable table rather
//! than smeared across the call sites.

use super::ladder::{CoachStanding, StandingRung};
use super::tuning::StandingTuning;
use crate::club::staff::coach::plan::PlannedRole;
use chrono::NaiveDate;

/// What one standing is worth at one decision point.
#[derive(Debug, Clone, Copy, Default)]
pub struct StandingOutcome {
    /// Signed shift on the coach's preference to start him.
    pub start_shift: f32,
    /// Signed shift on his preference to name him in the eighteen.
    pub bench_shift: f32,
    /// How much of the ordinary form pressure still reaches him.
    pub form_dampener: f32,
    /// He is owed a start and this is a fixture the coach can pay it in.
    pub owed_a_start: bool,
    /// The coach is not picking him for this one.
    pub frozen_out: bool,
    /// He has run out of chances on the big nights, and this is one.
    pub big_match_untrusted: bool,
}

impl StandingOutcome {
    /// What a player with no standing gets: nothing, in either direction.
    pub fn neutral() -> Self {
        StandingOutcome {
            form_dampener: 1.0,
            ..StandingOutcome::default()
        }
    }
}

/// Reads a standing for the layer asking.
pub struct StandingRead;

impl StandingRead {
    /// What this standing is worth for one fixture.
    pub fn for_selection(
        standing: Option<&CoachStanding>,
        big_match: bool,
        match_importance: f32,
        man_management: f32,
        today: NaiveDate,
    ) -> StandingOutcome {
        let Some(standing) = standing else {
            return StandingOutcome::neutral();
        };

        let mut start_shift = standing.rung.selection_shift();
        let big_match_untrusted = big_match && standing.is_big_match_untrusted();
        if big_match_untrusted {
            start_shift += StandingTuning::BIG_MATCH_UNTRUSTED_SHIFT;
        }

        StandingOutcome {
            start_shift,
            bench_shift: standing.rung.bench_shift(),
            form_dampener: standing.form_dampener(today),
            owed_a_start: standing.owes_a_start(man_management, match_importance),
            frozen_out: standing.rung == StandingRung::FrozenOut,
            big_match_untrusted,
        }
    }

    /// The most, and the least, central role the coach is willing to give a
    /// man on this rung. `None` on either side means he is not constrained.
    ///
    /// This is what turns a standing into a career: a plan role feeds the
    /// renewal desk, the listing sweep and the conversation where a player
    /// is told where he stands.
    pub fn plan_bounds(standing: Option<&CoachStanding>) -> (Option<PlannedRole>, Option<PlannedRole>) {
        let Some(standing) = standing else {
            return (None, None);
        };
        match standing.rung {
            // A man he will not leave out is the man he builds around.
            StandingRung::Undroppable => (Some(PlannedRole::Cornerstone), None),
            StandingRung::Trusted => (Some(PlannedRole::Starter), None),
            StandingRung::InFavour | StandingRung::Neutral | StandingRung::UnderReview => {
                (None, None)
            }
            // He still plays, but not as anybody's first choice.
            StandingRung::OutOfFavour => (None, Some(PlannedRole::Rotation)),
            // There is no honest role here at all.
            StandingRung::FrozenOut => (None, Some(PlannedRole::ShopWindow)),
        }
    }

    /// Clamp a derived role into what the standing allows.
    ///
    /// The floor only applies where the coach can honour it: it lifts a man
    /// who is *ranked* highly enough to deserve it, and it never overrides
    /// the specialist's read of the goalkeeping order.
    pub fn bound_role(
        derived: PlannedRole,
        standing: Option<&CoachStanding>,
        rank: usize,
    ) -> PlannedRole {
        let (floor, ceiling) = Self::plan_bounds(standing);
        let mut role = derived;
        if let Some(floor) = floor {
            // Only lift a man the depth chart already puts near the front.
            // A coach can trust his third-choice left-back without calling
            // him a cornerstone.
            let deserves = match floor {
                PlannedRole::Cornerstone => rank == 0,
                PlannedRole::Starter => rank <= 1,
                _ => true,
            };
            if deserves && !role.is_at_least(floor) {
                role = floor;
            }
        }
        if let Some(ceiling) = ceiling {
            if role.is_at_least(ceiling) && role != ceiling {
                role = ceiling;
            }
        }
        role
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ladder::{GrievanceFlags, LadderContext, StandingLadder};

    /// Fixture builders, grouped so the tests read as sentences.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap()
        }

        fn at(rung: StandingRung) -> CoachStanding {
            let mut standing = CoachStanding::default();
            standing.rung = rung;
            standing.since = Some(Self::date());
            standing
        }
    }

    #[test]
    fn a_player_with_no_standing_is_read_as_neutral() {
        let read = StandingRead::for_selection(None, true, 0.9, 0.5, Fx::date());
        assert_eq!(read.start_shift, 0.0);
        assert_eq!(read.bench_shift, 0.0);
        assert_eq!(read.form_dampener, 1.0);
        assert!(!read.frozen_out);
        assert_eq!(StandingRead::plan_bounds(None), (None, None));
    }

    #[test]
    fn the_ladder_reads_the_same_way_it_is_ordered() {
        let mut last = f32::INFINITY;
        for rung in [
            StandingRung::Undroppable,
            StandingRung::Trusted,
            StandingRung::InFavour,
            StandingRung::Neutral,
            StandingRung::UnderReview,
            StandingRung::OutOfFavour,
            StandingRung::FrozenOut,
        ] {
            let shift = rung.selection_shift();
            assert!(shift < last, "{rung:?} broke the order");
            last = shift;
        }
    }

    #[test]
    fn distrust_on_the_big_nights_only_bites_on_the_big_nights() {
        let mut standing = Fx::at(StandingRung::Trusted);
        standing
            .grievance
            .insert(GrievanceFlags::BIG_MATCH_UNTRUSTED);

        let tuesday = StandingRead::for_selection(Some(&standing), false, 0.6, 0.5, Fx::date());
        let the_final = StandingRead::for_selection(Some(&standing), true, 0.95, 0.5, Fx::date());

        assert!(tuesday.start_shift > 0.0, "he is fine on a wet Tuesday");
        assert!(the_final.start_shift < 0.0, "and not in the final");
    }

    #[test]
    fn a_frozen_out_man_has_no_honest_role_left() {
        let standing = Fx::at(StandingRung::FrozenOut);
        let read = StandingRead::for_selection(Some(&standing), false, 0.5, 0.9, Fx::date());
        assert!(read.frozen_out);
        assert!(read.start_shift <= StandingTuning::SHIFT_FROZEN_OUT);

        let bounded = StandingRead::bound_role(PlannedRole::Starter, Some(&standing), 0);
        assert!(bounded.is_exit_path());
    }

    #[test]
    fn a_man_out_of_favour_still_plays_but_not_as_a_first_choice() {
        let standing = Fx::at(StandingRung::OutOfFavour);
        assert_eq!(
            StandingRead::bound_role(PlannedRole::Starter, Some(&standing), 0),
            PlannedRole::Rotation
        );
        assert_eq!(
            StandingRead::bound_role(PlannedRole::Cover, Some(&standing), 4),
            PlannedRole::Cover,
            "the ceiling never promotes anybody"
        );
    }

    #[test]
    fn a_coach_can_trust_a_squad_player_without_calling_him_a_cornerstone() {
        let standing = Fx::at(StandingRung::Trusted);
        assert_eq!(
            StandingRead::bound_role(PlannedRole::Cover, Some(&standing), 4),
            PlannedRole::Cover,
            "fourth choice is fourth choice"
        );
        assert_eq!(
            StandingRead::bound_role(PlannedRole::Rotation, Some(&standing), 1),
            PlannedRole::Starter
        );
    }

    #[test]
    fn a_man_he_will_not_leave_out_is_the_man_he_builds_around() {
        let standing = Fx::at(StandingRung::Undroppable);
        assert_eq!(
            StandingRead::bound_role(PlannedRole::Starter, Some(&standing), 0),
            PlannedRole::Cornerstone
        );
    }

    #[test]
    fn a_debt_is_paid_in_a_game_the_coach_can_afford_to_lose() {
        let mut standing = Fx::at(StandingRung::UnderReview);
        standing.debt = 0.6;
        let context = LadderContext::default();
        StandingLadder::settle(&mut standing, &context, Fx::date());

        let cup_tie = StandingRead::for_selection(Some(&standing), false, 0.4, 0.8, Fx::date());
        let title_decider =
            StandingRead::for_selection(Some(&standing), false, 0.95, 0.8, Fx::date());
        assert!(cup_tie.owed_a_start);
        assert!(!title_decider.owed_a_start);
    }
}
