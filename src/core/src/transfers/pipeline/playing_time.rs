//! L6 — will I play?
//!
//! Personal terms read the wage, the stage, the badge and the player's own
//! ambition. What they never read is the one question a footballer weighing
//! a move actually asks first: *am I going to be in the team?* So a key
//! player at a mid-table club would sign for a giant's bench on the strength
//! of the money alone, and a squad man offered a starting shirt somewhere
//! smaller saw no reason to take it.
//!
//! The buyer already says what it is offering — `PromisedSquadStatus` on the
//! personal-terms package, now sourced from the recruitment brief's tier. The
//! seller side already knows what he currently is: `importance`, the same
//! 0..1 read the fee resolver uses. This turns the gap between the two into a
//! term on the acceptance roll.
//!
//! It is continuous in both directions and it is compensable. A bigger stage
//! or a much bigger wage can still buy a bench seat — that is how the real
//! version of this decision works, and it is why the term is a weight rather
//! than a veto.

use crate::transfers::offer::PromisedSquadStatus;

/// The shirt a LOAN offer promises.
///
/// A loan carried no promise at all, so the appraisal read
/// [`PlayingTimeExpectation::promised_standing`]`(None)` = 0.50 — "he will
/// have to earn it" — on the one kind of move whose entire purpose is
/// minutes. The Part V loan rows are written against a Regular promise
/// (`R` +0.23); the shipped path was giving +0.13.
///
/// The borrower has already answered the question by the time it makes the
/// offer: `BorrowerDepth::would_get_loan_minutes` is a gate every loan
/// clears, and it runs at its stricter bar on a development loan. So a
/// development loan promises a regular starting place and a cover loan
/// promises rotation, which is exactly what the two are for.
pub struct LoanPromise;

impl LoanPromise {
    pub fn for_loan(is_development: bool) -> Option<PromisedSquadStatus> {
        Some(if is_development {
            PromisedSquadStatus::FirstTeamRegular
        } else {
            PromisedSquadStatus::FirstTeamSquadRotation
        })
    }
}

/// The playing-time term on personal terms.
pub struct PlayingTimeExpectation;

impl PlayingTimeExpectation {
    /// Acceptance points a full demotion costs an indispensable player who
    /// is being offered nothing else. Large enough to turn a marginal yes
    /// into a no, small enough that a real step up in stage still carries
    /// the move.
    pub const PT_PENALTY: f32 = 25.0;
    /// Acceptance points a full promotion is worth. Smaller than the
    /// penalty, deliberately: losing a starting shirt hurts more than
    /// gaining one delights, which is what the transfer market's revealed
    /// behaviour looks like.
    pub const PT_BONUS: f32 = 15.0;
    /// League-reputation gain at which the stage fully compensates for a
    /// smaller role. Two thousand points is roughly the distance from a
    /// strong sub-elite league to a top-five one — the move for which
    /// players genuinely accept a bench.
    const STAGE_SPAN: f32 = 2000.0;
    /// Wage multiple at which the money fully compensates. Doubling the
    /// salary is the going rate for accepting a smaller role.
    const WAGE_SPAN: f32 = 1.0;

    /// Where a promised shirt sits on the same 0..1 scale the seller-side
    /// importance read uses, so the two can simply be subtracted.
    ///
    /// No promise at all reads as the middle of the range: the club has not
    /// said, and a player with no assurance assumes he will have to earn it.
    pub fn promised_standing(promised: Option<&PromisedSquadStatus>) -> f32 {
        match promised {
            Some(PromisedSquadStatus::KeyPlayer) => 0.95,
            Some(PromisedSquadStatus::FirstTeamRegular) => 0.75,
            Some(PromisedSquadStatus::FirstTeamSquadRotation) => 0.45,
            Some(PromisedSquadStatus::HotProspectForTheFuture) => 0.30,
            None => 0.50,
        }
    }

