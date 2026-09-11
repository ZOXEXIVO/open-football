//! Moved verbatim out of `loan_market.rs` — see that file's `mod borrower_gate_tests`.

use super::super::*;
use crate::club::player::builder::PlayerBuilder;
use crate::shared::fullname::FullName;
use crate::transfers::loan::LoanPipeline;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, TeamReputation, TeamType,
    TrainingSchedule,
};
use chrono::{NaiveDate, NaiveTime};

/// Borrower-side fixtures. Wrapped in a unit struct per the
/// project's no-free-helpers convention.
struct BorrowerFixtures;

impl BorrowerFixtures {
    fn player(id: u32, position: PlayerPositionType, ca: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Loan".to_string(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    // Natural. These fixtures stand for "the club's man in
                    // that shirt", and the depth gates now read ability
                    // through `RoleFamiliarity` — an accomplished-but-not-
                    // natural 16 would quietly shade every incumbent down
                    // and make the fixtures test the discount rather than
                    // the gate.
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn team(players: Vec<Player>) -> Team {
        Team::builder()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Borrower".to_string())
            .slug("borrower".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(2000, 2000, 2000))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }
}

/// WI-5: the minutes gate had no UPPER bound. It asked only "would he
/// play here", and a loanee forty points better than the borrower's
/// best is the strongest possible yes — which is precisely the
/// mismatch that made a second-division side the most attractive
/// destination in the country for a giant's first-choice teenager.
#[test]
fn an_absurdly_overqualified_loanee_is_a_mismatch_not_a_minutes_loan() {
    let depth = BorrowerPositionDepth::snapshot(&BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::Striker, 120),
        BorrowerFixtures::player(2, PlayerPositionType::Striker, 115),
    ]));
    // Ready for his parent's own first team (176 against a 176 best):
    // the bound is at its tightest, 25 over the borrower's best.
    assert!(
        !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Forward, 176, false, 176),
        "56 CA over the borrower's best is not a minutes loan"
    );
    assert!(
        depth.would_get_loan_minutes(PlayerFieldPositionGroup::Forward, 140, false, 176),
        "a 20-point upgrade is exactly the loan that should happen"
    );
    // A raw prospect gets the wider bound: dropping below his own
    // level IS his pathway.
    assert!(
        depth.would_get_loan_minutes(PlayerFieldPositionGroup::Forward, 150, true, 245),
        "a genuinely raw loanee keeps the wider allowance"
    );
    // An unknown parent standard stands the bound down rather than
    // guessing, exactly as an unknown competition does.
    assert!(depth.would_get_loan_minutes(PlayerFieldPositionGroup::Forward, 176, false, 0));
}
#[test]
fn full_group_rejects_comparable_loan_but_accepts_clear_upgrade() {
    // Three keepers fill the GK cap; a comparable 4th is bloat, a
    // clear upgrade still gets through.
    let team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::Goalkeeper, 80),
        BorrowerFixtures::player(2, PlayerPositionType::Goalkeeper, 70),
        BorrowerFixtures::player(3, PlayerPositionType::Goalkeeper, 60),
    ]);
    let depth = BorrowerPositionDepth::snapshot(&team);
    assert!(
        !depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 85, false),
        "comparable keeper into a full group is squad bloat"
    );
    assert!(
        depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 95, false),
        "a clear upgrade (≥10 over the best) is still allowed"
    );
}

#[test]
fn gk_loan_blocked_when_clearly_better_keeper_present() {
    let team = BorrowerFixtures::team(vec![BorrowerFixtures::player(
        1,
        PlayerPositionType::Goalkeeper,
        80,
    )]);
    let depth = BorrowerPositionDepth::snapshot(&team);
    assert!(
        !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Goalkeeper, 70, false, 0),
        "a dev keeper behind a clearly better #1 plays zero minutes"
    );
    assert!(
        depth.would_get_loan_minutes(PlayerFieldPositionGroup::Goalkeeper, 75, false, 0),
        "a keeper close to the incumbent can compete for the shirt"
    );
}

#[test]
fn outfield_loan_blocked_behind_three_clearly_better_players() {
    let blocked_team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 90),
        BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 90),
        BorrowerFixtures::player(3, PlayerPositionType::MidfielderCenter, 90),
    ]);
    let depth = BorrowerPositionDepth::snapshot(&blocked_team);
    assert!(
        !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, false, 0),
        "three clearly better midfielders leave no realistic minutes"
    );

    let open_team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 90),
        BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 90),
        BorrowerFixtures::player(3, PlayerPositionType::MidfielderCenter, 72),
    ]);
    let depth = BorrowerPositionDepth::snapshot(&open_team);
    assert!(
        depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, false, 0),
        "with only two clearly better names the loanee can rotate in"
    );
}

