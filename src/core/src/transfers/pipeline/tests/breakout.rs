//! Moved verbatim out of `helpers.rs` — see that file's `mod breakout_sweep_tests`.

//! The performance-breakout discovery path through
//! [`ListedTargetScreen::evaluate`]: a loan-listed (or, in form-discovery
//! mode, an unlisted) high-form player becomes a target for stronger
//! clubs — without the breakout ever relaxing the affordability, tier,
//! reputation, or squad-need gates. Mirrors the reported Arseny Filev
//! case: a 22-y-o striker top-scoring a second division, loan-listed
//! over a contract dispute, who should still draw realistic interest.

use crate::PlayerFieldPositionGroup;
use crate::transfers::gate::fit::SquadFitSnapshot;
use crate::transfers::pipeline::advice::{
    BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
};
use crate::transfers::scouting::breakout::BreakoutPerformanceSignal;

/// Fixtures wrapped in a unit struct per the no-free-helpers
/// convention. Each accessor returns a baseline the test tweaks.
struct BreakoutFixtures;

impl BreakoutFixtures {
    /// A strong top-flight domestic club (Continental-ish). `weak_group`
    /// forces a positional need; otherwise the group is well-stocked.
    fn top_domestic_buyer(weak_group: bool) -> BuyerContext {
        BuyerContext {
            buyer_rep_score: 0.72,
            plan_total_budget: 30_000_000.0,
            buyer_best_in_group: if weak_group { 118 } else { 135 },
            has_open_request: false,
            has_aging_starter: false,
            form_discovery_mode: false,
            fit: SquadFitSnapshot::disabled(),
        }
    }

    /// The reported player: 22-y-o striker, loan-listed (not transfer
    /// listed / requested / unhappy), a genuine breakout, with resale
    /// upside, comfortably affordable, in a smaller club.
    fn loan_listed_breakout_striker() -> ListedTargetView {
        ListedTargetView {
            nationality_country_id: 0,
            ability: 130,
            estimated_potential: 142,
            age: 22,
            estimated_value: 5_000_000.0,
            position_group: PlayerFieldPositionGroup::Forward,
            is_listed: false,
            is_transfer_requested: false,
            is_unhappy: false,
            is_loan_listed: true,
            breakout_score: 55.0,
            world_reputation: 5000,
            ambition: 0.7,
            parent_club_score: 0.40,
            parent_club_in_debt: false,
            days_available: 5,
            contract_months_remaining: 24,
            low_usage: false,
            recent_interest_count: 0,
            failed_scans: 0,
        }
    }
}

#[test]
fn loan_listed_breakout_striker_is_visible_to_a_top_domestic_club() {
    // Req: a loan-listed breakout striker at a lower-rep club is visible
    // to top domestic clubs. No open positional need — admission and the
    // opportunity route ride entirely on the loan-listing + breakout +
    // resale upside. This is also the "window open" case: the in-window
    // listed sweep (form_discovery_mode = false) admits him.
    let target = BreakoutFixtures::loan_listed_breakout_striker();
    let buyer = BreakoutFixtures::top_domestic_buyer(false);
    match ListedTargetScreen::evaluate(&target, &buyer) {
        ListedTargetVerdict::Accept(score) => {
            assert!(score > 10.0, "expected a meaningful score, got {}", score)
        }
        other => panic!(
            "expected Accept for a loan-listed breakout, got {:?}",
            other
        ),
    }
}

#[test]
fn mediocre_loan_listed_player_without_breakout_is_ignored() {
    // Req: a mediocre loan-listed player with no goals / awards stays
    // ignored — loan-listing alone routes to the loan market, not the
    // permanent-interest path.
    let mut target = BreakoutFixtures::loan_listed_breakout_striker();
    target.breakout_score = 0.0; // no output, no recognition
    target.estimated_potential = 128; // no resale upside either
    let buyer = BreakoutFixtures::top_domestic_buyer(true);
    assert_eq!(
        ListedTargetScreen::evaluate(&target, &buyer),
        ListedTargetVerdict::Reject(ListedRejectReason::NotListed),
        "a loan-listed player without breakout must not enter the permanent path"
    );
}

#[test]
fn elite_club_skips_breakout_player_who_is_no_upgrade_no_resale_no_need() {
    // Req: an elite club does NOT pursue even a high-breakout player when
    // he is not an upgrade, has no resale value, and fills no squad need.
    // The breakout score does not manufacture a reason to buy.
    let elite = BuyerContext {
        buyer_rep_score: 0.90,
        plan_total_budget: 150_000_000.0,
        buyer_best_in_group: 165, // well-stocked
        has_open_request: false,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };
    // In the elite tier window, publicly listed, but a 28-y-o who is no
    // upgrade (135 < 165) and no resale prospect (age > 23).
    let mut target = BreakoutFixtures::loan_listed_breakout_striker();
    target.ability = 135;
    target.age = 28;
    target.estimated_potential = 137;
    target.is_listed = true;
    target.is_loan_listed = false;
    target.world_reputation = 6000;
    target.breakout_score = 55.0;
    assert_eq!(
        ListedTargetScreen::evaluate(&target, &elite),
        ListedTargetVerdict::Reject(ListedRejectReason::NoSquadNeed),
        "breakout must not bypass the upgrade / resale / need requirement"
    );
}

#[test]
fn availability_sweep_admits_an_unlisted_breakout_only_in_form_discovery() {
    // A not-yet-listed breakout is not on the in-window availability sweep
    // for anybody, however much bigger the buyer: whether a club may pursue
    // him is the gate's question. The year-round watch admits him for
    // monitoring on form alone.
    let mut target = BreakoutFixtures::loan_listed_breakout_striker();
    target.is_loan_listed = false; // not on any list at all
    target.breakout_score = 60.0;

    for rep in [0.72, 0.45] {
        let mut buyer = BreakoutFixtures::top_domestic_buyer(true);
        buyer.buyer_rep_score = rep;
        assert_eq!(
            ListedTargetScreen::evaluate(&target, &buyer),
            ListedTargetVerdict::Reject(ListedRejectReason::NotListed),
            "an unlisted player is not pursued in-window on form alone"
        );
    }

    let mut watch_buyer = BreakoutFixtures::top_domestic_buyer(true);
    watch_buyer.form_discovery_mode = true;
    assert!(
        matches!(
            ListedTargetScreen::evaluate(&target, &watch_buyer),
            ListedTargetVerdict::Accept(_)
        ),
        "form-discovery mode must admit an unlisted breakout for monitoring"
    );
}

#[test]
fn breakout_threshold_governs_loan_listed_admission() {
    // The admission bar is exactly the breakout threshold: a loan-listed
    // player just below it stays loan-only; at/above it he enters the
    // permanent-interest path.
    let buyer = BreakoutFixtures::top_domestic_buyer(true);

    let mut below = BreakoutFixtures::loan_listed_breakout_striker();
    below.breakout_score = BreakoutPerformanceSignal::BREAKOUT_THRESHOLD - 0.1;
    assert_eq!(
        ListedTargetScreen::evaluate(&below, &buyer),
        ListedTargetVerdict::Reject(ListedRejectReason::NotListed),
        "just below the breakout bar a loan-listed player stays loan-only"
    );

    let mut at_bar = BreakoutFixtures::loan_listed_breakout_striker();
    at_bar.breakout_score = BreakoutPerformanceSignal::BREAKOUT_THRESHOLD;
    assert!(
        matches!(
            ListedTargetScreen::evaluate(&at_bar, &buyer),
            ListedTargetVerdict::Accept(_)
        ),
        "at the breakout bar a loan-listed player enters the permanent path"
    );
}