    /// How much a bigger stage and a bigger wage make up for a smaller role.
    /// 0 = nothing on offer but the shirt; 1 = the move is worth it whatever
    /// the role.
    pub fn compensation(
        selling_league_reputation: u16,
        buying_league_reputation: u16,
        current_wage: f64,
        offered_wage: f64,
    ) -> f32 {
        let stage_gain = ((buying_league_reputation as f32 - selling_league_reputation as f32)
            / Self::STAGE_SPAN)
            .clamp(0.0, 1.0);
        let wage_gain = if current_wage > 0.0 {
            (((offered_wage / current_wage) - 1.0) as f32 / Self::WAGE_SPAN).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (stage_gain + wage_gain).clamp(0.0, 1.0)
    }

    /// The acceptance delta. Positive when the move is a promotion, negative
    /// when it asks him to accept less than he already has.
    ///
    /// `importance` is the seller-side 0..1 read of what he is at his
    /// current club — the same number the fee resolver prices his premium
    /// with, so the two sides of the deal are describing the same player.
    pub fn terms_delta(
        importance: f32,
        promised: Option<&PromisedSquadStatus>,
        compensation: f32,
    ) -> f32 {
        let offered = Self::promised_standing(promised);
        let delta = offered - importance.clamp(0.0, 1.0);
        if delta >= 0.0 {
            // A player being offered more than he has takes it — and the
            // further up the offer reaches, the more it moves him.
            Self::PT_BONUS * delta
        } else {
            // A demotion costs most to the man who has most to lose, and
            // costs nothing at all when the stage or the money already
            // makes up for it.
            -Self::PT_PENALTY
                * importance.clamp(0.0, 1.0)
                * (-delta)
                * (1.0 - compensation).clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod playing_time_tests {
    use super::*;

    #[test]
    fn a_key_player_offered_a_bench_seat_for_nothing_else_says_no() {
        let delta = PlayingTimeExpectation::terms_delta(
            0.95,
            Some(&PromisedSquadStatus::FirstTeamSquadRotation),
            0.0,
        );
        assert!(delta < -10.0, "{delta}");
    }

    #[test]
    fn the_same_offer_from_a_far_bigger_stage_costs_him_nothing() {
        let compensation = PlayingTimeExpectation::compensation(6000, 9000, 1.0, 1.0);
        let delta = PlayingTimeExpectation::terms_delta(
            0.95,
            Some(&PromisedSquadStatus::FirstTeamSquadRotation),
            compensation,
        );
        assert!(
            delta.abs() < 1e-6,
            "a big enough stage buys a bench seat: {delta}"
        );
    }

    #[test]
    fn doubling_the_wage_buys_most_of_the_way_there_too() {
        let poor = PlayingTimeExpectation::compensation(7000, 7000, 1_000_000.0, 1_050_000.0);
        let rich = PlayingTimeExpectation::compensation(7000, 7000, 1_000_000.0, 2_000_000.0);
        assert!(rich > poor);
        assert!((rich - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_backup_offered_a_starting_shirt_is_moved_by_it() {
        let delta =
            PlayingTimeExpectation::terms_delta(0.25, Some(&PromisedSquadStatus::KeyPlayer), 0.0);
        assert!(delta > 5.0, "{delta}");
    }

    #[test]
    fn an_offer_that_matches_his_standing_is_neutral() {
        let delta = PlayingTimeExpectation::terms_delta(
            0.75,
            Some(&PromisedSquadStatus::FirstTeamRegular),
            0.0,
        );
        assert!(delta.abs() < 1e-5, "{delta}");
    }

    #[test]
    fn a_club_that_promises_nothing_is_read_as_the_middle_of_the_range() {
        assert!(PlayingTimeExpectation::terms_delta(0.95, None, 0.0) < 0.0);
        assert!(PlayingTimeExpectation::terms_delta(0.20, None, 0.0) > 0.0);
    }

    #[test]
    fn the_penalty_is_bounded_by_its_own_constant() {
        let worst = PlayingTimeExpectation::terms_delta(
            1.0,
            Some(&PromisedSquadStatus::HotProspectForTheFuture),
            0.0,
        );
        assert!(worst >= -PlayingTimeExpectation::PT_PENALTY);
    }
}
