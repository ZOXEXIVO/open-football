//! Scenario tests for the whole boardroom: archetype governance,
//! season-phase sacking protection, FFP reactions, takeovers, promises and
//! the owner's money reaching the market.

use super::*;
use crate::MatchTacticType;
use crate::club::board::chairman::{ChairmanAmbition, ChairmanPatience};
use crate::club::board::context::FfpStatus;
use crate::club::board::governance::{
    BoardDossierSummary, BoardTransferConcern, BoardTransferDecision, BoardTransferEconomics,
    BoardTransferProposal,
};
use crate::club::board::infrastructure::FacilityReview;
use crate::club::board::pressure::SupporterEvent;
use crate::club::board::promise::{BoardPromise, PromiseType};
use crate::club::board::relationship::ManagerRelationship;
use crate::club::board::roll::DeterministicRoll;
use crate::club::board::sale::ForcedSaleTarget;
use crate::club::board::scoring::SeasonPhase;
use crate::club::board::strategy::{
    InfrastructurePriority, ManagerAutonomy, ReviewFrequency, SquadProfile,
};
use crate::club::board::takeover::TakeoverStatus;
use crate::club::board::vision::{
    ClubVision, FinancialStance, LongTermGoal, VisionPlayingStyle, VisionYouthFocus,
};
use crate::club::board::{BoardFacility, BoardManagerMeeting, BoardMoodState};
use crate::club::finance::{DebtStanding, SeasonTransferFees};
use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason};
use chrono::{Duration, NaiveDate};

// Scenario tests for the expanded board: archetype governance,
// season-phase sacking protection, FFP reactions, takeovers, and
// manager-relationship renewals. These exercise the integrated
// `evaluate_performance` / governance / takeover paths end to end.
use crate::club::board::ownership::{OwnershipModel, OwnershipType};
use crate::club::board::severance::Severance;

/// Boardroom scenarios the tests below are built out of.
struct Scenario;

impl Scenario {
    fn targets(expected: u8, min_acceptable: u8) -> SeasonTargets {
        SeasonTargets {
            transfer_budget: 30_000_000,
            wage_budget: 50_000_000,
            max_squad_size: 30,
            min_squad_size: 18,
            expected_position: expected,
            min_acceptable_position: min_acceptable,
            ..Default::default()
        }
    }

    fn poor_ctx(matches_played: u8, total: u8, position: u8, size: u8) -> BoardContext {
        let mut c = BoardContext::new();
        c.total_annual_wages = 12_000_000;
        c.balance = -5_000_000;
        c.league_size = size;
        c.league_position = position;
        c.matches_played = matches_played;
        c.total_matches = total;
        c.points_per_match = 0.5;
        c.recent_wins = 0;
        c.recent_losses = 4;
        c.recent_goal_difference = -8;
        c.goal_difference = -25;
        c.distance_to_relegation = -1;
        c
    }

    fn strong_ctx(position: u8, size: u8) -> BoardContext {
        let mut c = BoardContext::new();
        c.total_annual_wages = 12_000_000;
        c.balance = 30_000_000;
        c.league_size = size;
        c.league_position = position;
        c.matches_played = 19;
        c.total_matches = 38;
        c.points_per_match = 2.2;
        c.recent_wins = 4;
        c.recent_losses = 0;
        c.recent_goal_difference = 8;
        c.goal_difference = 25;
        c.distance_to_relegation = 15;
        c.profit_loss_12m = 5_000_000;
        c
    }

    fn proposal(
        fee: f64,
        age: u8,
        ability: u8,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
    ) -> BoardTransferProposal {
        BoardTransferProposal {
            fee,
            allocated_budget: 1_000_000.0,
            remaining_transfer_budget: 10_000_000.0,
            priority,
            reason,
            player_age: Some(age),
            player_ability: Some(ability),
            squad_avg_ability: 65,
            shortlist_score: 1.0,
            dossier: None,
            economics: None,
        }
    }

    /// Build a small / poor member-owned board to take over.
    fn member_owned_board() -> ClubBoard {
        let mut board = ClubBoard::new();
        board.ownership = OwnershipModel {
            ownership_type: OwnershipType::MemberOwned,
            base_wealth: 25,
            interference: 20,
            risk_tolerance: 25,
            exit_pressure: 60,
            benefactor: 0.0,
            idle_at_derive: 0.0,
        };
        board.vision.financial_stance = FinancialStance::Conservative;
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.relationship.trust_results = 10;
        board
    }

    /// A wealthy board with a clear infrastructure mandate and money to burn.
    fn capex_board() -> (ClubBoard, BoardContext) {
        let mut board = ClubBoard::new();
        board.ownership = OwnershipModel {
            ownership_type: OwnershipType::StateBacked,
            base_wealth: 90,
            interference: 60,
            risk_tolerance: 80,
            exit_pressure: 5,
            benefactor: 0.0,
            idle_at_derive: 0.0,
        };
        board.vision.infrastructure_priority = InfrastructurePriority::Training;
        let mut ctx = BoardContext::new();
        ctx.balance = 200_000_000;
        ctx.profit_loss_12m = 40_000_000;
        ctx.ffp_status = FfpStatus::Clean;
        (board, ctx)
    }

    /// A top-flight club of middling standing, for the takeover tests —
    /// the new owner's brief is derived from its tier and reputation.
    fn takeover_ctx() -> BoardContext {
        let mut ctx = BoardContext::new();
        ctx.league_tier = 1;
        ctx.league_size = 20;
        ctx.reputation_score = 0.6;
        ctx
    }

    fn season_start() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2025, 7, 1).unwrap()
    }

    /// Assert every bounded board gauge is in range. Called every simulated
    /// month in the long-progression test.
    fn assert_board_invariants(board: &ClubBoard, result: &BoardResult) {
        assert!(
            (0..=100).contains(&board.confidence.level),
            "confidence out of range: {}",
            board.confidence.level
        );
        for g in [
            board.pressure.supporter_pressure,
            board.pressure.media_pressure,
            board.pressure.dressing_room_pressure,
            board.pressure.financial_pressure,
            board.pressure.regulatory_pressure,
        ] {
            assert!(g <= 100, "pressure gauge out of range: {g}");
        }
        for f in [
            board.relationship.trust_results,
            board.relationship.trust_finances,
            board.relationship.trust_squad_building,
            board.relationship.trust_communication,
            board.relationship.style_alignment,
        ] {
            assert!(f <= 100, "relationship facet out of range: {f}");
        }
        // Budget decisions only ever carry non-negative magnitudes, so
        // `process` can never drive the club's budget negative from them.
        for d in &result.decisions {
            match d {
                BoardDecision::CutTransferBudget { amount, .. }
                | BoardDecision::IncreaseTransferBudget { amount, .. } => {
                    assert!(*amount >= 0, "budget decision amount went negative: {d:?}");
                }
                _ => {}
            }
        }
    }
}

#[test]
fn early_season_bad_form_does_not_sack_manager() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(5, 8));
    let ctx = Scenario::poor_ctx(8, 38, 19, 20); // Early phase
    let mut sacked = false;
    for _ in 0..8 {
        let mut r = BoardResult::new();
        board.evaluate_performance(&ctx, &mut r);
        sacked |= r.manager_sacked;
    }
    assert!(!sacked, "early-season form must not cost a job");
}

