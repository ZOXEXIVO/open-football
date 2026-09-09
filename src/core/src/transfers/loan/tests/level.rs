//! Moved verbatim out of `loan_market.rs` — see that file's `mod loan_destination_level_tests`.

use super::super::*;

/// A top-flight parent and the divisions below it, on the reputation
/// scale the compiled database actually uses for Russia: Premier League
/// 6500, First Division 4500, third tier ~3000.
struct Fx;

impl Fx {
    const TOP_FLIGHT: u16 = 6500;
    const SECOND_TIER: u16 = 4500;
    const THIRD_TIER: u16 = 3000;
    /// Best current ability in the loanee's position group at the parent.
    const PARENT_BEST: u8 = 136;

    /// A loan from a top-flight giant (rep 7600) to a club of
    /// `borrower_rep` playing in `borrower_league_rep`.
    fn to(
        ability: u8,
        borrower_league_rep: u16,
        borrower_rep: u16,
        is_development: bool,
    ) -> LoanDestinationLevel {
        LoanDestinationLevel {
            ability,
            parent_best_in_group: Self::PARENT_BEST,
            parent_rep: 7600,
            borrower_rep,
            parent_league_rep: Self::TOP_FLIGHT,
            borrower_league_rep,
            is_development,
        }
    }
}

/// The Litvinov destination. A centre-back already competing for his
/// parent's first team is not loaned into the division below — and now
/// BOTH halves say so, because the club-standing floor is continuous
/// in readiness too: a man at his parent's own level goes to a peer or
/// he does not go. A borrower at 42 % of a giant's standing is not a
/// peer.
#[test]
fn near_ready_regular_is_not_loaned_a_division_down() {
    let drop = Fx::to(122, Fx::SECOND_TIER, 3200, false);
    assert!(!drop.clears_club_standing());
    assert!(!drop.clears_division());
    assert!(!drop.is_plausible());
}

/// …but a sideways move to a PEER is fine: same division, comparable
/// standing. This is the destination the doctrine leaves open for him,
/// and the one the flat 0.25 standing floor used to share with clubs a
/// quarter of his parent's size.
#[test]
fn near_ready_regular_may_loan_to_a_peer_in_his_own_division() {
    assert!(Fx::to(122, Fx::TOP_FLIGHT, 6000, false).is_plausible());
    assert!(
        !Fx::to(122, Fx::TOP_FLIGHT, 3200, false).is_plausible(),
        "same division is not the same level"
    );
}

/// The development pathway survives, keyed to how raw he actually is
/// rather than to his birth year: one division down for a prospect who
/// is some way off his parent's standard, two for one who is genuinely
/// years away.
#[test]
fn raw_prospect_still_drops_for_minutes() {
    assert!(Fx::to(95, Fx::SECOND_TIER, 3200, true).is_plausible());
    assert!(Fx::to(85, Fx::THIRD_TIER, 3200, true).is_plausible());
    assert!(
        !Fx::to(120, Fx::THIRD_TIER, 3200, true).is_plausible(),
        "a near-ready player gets none of the development allowance"
    );
}

/// And a genuine fringe senior — clearly short of his parent's standard,
/// but not a prospect — still drops one division on an ordinary cover
/// loan.
#[test]
fn fringe_senior_still_drops_one_division() {
    assert!(Fx::to(100, Fx::SECOND_TIER, 3200, false).is_plausible());
    assert!(!Fx::to(100, Fx::THIRD_TIER, 3200, false).clears_division());
}

/// The three readiness stops the WI-5 curve is specified at. The
/// allowance a drop earns is continuous in readiness rather than
/// switched on by a birth year, so a raw player keeps today's full
/// width and a first-team-ready one is held to his parent's level.
#[test]
fn the_division_floor_tracks_readiness_at_every_stop() {
    // readiness 0: exactly the old raw floor with the full allowance.
    let raw = Fx::to(
        (Fx::PARENT_BEST as f32 * 0.60).round() as u8,
        Fx::SECOND_TIER,
        3200,
        true,
    );
    assert!(
        (raw.division_floor() - 0.45 * 0.75).abs() < 0.01,
        "raw reads {}",
        raw.division_floor()
    );

    // readiness 0.6: the mid-pathway prospect, comfortably clear of
    // the Segunda-to-La-Liga ratio of 0.707.
    let mid = Fx::to(
        (Fx::PARENT_BEST as f32 * (0.60 + 0.6 * 0.30)).round() as u8,
        Fx::SECOND_TIER,
        3200,
        true,
    );
    let mid_floor = mid.division_floor();
    assert!(
        (0.60..0.64).contains(&mid_floor),
        "0.6 readiness reads {mid_floor}"
    );
    assert!(mid_floor < 0.707);

    // readiness 1: no allowance at all, and 0.707 fails — the Yamal
    // destination, refused on the division alone.
    let ready = Fx::to(Fx::PARENT_BEST, Fx::SECOND_TIER, 3200, true);
    assert!((ready.division_floor() - 0.85).abs() < 0.01);
    assert!(ready.division_floor() > 0.707);
}

/// The readiness curve is continuous — the further a player is from
/// his parent's best, the further he may drop.
#[test]
fn division_floor_falls_as_the_player_gets_rawer() {
    let floor = |ability| Fx::to(ability, Fx::SECOND_TIER, 3200, false).division_floor();
    assert!(floor(122) > floor(110));
    assert!(floor(110) > floor(95));
    assert!(floor(95) > floor(80));
}

/// A club with no league of its own (a friendly-only side) leaves the
/// division gate with nothing to judge, so it stands down and the
/// club-standing gate owns the decision rather than guessing.
#[test]
fn unknown_division_stands_down() {
    assert!(Fx::to(122, 0, 3200, false).clears_division());
    let mut no_parent_league = Fx::to(122, Fx::SECOND_TIER, 3200, false);
    no_parent_league.parent_league_rep = 0;
    assert!(no_parent_league.clears_division());
}