#[test]
fn development_loans_demand_stricter_minutes_than_generic_cover() {
    // Two clearly better midfielders: fine for emergency cover,
    // not for a development loan that exists to buy starts.
    let team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 90),
        BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 90),
    ]);
    let depth = BorrowerPositionDepth::snapshot(&team);
    assert!(
        depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, false, 0),
        "generic cover tolerates two better names"
    );
    assert!(
        !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, true, 0),
        "a development loanee behind two starters won't get his minutes"
    );
}

#[test]
fn pending_incoming_loan_counts_against_the_position_cap() {
    // Two keepers on the books leave one slot under the GK cap of 3,
    // so a comparable keeper would normally be allowed in.
    let team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::Goalkeeper, 80),
        BorrowerFixtures::player(2, PlayerPositionType::Goalkeeper, 70),
    ]);
    let bare = BorrowerPositionDepth::snapshot(&team);
    assert!(
        bare.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 78, false),
        "two keepers leave room for a third under the cap"
    );

    // A keeper loan already in flight fills that last slot: the same
    // comparable target is now squad bloat, while a clear upgrade still
    // gets through. This is what stops a club opening several keeper
    // loans against one unchanged snapshot.
    let with_pending = BorrowerPositionDepth::snapshot(&team)
        .with_pending_loans(&[(PlayerFieldPositionGroup::Goalkeeper, 72)]);
    assert!(
        !with_pending.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 78, false),
        "a pending keeper loan fills the cap — no second comparable loan"
    );
    assert!(
        with_pending.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 95, false),
        "a clear upgrade is still allowed even with a loan in flight"
    );
}

/// A development keeper loan may join a full-but-weak GK line: a giant's
/// prospect who is a clear upgrade on the borrower's WEAKEST keeper gets
/// in (and `would_get_loan_minutes` guarantees he competes), where the
/// strict cover bar would have demanded an impossible +10 over their best.
#[test]
fn development_keeper_joins_full_but_weak_line() {
    // A weak, full three-keeper line (best 78) — no prospect could ever
    // clear best+10 = 88, so cover loans are locked out entirely.
    let team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::Goalkeeper, 78),
        BorrowerFixtures::player(2, PlayerPositionType::Goalkeeper, 75),
        BorrowerFixtures::player(3, PlayerPositionType::Goalkeeper, 72),
    ]);
    let depth = BorrowerPositionDepth::snapshot(&team);

    // Cover loan: still blocked (85 < best 78 + 10).
    assert!(
        !depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 85, false),
        "a cover keeper still needs the clear +10 upgrade"
    );
    // Development loan: an 85 prospect clears the weakest (72) + 8 = 80,
    // so he joins to compete for the shirt.
    assert!(
        depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 85, true),
        "a development keeper who clearly beats the fringe keeper joins a full weak line"
    );
    // A marginal keeper who barely beats the fringe keeper is still bloat.
    assert!(
        !depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 79, true),
        "a keeper who is not clearly better than the weakest is still squad bloat"
    );
}

/// The development relaxation admits only ONE over-cap keeper: once a loan
/// is inbound (folded in via `with_pending_loans`, pushing the count past
/// the cap) the strict bar returns, so a club can't stockpile loanees.
#[test]
fn development_keeper_relaxation_is_bounded_to_one_over_cap() {
    let team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::Goalkeeper, 78),
        BorrowerFixtures::player(2, PlayerPositionType::Goalkeeper, 75),
        BorrowerFixtures::player(3, PlayerPositionType::Goalkeeper, 72),
    ]);
    // One keeper loan already in flight → count is now 4 (> cap 3).
    let with_pending = BorrowerPositionDepth::snapshot(&team)
        .with_pending_loans(&[(PlayerFieldPositionGroup::Goalkeeper, 85)]);
    assert!(
        !with_pending.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 84, true),
        "a second development keeper is blocked once one is already inbound"
    );
}