#[test]
fn run_in_underperformance_can_trigger_sacking() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(5, 8));
    let ctx = Scenario::poor_ctx(32, 38, 19, 20); // RunIn phase
    let mut sacked = false;
    for _ in 0..12 {
        let mut r = BoardResult::new();
        board.evaluate_performance(&ctx, &mut r);
        if r.manager_sacked {
            sacked = true;
            break;
        }
    }
    assert!(sacked, "sustained run-in collapse should cost the job");
}

#[test]
fn sack_requires_a_lived_ultimatum_month() {
    // The FIRST evaluation that reaches crisis issues the public
    // ultimatum; the sack may only follow on a LATER evaluation —
    // the squad gets a real month to react.
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(5, 8));
    let ctx = Scenario::poor_ctx(32, 38, 19, 20); // RunIn phase
    let mut announced_at: Option<usize> = None;
    let mut sacked_at: Option<usize> = None;
    for month in 0..12 {
        let mut r = BoardResult::new();
        board.evaluate_performance(&ctx, &mut r);
        if r.manager_ultimatum_announced && announced_at.is_none() {
            announced_at = Some(month);
        }
        if r.manager_sacked {
            sacked_at = Some(month);
            break;
        }
    }
    let announced = announced_at.expect("a collapse must produce a public ultimatum");
    let sacked = sacked_at.expect("a sustained collapse still costs the job");
    assert!(
        sacked > announced,
        "the ultimatum (month {}) must precede the sack (month {})",
        announced,
        sacked
    );
}

#[test]
fn results_on_final_warning_save_the_job() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(5, 8));
    let poor = Scenario::poor_ctx(32, 38, 19, 20);
    let mut announced = false;
    for _ in 0..12 {
        let mut r = BoardResult::new();
        board.evaluate_performance(&poor, &mut r);
        if r.manager_sacked {
            panic!("must not sack before the ultimatum has been lived with");
        }
        if r.manager_ultimatum_announced {
            announced = true;
            break;
        }
    }
    assert!(announced, "the collapse must reach the ultimatum stage");

    // Form turns — the warning lapses instead of becoming a sack.
    let strong = Scenario::strong_ctx(3, 20);
    let mut sacked = false;
    for _ in 0..4 {
        let mut r = BoardResult::new();
        board.evaluate_performance(&strong, &mut r);
        sacked |= r.manager_sacked;
    }
    assert!(
        !sacked,
        "recovered form on the final warning must save the job"
    );
    assert!(
        !board.manager_on_final_warning,
        "the warning lapses once results recover"
    );
}

#[test]
fn ffp_breach_cuts_budget_and_raises_financial_pressure() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(8, 12));
    let mut ctx = Scenario::strong_ctx(8, 20);
    ctx.ffp_status = FfpStatus::Breach;
    ctx.wage_budget_usage = 1.2;
    ctx.debt_ratio = 1.2;
    ctx.profit_loss_12m = -10_000_000;

    let mut r = BoardResult::new();
    board.evaluate_performance(&ctx, &mut r);

    assert!(
        r.decisions.iter().any(|d| matches!(
            d,
            BoardDecision::CutTransferBudget {
                reason: DecisionReason::FfpPressure,
                ..
            }
        )),
        "FFP breach must emit a budget cut: {:?}",
        r.decisions
    );
    assert!(board.pressure.regulatory_pressure > 0);
    assert!(board.pressure.financial_pressure > 0);
}

#[test]
fn reckless_owner_increases_budget_but_lowers_patience() {
    // Elite club, seed 0 -> StateBacked (reckless) ownership.
    let mut ctx = BoardContext::new();
    ctx.reputation_score = 0.9;
    ctx.balance = 50_000_000;
    ctx.country_economic_factor = 1.2;
    ctx.country_price_level = 1.0;
    ctx.trailing_annual_income = 60_000_000;
    ctx.trailing_annual_outcome = 40_000_000;
    ctx.projected_annual_income = 60_000_000;

    let mut reckless = ClubBoard::new();
    reckless.bootstrap_personality(&ctx, 0);
    assert!(matches!(
        reckless.ownership.ownership_type,
        OwnershipType::StateBacked
    ));
    assert!(matches!(
        reckless.chairman.ambition,
        ChairmanAmbition::Reckless
    ));
    assert!(matches!(reckless.chairman.patience, ChairmanPatience::Low));

    reckless.calculate_season_targets(&ctx);
    let reckless_budget = reckless.season_targets.as_ref().unwrap().transfer_budget;

    let mut neutral = ClubBoard::new();
    neutral.calculate_season_targets(&ctx);
    let neutral_budget = neutral.season_targets.as_ref().unwrap().transfer_budget;

    assert!(
        reckless_budget > neutral_budget,
        "reckless owner should out-spend neutral: {reckless_budget} vs {neutral_budget}"
    );
    assert!(
        reckless.chairman.poor_mood_threshold() < ChairmanProfile::new().poor_mood_threshold(),
        "reckless owner should be quicker to act"
    );
}

#[test]
fn conservative_owner_blocks_wage_heavy_transfer() {
    let mut board = ClubBoard::new();
    board.vision.financial_stance = FinancialStance::Conservative;
    let mut p = Scenario::proposal(
        500_000.0,
        26,
        70,
        TransferNeedPriority::Important,
        TransferNeedReason::QualityUpgrade,
    );
    p.economics = Some(BoardTransferEconomics {
        wage_impact_annual: 5_000_000.0,
        wage_budget_headroom: 0.0,
        contract_length_years: 4,
        ..Default::default()
    });
    assert!(matches!(
        board.review_transfer_proposal(&p),
        BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
    ));
}

#[test]
fn private_equity_board_flags_poor_resale() {
    let mut board = ClubBoard::new();
    board.ownership.ownership_type = OwnershipType::PrivateEquity;
    // An ageing target whose projected resale is far below the fee.
    let mut p = Scenario::proposal(
        800_000.0,
        30,
        70,
        TransferNeedPriority::Important,
        TransferNeedReason::QualityUpgrade,
    );
    p.economics = Some(BoardTransferEconomics {
        resale_projection: p.fee * 0.3, // < 40% of fee
        wage_budget_headroom: 50_000_000.0,
        ..Default::default()
    });
    assert!(
        matches!(
            board.review_transfer_proposal(&p),
            BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision)
        ),
        "PE owner should flag an ageing, poor-resale signing"
    );
}

#[test]
fn state_backed_board_allows_elite_exception_despite_wage_breach() {
    let mut board = ClubBoard::new();
    board.ownership.ownership_type = OwnershipType::StateBacked;
    board.chairman.ambition = ChairmanAmbition::Reckless;
    // Elite signing (well above squad average) on a critical need, even
    // though wages blow past the headroom.
    let mut p = Scenario::proposal(
        1_200_000.0,
        25,
        80,
        TransferNeedPriority::Critical,
        TransferNeedReason::QualityUpgrade,
    );
    p.economics = Some(BoardTransferEconomics {
        wage_impact_annual: 5_000_000.0,
        wage_budget_headroom: 0.0, // breach
        ..Default::default()
    });
    assert!(
        board.review_transfer_proposal(&p).is_approved(),
        "state-backed board should grant the elite exception"
    );
}

#[test]
fn member_owned_board_values_homegrown_fit() {
    let mut board = ClubBoard::new();
    board.ownership.ownership_type = OwnershipType::MemberOwned;

    // A borderline-priced signing the import version can't quite justify.
    let make = |homegrown: bool| {
        let mut p = Scenario::proposal(
            1_900_000.0,
            24,
            70,
            TransferNeedPriority::Optional,
            TransferNeedReason::QualityUpgrade,
        );
        p.economics = Some(BoardTransferEconomics {
            wage_budget_headroom: 50_000_000.0,
            resale_projection: p.fee,
            homegrown_fit: homegrown,
            ..Default::default()
        });
        p
    };

    assert!(
        board.review_transfer_proposal(&make(true)).is_approved(),
        "member-owned board should back the homegrown signing"
    );
    assert!(
        matches!(
            board.review_transfer_proposal(&make(false)),
            BoardTransferDecision::Vetoed(_)
        ),
        "the same deal for an import gets less rope and is vetoed"
    );
}

#[test]
fn youth_board_accepts_weak_young_blocks_old_depth() {
    let mut board = ClubBoard::new();
    board.vision.preferred_squad_profile = SquadProfile::Youth;

    // Weaker-but-young development signing: welcomed.
    let young = Scenario::proposal(
        300_000.0,
        19,
        50,
        TransferNeedPriority::Optional,
        TransferNeedReason::DevelopmentSigning,
    );
    assert!(
        board.review_transfer_proposal(&young).is_approved(),
        "youth board should accept a promising teenager"
    );

    // Ageing depth signing: blocked.
    let old = Scenario::proposal(
        400_000.0,
        31,
        66,
        TransferNeedPriority::Important,
        TransferNeedReason::DepthCover,
    );
    assert!(matches!(
        board.review_transfer_proposal(&old),
        BoardTransferDecision::Vetoed(BoardTransferConcern::ConflictsWithVision)
    ));
}

#[test]
fn manager_renewal_merited_after_sustained_high_trust() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(8, 12));
    let ctx = Scenario::strong_ctx(2, 20); // overachieving
    for _ in 0..14 {
        let mut r = BoardResult::new();
        board.evaluate_performance(&ctx, &mut r);
    }
    assert!(
        board.relationship.merits_renewal(),
        "sustained overperformance should merit a renewal"
    );
    assert!(board.confidence.level >= 70);
}

#[test]
fn poor_and_ffp_breach_apply_exactly_one_budget_cut() {
    // Regression for the double-apply bug: a Poor-mood month that is
    // also an FFP breach must cut the budget once (the FFP cut), never
    // a mood percentage *and* an FFP amount in the same tick.
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(5, 8));
    let mut ctx = Scenario::poor_ctx(8, 38, 19, 20); // Early phase → no sacking
    ctx.ffp_status = FfpStatus::Breach;

    let mut first = BoardResult::new();
    board.evaluate_performance(&ctx, &mut first);

    let cuts = first
        .decisions
        .iter()
        .filter(|d| matches!(d, BoardDecision::CutTransferBudget { .. }))
        .count();
    assert_eq!(
        cuts, 1,
        "exactly one cut in a Poor+breach month, got {:?}",
        first.decisions
    );
    // The single cut is the dominant FFP one — the mood cut is pre-empted.
    assert!(first.decisions.iter().any(|d| matches!(
        d,
        BoardDecision::CutTransferBudget {
            reason: DecisionReason::FfpPressure,
            ..
        }
    )));
    // No increase is emitted while breaching.
    assert!(
        !first
            .decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::IncreaseTransferBudget { .. })),
        "a breaching board must not also boost the budget"
    );
}

/// The board's ceiling on second-guessing its own budget. A grievance that
/// persists for a year used to re-cut the mandate every single month, each
/// cut a fresh "the board slash the budget" story for a mandate the club
/// rebuilt from scratch anyway.
#[test]
fn a_grievance_that_lasts_a_year_does_not_cut_the_budget_twelve_times() {
    let mut board = ClubBoard::new();
    let targets = Scenario::targets(5, 8);
    let mandate = targets.transfer_budget as i64;
    board.season_targets = Some(targets);
    let mut ctx = Scenario::poor_ctx(8, 38, 19, 20);
    ctx.ffp_status = FfpStatus::Breach;

    let mut total_cut = 0i64;
    for _ in 0..12 {
        let mut result = BoardResult::new();
        board.evaluate_performance(&ctx, &mut result);
        for decision in &result.decisions {
            if let BoardDecision::CutTransferBudget { amount, .. } = decision {
                total_cut += amount;
            }
        }
    }

    let cap = (mandate as f64 * ClubBoard::SEASON_ADJUSTMENT_CAP) as i64;
    assert!(
        total_cut <= cap,
        "a season of grievance took {total_cut} off a {mandate} mandate, past the {cap} ceiling"
    );
    assert!(total_cut > 0, "a breaching board should still cut once");
}

/// A sustained poor mood is one decision, not one a month — and the cut it
/// makes survives the monthly rebuild of the live budget.
#[test]
fn a_poor_mood_cuts_the_mandate_once_and_the_cut_sticks() {
    let mut board = ClubBoard::new();
    let targets = Scenario::targets(5, 8);
    let mandate = targets.transfer_budget as i64;
    board.season_targets = Some(targets);
    let ctx = Scenario::poor_ctx(8, 38, 19, 20);

    let mut cuts = 0usize;
    let mut cut_total = 0i64;
    for _ in 0..4 {
        let mut result = BoardResult::new();
        board.evaluate_performance(&ctx, &mut result);
        for decision in &result.decisions {
            if let BoardDecision::CutTransferBudget { amount, .. } = decision {
                cuts += 1;
                cut_total += amount;
                // What `BoardResult::apply_decisions` would do to the mandate.
                if let Some(t) = board.season_targets.as_mut() {
                    t.mandate_adjustment -= amount;
                }
            }
        }
    }

    assert_eq!(cuts, 1, "four poor months, one cut");
    assert_eq!(
        cut_total,
        (mandate as f64 * ClubBoard::POOR_MOOD_CUT_SHARE) as i64
    );
    // …and the mandate the club rebuilds its live budget from is the cut one.
    let targets = board.season_targets.as_ref().unwrap();
    assert_eq!(targets.adjusted_transfer_budget(), mandate - cut_total);
}

#[test]
fn excellent_and_injection_emit_a_single_increase() {
    // A strong run under a wealthy owner injects once — not a fixed
    // injection plus a separate excellent-mood percentage on top.
    let mut ctx = BoardContext::new();
    ctx.reputation_score = 0.9;
    ctx.balance = 50_000_000;
    ctx.country_economic_factor = 1.2;
    ctx.trailing_annual_income = 60_000_000;
    ctx.trailing_annual_outcome = 40_000_000;
    let mut board = ClubBoard::new();
    board.bootstrap_personality(&ctx, 0); // StateBacked → high injection appetite
    board.season_targets = Some(Scenario::targets(2, 6));

    let strong = Scenario::strong_ctx(2, 20);
    let mut increases = 0usize;
    let mut owner_money = false;
    for _ in 0..4 {
        let mut result = BoardResult::new();
        board.evaluate_performance(&strong, &mut result);
        for decision in &result.decisions {
            if let BoardDecision::IncreaseTransferBudget { reason, .. } = decision {
                increases += 1;
                owner_money |= matches!(reason, DecisionReason::OwnerInjection);
            }
        }
    }

    // Four strong months, one cheque: the gap rule holds the owner to one
    // injection until three months have passed.
    assert_eq!(increases, 1, "a good run should not be milked monthly");
    assert!(owner_money, "the increase should be the owner's injection");
}