/// The relaxation is keeper-specific: a full outfield line keeps the
/// strict clear-upgrade bar even for development loans.
#[test]
fn development_relaxation_does_not_apply_to_outfield() {
    let team = BorrowerFixtures::team(vec![
        BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 78),
        BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 75),
        BorrowerFixtures::player(3, PlayerPositionType::MidfielderCenter, 72),
        BorrowerFixtures::player(4, PlayerPositionType::MidfielderCenter, 70),
        BorrowerFixtures::player(5, PlayerPositionType::MidfielderCenter, 68),
        BorrowerFixtures::player(6, PlayerPositionType::MidfielderCenter, 66),
        BorrowerFixtures::player(7, PlayerPositionType::MidfielderCenter, 64),
        BorrowerFixtures::player(8, PlayerPositionType::MidfielderCenter, 62),
    ]);
    let depth = BorrowerPositionDepth::snapshot(&team);
    // Full midfield (cap 8, best 78): a development midfielder still needs
    // the clear +10 (≥ 88), unlike keepers.
    assert!(
        !depth.has_room_for(PlayerFieldPositionGroup::Midfielder, 85, true),
        "outfield development loans keep the strict full-line bar"
    );
}

#[test]
fn reputation_drop_gate_blocks_giant_to_minnow_unless_player_is_raw() {
    // Established player (close to the parent's best): a 2000-rep
    // borrower is too far below a 9000-rep parent.
    assert!(!LoanPipeline::loan_reputation_drop_ok(
        2000, 9000, 120, 130, false
    ));
    // Same borrower is fine for a genuinely raw player — any senior
    // football is the point of the loan. The floor tracks readiness
    // continuously now rather than switching on a "very raw" flag, so
    // this is the man who is years off the shirt, not merely below it.
    assert!(LoanPipeline::loan_reputation_drop_ok(
        2000, 9000, 80, 130, false
    ));
    // A PEER-level borrower clears the floor for the established
    // player — that is the destination doctrine leaves him.
    assert!(LoanPipeline::loan_reputation_drop_ok(
        7000, 9000, 120, 130, false
    ));
    // …and a mid-table one does not: a man at his club's own level
    // goes sideways or stays.
    assert!(!LoanPipeline::loan_reputation_drop_ok(
        3000, 9000, 120, 130, false
    ));
    // Unknown parent reputation never blocks.
    assert!(LoanPipeline::loan_reputation_drop_ok(
        2000, 0, 120, 130, false
    ));
    // Readiness, not age, decides the drop. The development flag no longer
    // lifts the floor for a NEAR-READY player: a young displaced first-
    // choice (close to the parent's best) is held to the same peer-level
    // floor as an established player, so the giant-to-minnow drop is
    // blocked — he moves down a tier or two, not several.
    assert!(!LoanPipeline::loan_reputation_drop_ok(
        2000, 9000, 120, 130, true
    ));
    // A genuinely raw development player still has the floor lifted
    // entirely — any senior football is the point, and the minutes gate is
    // the realism check instead.
    assert!(LoanPipeline::loan_reputation_drop_ok(
        400, 9000, 90, 130, true
    ));
}

#[test]
fn foreign_loan_country_rep_gate_lifts_for_development_step_down() {
    // Higher-reputation nation → lower (e.g. Russia → Belarus): an
    // established fringe player can't drop a national tier on loan.
    assert!(!LoanPipeline::foreign_loan_country_rep_ok(
        7000, 5000, false
    ));
    // ...but a development-profile youngster going abroad for senior
    // minutes is exactly the move the gate is meant to permit. The
    // region-prestige and club-rep gates still bound how far he falls.
    assert!(LoanPipeline::foreign_loan_country_rep_ok(7000, 5000, true));
    // Equal-or-lower-reputation source never trips the gate regardless
    // of profile — there's no step-down to guard against.
    assert!(LoanPipeline::foreign_loan_country_rep_ok(5000, 7000, false));
    assert!(LoanPipeline::foreign_loan_country_rep_ok(5000, 5000, false));
}

#[test]
fn foreign_loan_region_gate_lifts_for_development_step_down() {
    // Italy (Western Europe, 1.0) → Romania/Russia (Eastern Europe, 0.50).
    // A settled player won't loan down two prestige bands for a bit-part
    // role — the 0.50 gap exceeds the 0.20 cover allowance.
    assert!(!LoanPipeline::foreign_loan_region_ok(1.0, 0.50, false));
    // ...but a development youngster going abroad for senior minutes is
    // exactly the "go abroad to play" move the region gate used to block —
    // the wider development allowance clears the gap. The downstream
    // club-rep band still bounds how far he actually falls.
    assert!(LoanPipeline::foreign_loan_region_ok(1.0, 0.50, true));
    // The development lift stays bounded: a top-region prospect still can't
    // reach the very bottom regions (e.g. South Asia, 0.10) from 1.0.
    assert!(!LoanPipeline::foreign_loan_region_ok(1.0, 0.10, true));
    // Moving to an equal-or-more-prestigious region is never blocked.
    assert!(LoanPipeline::foreign_loan_region_ok(0.50, 1.0, false));
}