#[test]
fn takeover_roll_is_deterministic_and_in_range() {
    let date = chrono::NaiveDate::from_ymd_opt(2025, 8, 1).unwrap();
    let a = DeterministicRoll::percent(42, date, 0);
    assert_eq!(
        a,
        DeterministicRoll::percent(42, date, 0),
        "same inputs → same roll"
    );
    assert!(a < 100, "roll must be a 0..99 percentage");
    // The salt axes actually move the output (so a rumour-start roll and
    // a resolution roll on the same day diverge).
    assert!(
        DeterministicRoll::percent(43, date, 0) != a
            || DeterministicRoll::percent(42, date, 1) != a
            || DeterministicRoll::percent(
                42,
                chrono::NaiveDate::from_ymd_opt(2025, 9, 1).unwrap(),
                0
            ) != a,
        "roll should vary across club / date / status"
    );
}

#[test]
fn takeover_decision_stream_replays_identically() {
    // Two runs with identical club id, dates and starting state must
    // produce the byte-for-byte same decision stream — no global RNG.
    fn run(club_id: u32) -> Vec<&'static str> {
        let mut board = ClubBoard::new();
        board.ownership = OwnershipModel {
            ownership_type: OwnershipType::PrivateEquity,
            base_wealth: 45,
            interference: 55,
            risk_tolerance: 65,
            exit_pressure: 80,
            benefactor: 0.0,
            idle_at_derive: 0.0,
        };
        let mut ctx = BoardContext::new();
        ctx.balance = -80_000_000;
        ctx.profit_loss_12m = -20_000_000;
        ctx.ffp_status = FfpStatus::Breach;
        ctx.reputation_score = 0.5;
        board.season_targets = Some(Scenario::targets(8, 12));

        let mut labels = Vec::new();
        let mut date = chrono::NaiveDate::from_ymd_opt(2025, 7, 1).unwrap();
        for _ in 0..24 {
            let mut r = BoardResult::new();
            r.club_id = club_id;
            board.tick_takeover(&ctx, date, &mut r);
            labels.extend(r.decisions.iter().map(|d| d.label()));
            date += chrono::Duration::days(30);
        }
        labels
    }
    assert_eq!(run(7), run(7), "identical state must replay identically");
    assert_eq!(run(99), run(99));
}

#[test]
fn takeover_always_installs_a_wealthier_owner_and_resets_relationship() {
    // Whatever the archetype, the buyer arrives richer and the manager
    // relationship resets to the fresh-appointment baseline.
    for seed in 0..3u32 {
        let mut board = Scenario::member_owned_board();
        board.apply_takeover_completion(&Scenario::takeover_ctx(), Scenario::season_start(), seed);
        assert!(
            board.ownership.wealth() >= 70,
            "new owner should be wealthy"
        );
        assert_eq!(board.confidence.level, 60);
        // Relationship was reset (trust_results back above the crisis level).
        assert!(board.relationship.trust_results >= 50);
    }
}

#[test]
fn state_backed_takeover_chases_trophies_with_stars() {
    let mut board = Scenario::member_owned_board();
    board.apply_takeover_completion(&Scenario::takeover_ctx(), Scenario::season_start(), 0); // seed % 3 == 0 → StateBacked
    assert!(matches!(
        board.ownership.ownership_type,
        OwnershipType::StateBacked
    ));
    assert!(matches!(
        board.vision.preferred_squad_profile,
        SquadProfile::Stars
    ));
    assert!(matches!(
        board.vision.financial_stance,
        FinancialStance::Ambitious
    ));
    assert_eq!(board.vision.long_term_goal, Some(LongTermGoal::WinLeague));
    assert!(matches!(
        board.chairman.ambition,
        ChairmanAmbition::Reckless
    ));
}

#[test]
fn private_equity_takeover_prioritises_resale_and_wage_discipline() {
    let mut board = Scenario::member_owned_board();
    board.apply_takeover_completion(&Scenario::takeover_ctx(), Scenario::season_start(), 1); // seed % 3 == 1 → PrivateEquity
    assert!(matches!(
        board.ownership.ownership_type,
        OwnershipType::PrivateEquity
    ));
    assert!(matches!(
        board.vision.preferred_squad_profile,
        SquadProfile::ResaleValue
    ));
    assert!(
        matches!(board.vision.financial_stance, FinancialStance::Conservative),
        "PE owners run a tight wage ship, not a Galáctico policy"
    );
    assert_eq!(
        board.vision.long_term_goal,
        Some(LongTermGoal::EstablishTopHalf)
    );
    assert!(matches!(
        board.vision.infrastructure_priority,
        InfrastructurePriority::Commercial
    ));
}

#[test]
fn consortium_takeover_builds_a_balanced_prime_age_side() {
    let mut board = Scenario::member_owned_board();
    board.apply_takeover_completion(&Scenario::takeover_ctx(), Scenario::season_start(), 2); // seed % 3 == 2 → Consortium
    assert!(matches!(
        board.ownership.ownership_type,
        OwnershipType::Consortium
    ));
    assert!(matches!(
        board.vision.preferred_squad_profile,
        SquadProfile::PrimeAge
    ));
    assert!(matches!(
        board.vision.financial_stance,
        FinancialStance::Balanced
    ));
    assert_eq!(
        board.vision.long_term_goal,
        Some(LongTermGoal::WinContinental)
    );
}

#[test]
fn facility_cooldown_blocks_consecutive_season_upgrades() {
    let (mut board, ctx) = Scenario::capex_board();

    // Season 1: the upgrade is approved and starts the cooldown.
    let y1 = board.run_facility_review(&ctx, 2025);
    assert!(
        y1.iter()
            .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. })),
        "wealthy mandated board should approve in season 1: {y1:?}"
    );
    assert_eq!(board.last_facility_upgrade_year, Some(2025));

    // Season 2: still inside the cooldown → no upgrade at all.
    let y2 = board.run_facility_review(&ctx, 2026);
    assert!(
        y2.is_empty(),
        "cooldown must suppress the very next season: {y2:?}"
    );

    // Season 3: cooldown elapsed → upgrades allowed again.
    let y3 = board.run_facility_review(&ctx, 2027);
    assert!(
        y3.iter()
            .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. })),
        "after the cooldown the board may upgrade again: {y3:?}"
    );
}

#[test]
fn ffp_breach_blocks_capex_and_keeps_cooldown_unused() {
    let (mut board, mut ctx) = Scenario::capex_board();
    ctx.ffp_status = FfpStatus::Breach;

    let d = board.run_facility_review(&ctx, 2025);
    assert!(d.iter().any(|x| matches!(
        x,
        BoardDecision::RejectFacilityUpgrade {
            reason: DecisionReason::FfpPressure,
            ..
        }
    )));
    assert!(
        !d.iter()
            .any(|x| matches!(x, BoardDecision::ApproveFacilityUpgrade { .. })),
        "a breaching club can't fund capex"
    );
    // A rejection doesn't consume the cooldown — once compliant the club
    // is free to upgrade without waiting out a phantom cooldown.
    assert_eq!(board.last_facility_upgrade_year, None);
}

#[test]
fn survival_promise_is_created_then_kept_and_builds_trust() {
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::Survive);
    board.season_targets = Some(Scenario::targets(17, 20));
    let today = Scenario::season_start();

    board.open_season_promises(&Scenario::strong_ctx(8, 20), today, &[]);
    assert!(
        board.promises.has_active(PromiseType::Survival),
        "a survival-minded board should open a survival promise"
    );

    // Comfortably safe in the run-in → promise kept, trust rises.
    let before = board.relationship.trust_communication;
    let mut ctx = Scenario::strong_ctx(8, 20);
    ctx.matches_played = 32;
    ctx.total_matches = 38; // RunIn
    ctx.distance_to_relegation = 6;
    let mut r = BoardResult::new();
    board.resolve_promises(&ctx, today + chrono::Duration::days(250), &mut r);

    assert!(
        !board.promises.has_active(PromiseType::Survival),
        "staying clear of the drop should keep the survival promise"
    );
    assert!(board.relationship.trust_communication >= before);
}

#[test]
fn unkept_promise_breaks_and_costs_trust_at_season_end() {
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::Survive);
    board.season_targets = Some(Scenario::targets(17, 20));
    let today = Scenario::season_start();
    board.open_season_promises(&Scenario::strong_ctx(8, 20), today, &[]);

    let before = board.relationship.trust_communication;
    // Mirror simulate's season-start reckoning a year later.
    let penalty = board
        .promises
        .break_overdue(today + chrono::Duration::days(366));
    assert!(penalty < 0, "an unmet survival promise must break");
    board.relationship.adjust_communication(penalty);
    assert!(board.relationship.trust_communication < before);
}

#[test]
fn season_promises_do_not_duplicate_within_window() {
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::Survive);
    board.vision.youth_focus = VisionYouthFocus::DevelopYouth;
    board.season_targets = Some(Scenario::targets(17, 20));
    let today = Scenario::season_start();

    board.open_season_promises(&Scenario::strong_ctx(8, 20), today, &[]);
    board.open_season_promises(&Scenario::strong_ctx(8, 20), today, &[]);

    let survival = board
        .promises
        .active()
        .filter(|p| p.promise_type == PromiseType::Survival)
        .count();
    let youth = board
        .promises
        .active()
        .filter(|p| p.promise_type == PromiseType::YouthMinutes)
        .count();
    assert_eq!(survival, 1, "survival promise must not duplicate");
    assert_eq!(youth, 1, "youth promise must not duplicate");
}

#[test]
fn deferred_capex_opens_a_facility_promise() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(10, 14));
    let today = Scenario::season_start();
    let rejected = [BoardDecision::RejectFacilityUpgrade {
        facility: BoardFacility::Training,
        reason: DecisionReason::DebtTooHigh,
    }];
    board.open_season_promises(&Scenario::strong_ctx(8, 20), today, &rejected);
    assert!(board.promises.has_active(PromiseType::FacilityImprovement));

    // A later approved upgrade keeps it.
    let mut r = BoardResult::new();
    r.decisions.push(BoardDecision::ApproveFacilityUpgrade {
        facility: BoardFacility::Training,
        cost: 5_000_000,
    });
    board.resolve_promises(
        &Scenario::strong_ctx(8, 20),
        today + chrono::Duration::days(370),
        &mut r,
    );
    assert!(!board.promises.has_active(PromiseType::FacilityImprovement));
}

#[test]
fn bootstrap_personality_is_deterministic_for_same_club() {
    // No global RNG in derivation: the same durable club signals must
    // always yield the same ownership archetype, so a re-derive (e.g.
    // after a hot-reload) never re-randomises the board.
    let mut ctx = BoardContext::new();
    ctx.reputation_score = 0.62;
    ctx.balance = 8_000_000;
    ctx.country_economic_factor = 1.1;

    let mut a = ClubBoard::new();
    a.bootstrap_personality(&ctx, 1234);
    let mut b = ClubBoard::new();
    b.bootstrap_personality(&ctx, 1234);

    assert_eq!(a.ownership.ownership_type, b.ownership.ownership_type);
    assert_eq!(a.ownership.wealth(), b.ownership.wealth());
    assert_eq!(a.ownership.risk_tolerance, b.ownership.risk_tolerance);
    assert!(a.personality_initialized);

    // …and the same brief, down to the football it asks for.
    assert_eq!(a.vision.playing_style, b.vision.playing_style);
    assert_eq!(a.vision.youth_focus, b.vision.youth_focus);
    assert_eq!(a.vision.signing_preference, b.vision.signing_preference);
    assert_eq!(a.vision.long_term_goal, b.vision.long_term_goal);
    assert_eq!(
        a.vision.long_term_horizon_seasons,
        b.vision.long_term_horizon_seasons
    );
}

/// Every board leaves its first tick with a brief it can judge somebody
/// against. Before this, five of the vision's axes were never written: the
/// style drag was permanently zero for every club in the world, the
/// long-term reckoning was unreachable, and the ambition multiplier on the
/// transfer budget was the same 0.85 everywhere.
#[test]
fn a_bootstrapped_board_has_a_brief_and_not_a_blank_one() {
    let mut ctx = BoardContext::new();
    ctx.reputation_score = 0.9;
    ctx.league_tier = 1;
    ctx.balance = 400_000_000;
    ctx.country_economic_factor = 1.4;
    ctx.main_tactic = Some(MatchTacticType::T433);

    let mut board = ClubBoard::new();
    board.bootstrap_personality(&ctx, 7);

    assert_eq!(
        board.vision.playing_style,
        VisionPlayingStyle::Possession,
        "an elite side already playing 4-3-3 is briefed to keep the ball"
    );
    assert!(board.vision.long_term_goal.is_some(), "no goal was set");
    assert_eq!(
        board.vision.long_term_goal,
        Some(LongTermGoal::WinLeague),
        "a 0.9-reputation top-flight club is asked to win the thing"
    );
    assert!(
        board.vision.long_term_horizon_seasons >= ClubVision::HORIZON_MIN_SEASONS,
        "the horizon is still zero, so the reckoning can never fire"
    );
    assert!(board.vision.long_term_horizon_seasons <= ClubVision::HORIZON_MAX_SEASONS);
    // The ambition multiplier now varies by club rather than sitting at the
    // goal-less 0.85 for everybody.
    assert!(board.vision.budget_multiplier() > 0.85);
}

/// A second-division side of real standing is told to go up; a small one in
/// the same division is told to stay there.
#[test]
fn the_division_and_the_standing_decide_what_is_asked_for() {
    let mut promotion_ctx = BoardContext::new();
    promotion_ctx.league_tier = 2;
    promotion_ctx.reputation_score = 0.62;
    let mut chaser = ClubBoard::new();
    chaser.bootstrap_personality(&promotion_ctx, 3);
    assert_eq!(
        chaser.vision.long_term_goal,
        Some(LongTermGoal::PromotionToTopFlight)
    );

    let mut struggler_ctx = BoardContext::new();
    struggler_ctx.league_tier = 2;
    struggler_ctx.reputation_score = 0.2;
    let mut struggler = ClubBoard::new();
    struggler.bootstrap_personality(&struggler_ctx, 3);
    assert_eq!(struggler.vision.long_term_goal, Some(LongTermGoal::Survive));
    assert!(
        struggler.vision.budget_multiplier() < chaser.vision.budget_multiplier(),
        "a survival brief should not fund a promotion war chest"
    );
}

/// A patient chairman buys the project a season; an impatient one takes one
/// off. Bounded at both ends so no board waits a decade.
#[test]
fn chairman_patience_stretches_the_horizon_within_bounds() {
    assert_eq!(ClubVision::horizon_for(3, ChairmanPatience::High), 4);
    assert_eq!(ClubVision::horizon_for(3, ChairmanPatience::Medium), 3);
    assert_eq!(ClubVision::horizon_for(3, ChairmanPatience::Low), 2);
    // Bounds hold at both ends.
    assert_eq!(
        ClubVision::horizon_for(2, ChairmanPatience::Low),
        ClubVision::HORIZON_MIN_SEASONS
    );
    assert_eq!(
        ClubVision::horizon_for(5, ChairmanPatience::High),
        ClubVision::HORIZON_MAX_SEASONS
    );
}

/// A missed horizon is a grievance, not an execution. The old path sacked
/// outright, bypassing the warning-then-ultimatum ladder entirely.
#[test]
fn a_missed_horizon_costs_confidence_before_it_costs_the_job() {
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::WinLeague);
    board.vision.long_term_horizon_seasons = 2;
    board.vision_start_year = Some(2030);
    let before = board.confidence.level;

    let mut result = BoardResult::new();
    board.evaluate_long_term_vision(2032, &mut result);

    assert!(
        !result.manager_sacked,
        "a 65-confidence board should not sack"
    );
    assert_eq!(
        board.confidence.level,
        before - ClubBoard::VISION_MISS_CONFIDENCE_PENALTY
    );
    assert!(
        result
            .decisions
            .contains(&BoardDecision::IssueFormalWarning),
        "the miss must be put to him formally"
    );
    // The clock restarts on the same goal.
    assert_eq!(board.vision_start_year, Some(2032));
    assert_eq!(board.vision.long_term_goal, Some(LongTermGoal::WinLeague));
}

/// …but a board already close to the edge does dismiss on it, and now says
/// so in a decision the news desk can read.
#[test]
fn a_missed_horizon_at_low_confidence_ends_the_job() {
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::Survive);
    board.vision.long_term_horizon_seasons = 2;
    board.vision_start_year = Some(2030);
    board.confidence.level = ClubBoard::VISION_MISS_CONFIDENCE_PENALTY + 30;

    let mut result = BoardResult::new();
    board.evaluate_long_term_vision(2033, &mut result);

    assert!(result.manager_sacked);
    assert!(result.decisions.contains(&BoardDecision::SackManager));
    assert!(!board.manager_on_final_warning);
}

/// A board that got what it asked for asks for more.
#[test]
fn a_delivered_horizon_rolls_the_goal_forward() {
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::EstablishTopHalf);
    board.vision.long_term_horizon_seasons = 2;
    board.vision_start_year = Some(2030);
    board.vision_goal_achieved = true;
    let loyalty_before = board.chairman.manager_loyalty;

    let mut result = BoardResult::new();
    board.evaluate_long_term_vision(2032, &mut result);

    assert!(!result.manager_sacked);
    assert_eq!(
        board.vision.long_term_goal,
        Some(LongTermGoal::WinDomesticCup),
        "the next rung up, not the same ask again"
    );
    assert!(board.chairman.manager_loyalty > loyalty_before);
    assert!(
        !board.vision_goal_achieved,
        "the flag resets for the new horizon"
    );
}

/// The style brief now bites: a board that wants attacking football loses
/// patience with a manager who parks the bus, and keeps it with one who
/// does not.
#[test]
fn a_style_clash_erodes_alignment_where_a_fit_does_not() {
    let mut ctx = Scenario::strong_ctx(8, 20);
    ctx.main_tactic = Some(MatchTacticType::T451);

    let mut clashing = ClubBoard::new();
    clashing.season_targets = Some(Scenario::targets(8, 13));
    clashing.vision.playing_style = VisionPlayingStyle::AttackingFootball;

    let mut fitting = ClubBoard::new();
    fitting.season_targets = Some(Scenario::targets(8, 13));
    fitting.vision.playing_style = VisionPlayingStyle::AttackingFootball;
    let mut fit_ctx = ctx.clone();
    fit_ctx.main_tactic = Some(MatchTacticType::T343);

    for _ in 0..6 {
        let mut result = BoardResult::new();
        clashing.evaluate_performance(&ctx, &mut result);
        let mut result = BoardResult::new();
        fitting.evaluate_performance(&fit_ctx, &mut result);
    }

    assert!(
        clashing.relationship.style_alignment < fitting.relationship.style_alignment,
        "a 4-5-1 under an attacking brief should cost alignment: {} vs {}",
        clashing.relationship.style_alignment,
        fitting.relationship.style_alignment
    );
}

#[test]
fn board_holds_all_invariants_over_three_sustained_poor_seasons() {
    // A relegation-bound mid-table club judged harshly for three
    // seasons. Confidence/pressure/relationship must stay in band, the
    // promise ledger must stay bounded, and the board must not sack the
    // manager in consecutive months (it resets and gives the caretaker
    // a run after each dismissal).
    let mut board = ClubBoard::new();
    board.vision.long_term_goal = Some(LongTermGoal::EstablishTopHalf);

    let total = 38u8;
    let mut today = Scenario::season_start();
    let mut sack_months: Vec<u32> = Vec::new();
    let mut month_counter = 0u32;
    let mut max_active_promises = 0usize;

    for _season in 0..3 {
        // Season-start reckoning (mirrors `simulate`).
        let mut start_ctx = Scenario::poor_ctx(0, total, 17, 20);
        start_ctx.league_size = 20;
        board.calculate_season_targets(&start_ctx);
        let penalty = board.promises.break_overdue(today);
        if penalty != 0 {
            board.relationship.adjust_communication(penalty);
        }
        board.promises.prune(today, 800);
        board.open_season_promises(&start_ctx, today, &[]);
        board.confidence.level = 65;
        board.poor_mood_months = 0;
        board.season_month_index = 0;

        for m in 0..11u8 {
            month_counter += 1;
            let matches_played = ((m as u16 * total as u16) / 10).min(total as u16) as u8;
            let mut ctx = Scenario::poor_ctx(matches_played, total, 17, 20);
            ctx.league_size = 20;

            let mut r = BoardResult::new();
            board.evaluate_performance(&ctx, &mut r);
            board.resolve_promises(&ctx, today, &mut r);

            if r.manager_sacked {
                sack_months.push(month_counter);
            }

            Scenario::assert_board_invariants(&board, &r);
            max_active_promises = max_active_promises.max(board.promises.active_count());

            board.season_month_index += 1;
            today += chrono::Duration::days(30);
        }
        today += chrono::Duration::days(65); // skip to next season start
    }

    // Never sacked in back-to-back months: after a dismissal the board
    // resets and the caretaker gets at least a couple of months.
    for w in sack_months.windows(2) {
        assert!(
            w[1] - w[0] >= 2,
            "manager sacked in consecutive months: {sack_months:?}"
        );
    }
    // Ledger stays small across seasons (break + prune + dedupe).
    assert!(
        max_active_promises <= 6,
        "promise ledger grew unbounded: {max_active_promises}"
    );
}

#[test]
fn takeover_rumour_always_resolves_within_its_window() {
    // A distressed, exit-pressured club: a rumour will open at some
    // point and must never sit Rumoured indefinitely — it resolves to
    // Completed or Failed within the simmer window.
    let mut board = ClubBoard::new();
    board.ownership = OwnershipModel {
        ownership_type: OwnershipType::PrivateEquity,
        base_wealth: 45,
        interference: 55,
        risk_tolerance: 65,
        exit_pressure: 80,
        benefactor: 0.0,
        idle_at_derive: 0.0,
    };
    board.season_targets = Some(Scenario::targets(10, 18));
    let mut ctx = BoardContext::new();
    ctx.balance = -80_000_000;
    ctx.profit_loss_12m = -20_000_000;
    ctx.ffp_status = FfpStatus::Breach;
    ctx.reputation_score = 0.5;

    let mut date = Scenario::season_start();
    let mut rumoured_streak = 0u8;
    let mut ever_resolved = false;
    for _ in 0..36 {
        let mut r = BoardResult::new();
        r.club_id = 123;
        board.tick_takeover(&ctx, date, &mut r);
        match board.takeover.status {
            TakeoverStatus::Rumoured => {
                rumoured_streak += 1;
                assert!(
                    rumoured_streak <= 3,
                    "rumour stuck unresolved for {rumoured_streak} months"
                );
            }
            TakeoverStatus::Completed | TakeoverStatus::Failed => {
                rumoured_streak = 0;
                ever_resolved = true;
            }
            TakeoverStatus::None => rumoured_streak = 0,
        }
        date += chrono::Duration::days(30);
    }
    assert!(
        ever_resolved,
        "an eligible distressed club should see a takeover resolve over 3 years"
    );
}

struct Fx;

impl Fx {
    /// A cash-rich club whose wage bill its revenue cannot carry —
    /// the shape [`ClubBenefactor::signal`] is written for. No
    /// finance history, as a freshly created world has none.
    fn benefactor_ctx() -> BoardContext {
        let mut ctx = BoardContext::new();
        ctx.balance = 300_000_000;
        ctx.total_annual_wages = 160_000_000;
        ctx.reputation_score = 0.75;
        ctx.country_economic_factor = 1.0;
        ctx.country_price_level = 1.0;
        ctx.league_size = 18;
        // Nothing has closed yet: the trailing sums are empty and the
        // projection is all the club can be judged on.
        ctx.trailing_annual_income = 0;
        ctx.trailing_annual_outcome = 0;
        ctx.projected_annual_income = 80_000_000;
        ctx
    }
}

/// A2 — the budget exists from the first tick, not from the first
/// season start. Between world creation and that date every club in
/// the world carried no mandate and no envelope.
#[test]
fn a_board_has_targets_after_its_first_tick_on_any_date() {
    let mut board = ClubBoard::new();
    assert!(board.season_targets.is_none());
    let ctx = Fx::benefactor_ctx();
    board.bootstrap_personality(&ctx, 7);
    // The `season_targets.is_none()` arm of `simulate`, on a date
    // that is not a season start.
    if board.season_targets.is_none() {
        board.calculate_season_targets(&ctx);
    }
    let targets = board.season_targets.expect("a first tick sizes a budget");
    assert!(targets.wage_budget > 0, "{}", targets.wage_budget);
}

/// A2 — …and that first computation carries the owner's cheque,
/// because it reads the PROJECTION. Reading the trailing zero took
/// the `projected_income < 1.0` arm, which sets the subsidy to zero
/// and holds the mandate at the existing bill.
#[test]
fn a_cold_start_benefactor_is_granted_an_envelope_on_its_first_tick() {
    let ctx = Fx::benefactor_ctx();
    let mut board = ClubBoard::new();
    board.bootstrap_personality(&ctx, 7);
    assert!(
        board.ownership.benefactor >= ClubBenefactor::STATE_BACKED_BAR,
        "the fixture must read as owner-funded: {}",
        board.ownership.benefactor
    );
    board.calculate_season_targets(&ctx);
    let targets = board.season_targets.expect("targets");
    assert!(
        targets.owner_subsidy > 0,
        "a benefactor's first season must carry his cheque: {}",
        targets.owner_subsidy
    );
    assert!(
        targets.owner_envelopes.total_granted() > 0.0,
        "…and it must be split into tier envelopes"
    );
}

/// A3 — the top-up is in the pot before the envelope is sized against
/// it. An owner who refills a drained club on 1 July and whose
/// envelope is sized off the pre-refill balance bought nothing with
/// the money.
#[test]
fn a_drained_benefactor_sizes_its_envelope_off_the_refilled_pile() {
    let mut ctx = Fx::benefactor_ctx();
    let mut board = ClubBoard::new();
    board.bootstrap_personality(&ctx, 7);

    // He has spent it: wages still above revenue, cash cover gone.
    ctx.balance = 1_000_000;
    board.calculate_season_targets(&ctx);
    let drained = board
        .season_targets
        .as_ref()
        .expect("targets")
        .owner_envelopes
        .total_granted();

    // Same tick, with the owner's yearly cheque folded into the idle
    // cash the split reads — the order `simulate` now uses.
    let mut funded = ctx.clone();
    funded.balance = ctx.balance + 250_000_000;
    board.calculate_season_targets(&funded);
    let refilled = board
        .season_targets
        .as_ref()
        .expect("targets")
        .owner_envelopes
        .total_granted();

    assert!(
        refilled > drained,
        "the cheque that landed this tick must be spendable this tick: {refilled} vs {drained}"
    );
}

/// A1 — the signal finds the club whose WAGES its revenue cannot
/// carry, not the one sitting on a pile. A second-division side with
/// database cash and a second-division wage bill is not a benefactor.
#[test]
fn a_cash_rich_club_living_inside_its_revenue_is_not_owner_funded() {
    let mut ctx = BoardContext::new();
    ctx.balance = 40_000_000;
    ctx.total_annual_wages = 3_000_000;
    ctx.reputation_score = 0.3;
    ctx.country_economic_factor = 1.0;
    ctx.country_price_level = 1.0;
    ctx.projected_annual_income = 8_000_000;

    let mut board = ClubBoard::new();
    board.bootstrap_personality(&ctx, 3);
    assert!(
        board.ownership.benefactor < ClubBenefactor::STATE_BACKED_BAR,
        "{}",
        board.ownership.benefactor
    );
}

/// The board can finally see what the manager has spent.
///
/// `transfer_budget_usage` was hard-coded to zero, so the financial
/// component score never registered a fee in either direction — a club that
/// had blown its whole chest scored the same as one that had not moved.
#[test]
fn the_board_reads_what_has_actually_been_spent() {
    let mut fees = SeasonTransferFees::default();
    assert_eq!(fees.usage_against(40_000_000), 0.0, "nothing spent yet");

    fees.paid = 20_000_000.0;
    assert!((fees.usage_against(40_000_000) - 0.5).abs() < 1e-6);

    fees.paid = 44_000_000.0;
    assert!(
        fees.usage_against(40_000_000) > 1.05,
        "an overspend reads as one"
    );

    // No mandate is "we don't know", not "spent nothing" — the financial
    // score treats a zero usage as neutral.
    assert_eq!(fees.usage_against(0), 0.0);

    fees.reset();
    assert_eq!(fees.paid, 0.0);
    assert_eq!(fees.received, 0.0);
}

/// A wage bill past its mandate for two reviews running gets trimmed. One
/// month is a signing landing mid-window; two is a habit.
#[test]
fn a_persistent_wage_overrun_trims_the_mandate_once() {
    let mut board = ClubBoard::new();
    let targets = Scenario::targets(8, 13);
    let mandate = targets.wage_budget as i64;
    board.season_targets = Some(targets);

    let mut ctx = Scenario::strong_ctx(8, 20);
    ctx.wage_budget_usage = 1.2;
    ctx.total_annual_wages = 40_000_000;

    let mut first = BoardResult::new();
    board.evaluate_performance(&ctx, &mut first);
    assert!(
        !first
            .decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::AdjustWageBudget { .. })),
        "one month over is not yet a habit: {:?}",
        first.decisions
    );

    let mut second = BoardResult::new();
    board.evaluate_performance(&ctx, &mut second);
    let trims: Vec<i64> = second
        .decisions
        .iter()
        .filter_map(|d| match d {
            BoardDecision::AdjustWageBudget { amount, .. } => Some(*amount),
            _ => None,
        })
        .collect();
    assert_eq!(trims.len(), 1, "got {:?}", second.decisions);
    assert_eq!(
        trims[0],
        -((mandate as f64 * ClubBoard::WAGE_CUT_OVERRUN) as i64)
    );
}

/// …and the board cannot mandate a bill the signed contracts already
/// exceed. Squads unwind through expiries and sales, not overnight.
#[test]
fn the_wage_mandate_never_falls_far_below_the_bill_already_signed() {
    let mut board = ClubBoard::new();
    let mut targets = Scenario::targets(8, 13);
    targets.wage_budget = 50_000_000;
    board.season_targets = Some(targets);

    let mut ctx = Scenario::strong_ctx(8, 20);
    ctx.ffp_status = FfpStatus::Breach;
    // The squad is already on almost exactly the mandate.
    ctx.total_annual_wages = 50_000_000;
    ctx.wage_budget_usage = 1.0;

    let mut result = BoardResult::new();
    board.evaluate_performance(&ctx, &mut result);
    let trim: i64 = result
        .decisions
        .iter()
        .filter_map(|d| match d {
            BoardDecision::AdjustWageBudget { amount, .. } => Some(*amount),
            _ => None,
        })
        .sum();
    let floor = (50_000_000.0 * ClubBoard::WAGE_FLOOR_OF_BILL) as i64;
    assert!(
        50_000_000 + trim >= floor,
        "the board mandated {} against a {} bill, below the {} floor",
        50_000_000 + trim,
        50_000_000,
        floor
    );
}

/// A dismissal bill the bank cannot comfortably carry buys the manager a
/// month. Nothing used to price it at all: the contract vanished with the
/// man and no money moved anywhere.
#[test]
fn an_unaffordable_payoff_makes_the_board_wait() {
    let board = ClubBoard::new();
    let mut ctx = BoardContext::new();
    ctx.manager_annual_salary = 4_000_000;
    ctx.manager_contract_months_left = 36;

    // Petty cash for a club of this size: no reason to hesitate.
    ctx.balance = 400_000_000;
    assert_eq!(board.severance_patience_bonus(&ctx), 0);

    // A twelve-million bill against two million in the bank is a reason to
    // wait that has nothing to do with the football.
    ctx.balance = 2_000_000;
    assert_eq!(
        board.severance_patience_bonus(&ctx),
        ClubBoard::SEVERANCE_PATIENCE_BONUS
    );

    // So is being overdrawn, whatever the deal is worth.
    ctx.balance = -1;
    ctx.manager_contract_months_left = 1;
    assert_eq!(
        board.severance_patience_bonus(&ctx),
        ClubBoard::SEVERANCE_PATIENCE_BONUS
    );

    // A vacant seat costs nothing to vacate.
    ctx.manager_annual_salary = 0;
    assert_eq!(board.severance_patience_bonus(&ctx), 0);
}

/// …and the extra month is real: it lands on the threshold the sacking
/// ladder actually counts against.
#[test]
fn the_bill_shows_up_as_an_extra_month_of_patience() {
    let board = ClubBoard::new();
    let mut ctx = BoardContext::new();
    ctx.manager_annual_salary = 4_000_000;
    ctx.manager_contract_months_left = 36;

    ctx.balance = 400_000_000;
    let affordable = board.patience_threshold(&ctx);
    ctx.balance = 2_000_000;
    let unaffordable = board.patience_threshold(&ctx);

    assert_eq!(
        unaffordable,
        affordable + ClubBoard::SEVERANCE_PATIENCE_BONUS as u8,
        "the pay-off should buy exactly one more month"
    );
    // Still bounded: no board waits for ever.
    assert!((1..=12).contains(&unaffordable));
}

/// The bill itself, and who pays what share of it.
#[test]
fn the_owner_decides_how_much_of_the_deal_gets_settled() {
    assert_eq!(OwnershipType::StateBacked.severance_share(), 1.00);
    assert!(
        OwnershipType::MemberOwned.severance_share() < OwnershipType::Consortium.severance_share(),
        "a fan-owned club negotiates harder than a consortium"
    );
    // Every archetype settles something: nobody walks for nothing.
    for owner in [
        OwnershipType::StateBacked,
        OwnershipType::PrivateEquity,
        OwnershipType::Consortium,
        OwnershipType::LocalBusiness,
        OwnershipType::FamilyOwned,
        OwnershipType::MemberOwned,
    ] {
        assert!(owner.severance_share() >= 0.5);
    }
}

/// A demand the board can hold somebody to. `DemandPlayerSale` used to
/// record a headline and stop — no player, no price, no deadline, and
/// nothing that noticed whether the money came in.
#[test]
fn a_sale_mandate_is_judged_on_the_money_that_arrives() {
    let mut board = ClubBoard::new();
    let target = ForcedSaleTarget {
        player_id: 77,
        asking_price: 8_000_000.0,
    };
    let today = Scenario::season_start();
    // The club had already banked 3M of sales this season; the mandate must
    // measure what comes in AFTER it, not the running total.
    board.open_sale_mandate(target, 3_000_000.0, today);

    assert!(board.promises.has_active(PromiseType::SaleMandate));
    let mandate = board.sale_mandate.expect("the board named somebody");
    assert_eq!(mandate.player_id, 77);
    assert!(
        !mandate.is_satisfied_by(3_000_000.0),
        "nothing new has come in"
    );
    assert!(
        !mandate.is_satisfied_by(10_000_000.0),
        "7M of new income is short of the 8M asked for"
    );
    assert!(mandate.is_satisfied_by(11_000_000.0));

    // A second demand while one is outstanding names nobody new.
    board.open_sale_mandate(
        ForcedSaleTarget {
            player_id: 99,
            asking_price: 1.0,
        },
        3_000_000.0,
        today,
    );
    assert_eq!(board.sale_mandate.unwrap().player_id, 77);
}

/// …and the money closes it out, returning trust to the manager.
#[test]
fn money_in_the_door_closes_the_sale_mandate() {
    let mut board = ClubBoard::new();
    board.season_targets = Some(Scenario::targets(8, 13));
    let today = Scenario::season_start();
    board.open_sale_mandate(
        ForcedSaleTarget {
            player_id: 42,
            asking_price: 5_000_000.0,
        },
        0.0,
        today,
    );
    let trust_before = board.relationship.trust_communication;

    let mut ctx = Scenario::strong_ctx(8, 20);
    ctx.fees_received_this_season = 6_000_000.0;
    let mut result = BoardResult::new();
    board.resolve_promises(&ctx, today, &mut result);

    assert!(board.sale_mandate.is_none(), "the mandate is discharged");
    assert!(!board.promises.has_active(PromiseType::SaleMandate));
    assert!(result.promises_kept >= 1);
    assert!(board.relationship.trust_communication > trust_before);
}
