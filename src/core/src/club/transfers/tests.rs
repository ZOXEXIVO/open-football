use super::strategy::*;
use crate::club::board::{
    ClubVision, FinancialStance, SigningPreference, VisionPlayingStyle, VisionYouthFocus,
};
use crate::club::player::builder::PlayerBuilder;
use crate::shared::fullname::FullName;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::{
    BoardRecruitmentDossier, TransferApproach, TransferNeedPriority, TransferNeedReason,
    TransferRequest,
};
use crate::{
    ClubPhilosophy, PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, PlayerStatusType,
};
use chrono::NaiveDate;

// ============================================================
// Test fixtures
// ============================================================

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Build a single-position player with the given age, CA/PA, and
/// optional contract expiration. Everything else is default so
/// tests stay focused on the strategy layer.
fn make_player(
    id: u32,
    birth: NaiveDate,
    position: PlayerPositionType,
    current_ability: u8,
    potential_ability: u8,
    contract_expiry: Option<NaiveDate>,
) -> Player {
    let mut player_attributes = PlayerAttributes::default();
    player_attributes.current_ability = current_ability;
    player_attributes.potential_ability = potential_ability;

    let mut p = PlayerBuilder::new()
        .id(id)
        .full_name(FullName::new("Test".into(), format!("P{}", id)))
        .birth_date(birth)
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(PlayerSkills::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position,
                level: 20,
            }],
        })
        .player_attributes(player_attributes)
        .build()
        .unwrap();
    p.contract = contract_expiry.map(|exp| PlayerClubContract::new(50_000, exp));
    p
}

fn vision(financial_stance: FinancialStance) -> ClubVision {
    ClubVision {
        playing_style: VisionPlayingStyle::Balanced,
        youth_focus: VisionYouthFocus::Balanced,
        signing_preference: SigningPreference::Anyone,
        financial_stance,
        long_term_goal: None,
        long_term_horizon_seasons: 3,
    }
}

fn ctx_for(date: NaiveDate, allocated: f64) -> TransferStrategyContext<'static> {
    let mut c = TransferStrategyContext::minimal(date);
    c.allocated_budget = allocated;
    c.available_budget = allocated;
    c
}

fn money_amount(c: &CurrencyValue) -> f64 {
    c.amount
}

// Pull the first matching clause off an offer so a test can
// inspect its payload (e.g. installment years, addon fee).
fn find_clause<'a>(offer: &'a TransferOffer, tag: &str) -> Option<&'a TransferClause> {
    offer.clauses.iter().find(|c| match (c, tag) {
        (TransferClause::SellOnClause(_), "sell_on") => true,
        (TransferClause::AppearanceFee(_, _), "appearance") => true,
        (TransferClause::GoalBonus(_, _), "goals") => true,
        (TransferClause::PromotionBonus(_), "promotion") => true,
        (TransferClause::Installments(_, _), "installments") => true,
        (TransferClause::LoanOptionToBuy(_), "loan_option") => true,
        (TransferClause::LoanObligationToBuy(_), "loan_obligation") => true,
        _ => false,
    })
}

// ============================================================
// Scenario tests
// ============================================================

#[test]
fn cash_rich_elite_club_offers_more_upfront_with_fewer_clauses() {
    // Same player, same asking price; an ambitious elite buyer
    // should put more cash on the table and attach fewer
    // installment-style clauses than an austerity buyer.
    let date = d(2026, 7, 1);
    let player = make_player(
        1,
        d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        140,
        150,
        Some(d(2029, 6, 30)),
    );
    let asking = CurrencyValue {
        amount: 10_000_000.0,
        currency: Currency::Usd,
    };

    let ambitious = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 100_000_000.0,
            currency: Currency::Usd,
        }),
        90,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::SignToCompete,
        &vision(FinancialStance::Ambitious),
        0.85,
    );
    let austerity = ClubTransferStrategy::from_club_context(
        2,
        Some(CurrencyValue {
            amount: 100_000_000.0,
            currency: Currency::Usd,
        }),
        90,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::Balanced,
        &vision(FinancialStance::Austerity),
        0.35,
    );

    let mut ctx = ctx_for(date, 50_000_000.0);
    ctx.available_budget = 100_000_000.0;

    let amb_offer = ambitious.calculate_initial_offer_with_context(&player, &asking, &ctx);
    let aus_offer = austerity.calculate_initial_offer_with_context(&player, &asking, &ctx);

    assert!(
        money_amount(&amb_offer.base_fee) > money_amount(&aus_offer.base_fee),
        "ambitious offer ({:?}) should be larger than austerity ({:?})",
        amb_offer.base_fee,
        aus_offer.base_fee
    );
    // Austerity attaches an installments clause when fee ≥ 1.5M
    // and installment_preference is high; ambitious club skips it.
    assert!(
        find_clause(&aus_offer, "installments").is_some(),
        "austerity club should propose installments"
    );
    assert!(
        find_clause(&amb_offer, "installments").is_none(),
        "ambitious club should not propose installments"
    );
}

#[test]
fn develop_and_sell_attaches_sell_on_for_young_high_upside() {
    let date = d(2026, 7, 1);
    // 20yo prospect with 50pt potential gap.
    let player = make_player(
        3,
        d(2006, 1, 1),
        PlayerPositionType::MidfielderCenter,
        110,
        160,
        Some(d(2029, 6, 30)),
    );
    let asking = CurrencyValue {
        amount: 5_000_000.0,
        currency: Currency::Usd,
    };

    let mut v = vision(FinancialStance::Balanced);
    v.youth_focus = VisionYouthFocus::DevelopYouth;
    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 30_000_000.0,
            currency: Currency::Usd,
        }),
        70,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::DevelopAndSell,
        &v,
        0.6,
    );

    let ctx = ctx_for(date, 10_000_000.0);
    let offer = s.calculate_initial_offer_with_context(&player, &asking, &ctx);

    assert!(
        find_clause(&offer, "sell_on").is_some(),
        "develop-and-sell club should attach sell-on clause"
    );
    // Young prospect → 5-year contract for resale-value protection.
    assert_eq!(offer.contract_length, Some(5));
}

#[test]
fn loan_focused_club_skips_long_contract_under_loan_approach() {
    let date = d(2026, 7, 1);
    let player = make_player(
        4,
        d(2002, 1, 1),
        PlayerPositionType::MidfielderCenter,
        130,
        140,
        Some(d(2029, 6, 30)),
    );
    let asking = CurrencyValue {
        amount: 500_000.0,
        currency: Currency::Usd,
    };

    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 2_000_000.0,
            currency: Currency::Usd,
        }),
        45,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::LoanFocused,
        &vision(FinancialStance::Conservative),
        0.4,
    );

    let mut ctx = ctx_for(date, 500_000.0);
    ctx.approach = TransferApproach::LoanWithOption;
    let offer = s.calculate_initial_offer_with_context(&player, &asking, &ctx);

    // Loan path: strategy returns the single-year placeholder
    // and leaves loan-specific clauses to the pipeline.
    assert_eq!(offer.contract_length, Some(1));
}

#[test]
fn older_player_gets_shorter_contract_and_appearance_clause() {
    let date = d(2026, 7, 1);
    // 32-year-old veteran with a few goals.
    let mut player = make_player(
        5,
        d(1994, 1, 1),
        PlayerPositionType::Striker,
        140,
        140,
        Some(d(2027, 6, 30)),
    );
    player.statistics.goals = 12;
    let asking = CurrencyValue {
        amount: 4_000_000.0,
        currency: Currency::Usd,
    };

    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 30_000_000.0,
            currency: Currency::Usd,
        }),
        80,
        vec![PlayerPositionType::Striker],
        &ClubPhilosophy::Balanced,
        &vision(FinancialStance::Balanced),
        0.6,
    );

    let ctx = ctx_for(date, 10_000_000.0);
    let offer = s.calculate_initial_offer_with_context(&player, &asking, &ctx);

    // 32yo → short contract.
    assert_eq!(offer.contract_length, Some(1));
    assert!(
        find_clause(&offer, "appearance").is_some(),
        "veteran should get an appearance-fee clause"
    );
    assert!(
        find_clause(&offer, "goals").is_some(),
        "scoring forward should get a goal-bonus clause"
    );
}

#[test]
fn critical_request_pushes_offer_higher_than_optional() {
    // Same strategy, same player, same asking price — only the
    // request priority differs. Critical request should produce
    // a meaningfully larger offer.
    let date = d(2026, 7, 1);
    let player = make_player(
        6,
        d(2000, 1, 1),
        PlayerPositionType::DefenderCenter,
        135,
        140,
        Some(d(2029, 6, 30)),
    );
    let asking = CurrencyValue {
        amount: 8_000_000.0,
        currency: Currency::Usd,
    };

    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 20_000_000.0,
            currency: Currency::Usd,
        }),
        80,
        vec![PlayerPositionType::DefenderCenter],
        &ClubPhilosophy::Balanced,
        &vision(FinancialStance::Balanced),
        0.6,
    );

    let critical_req = TransferRequest::new(
        1,
        PlayerPositionType::DefenderCenter,
        TransferNeedPriority::Critical,
        TransferNeedReason::QualityUpgrade,
        130,
        145,
        20_000_000.0,
    );
    let optional_req = TransferRequest::new(
        2,
        PlayerPositionType::DefenderCenter,
        TransferNeedPriority::Optional,
        TransferNeedReason::DepthCover,
        110,
        135,
        20_000_000.0,
    );

    let mut critical_ctx = ctx_for(date, 20_000_000.0);
    critical_ctx.request = Some(&critical_req);
    let mut optional_ctx = ctx_for(date, 20_000_000.0);
    optional_ctx.request = Some(&optional_req);

    let critical_offer = s.calculate_initial_offer_with_context(&player, &asking, &critical_ctx);
    let optional_offer = s.calculate_initial_offer_with_context(&player, &asking, &optional_ctx);

    assert!(
        money_amount(&critical_offer.base_fee) > money_amount(&optional_offer.base_fee),
        "critical request ({}) should outbid optional ({})",
        money_amount(&critical_offer.base_fee),
        money_amount(&optional_offer.base_fee),
    );
}

#[test]
fn low_scout_confidence_reduces_offer_amount() {
    let date = d(2026, 7, 1);
    let player = make_player(
        7,
        d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        135,
        145,
        Some(d(2029, 6, 30)),
    );
    let asking = CurrencyValue {
        amount: 5_000_000.0,
        currency: Currency::Usd,
    };

    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 15_000_000.0,
            currency: Currency::Usd,
        }),
        70,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::Balanced,
        &vision(FinancialStance::Balanced),
        0.5,
    );

    let mut high_ctx = ctx_for(date, 6_000_000.0);
    high_ctx.scout_confidence = Some(0.85);
    let mut low_ctx = ctx_for(date, 6_000_000.0);
    low_ctx.scout_confidence = Some(0.15);

    let high_offer = s.calculate_initial_offer_with_context(&player, &asking, &high_ctx);
    let low_offer = s.calculate_initial_offer_with_context(&player, &asking, &low_ctx);

    assert!(
        money_amount(&low_offer.base_fee) < money_amount(&high_offer.base_fee),
        "low scout confidence should reduce offer; high={}, low={}",
        money_amount(&high_offer.base_fee),
        money_amount(&low_offer.base_fee),
    );
}

#[test]
fn expiring_contract_and_listed_status_lower_offer() {
    let date = d(2026, 7, 1);
    let asking = CurrencyValue {
        amount: 6_000_000.0,
        currency: Currency::Usd,
    };

    // Baseline: long contract, not listed.
    let baseline_player = make_player(
        8,
        d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        135,
        140,
        Some(d(2029, 6, 30)),
    );
    // Distressed: 5 months left + transfer-listed.
    let mut distressed_player = make_player(
        9,
        d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        135,
        140,
        Some(d(2026, 12, 1)),
    );
    distressed_player.statuses.add(date, PlayerStatusType::Lst);

    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 30_000_000.0,
            currency: Currency::Usd,
        }),
        75,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::Balanced,
        &vision(FinancialStance::Balanced),
        0.55,
    );

    let ctx = ctx_for(date, 10_000_000.0);
    let baseline_offer = s.calculate_initial_offer_with_context(&baseline_player, &asking, &ctx);
    let distressed_offer =
        s.calculate_initial_offer_with_context(&distressed_player, &asking, &ctx);

    assert!(
        money_amount(&distressed_offer.base_fee) < money_amount(&baseline_offer.base_fee),
        "distressed seller ({}) should fetch less than baseline ({})",
        money_amount(&distressed_offer.base_fee),
        money_amount(&baseline_offer.base_fee),
    );
}

#[test]
fn evaluate_interest_passes_when_scouting_confidence_far_below_threshold() {
    let date = d(2026, 7, 1);
    let player = make_player(
        10,
        d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        130,
        140,
        Some(d(2029, 6, 30)),
    );

    // Conservative club requires high min scouting confidence.
    let s = ClubTransferStrategy::from_club_context(
        1,
        None,
        70,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::Balanced,
        &vision(FinancialStance::Conservative),
        0.5,
    );

    let mut ctx = TransferStrategyContext::minimal(date);
    ctx.scout_confidence = Some(0.10);
    let score = s.evaluate_interest(&player, &ctx);
    assert!(
        score
            .risks
            .contains(&TransferInterestRisk::LowScoutingConfidence),
        "low scout confidence should be flagged as a risk"
    );
}

#[test]
fn evaluate_interest_pursues_when_priority_and_scout_support_align() {
    let date = d(2026, 7, 1);
    let player = make_player(
        11,
        d(2002, 1, 1),
        PlayerPositionType::AttackingMidfielderCenter,
        140,
        160,
        Some(d(2029, 6, 30)),
    );

    let s = ClubTransferStrategy::from_club_context(
        1,
        None,
        80,
        vec![PlayerPositionType::AttackingMidfielderCenter],
        &ClubPhilosophy::SignToCompete,
        &vision(FinancialStance::Ambitious),
        0.75,
    );

    let req = TransferRequest::new(
        1,
        PlayerPositionType::AttackingMidfielderCenter,
        TransferNeedPriority::Critical,
        TransferNeedReason::QualityUpgrade,
        130,
        150,
        50_000_000.0,
    );
    let dossier = BoardRecruitmentDossier {
        player_id: 11,
        scout_votes: 3,
        chief_scout_support: true,
        avg_confidence: 0.8,
        avg_role_fit: 1.05,
        risk_flag_count: 0,
        consensus_score: 2.0,
        budget_fit: 0.7,
        data_support: true,
        matches_watched: 8,
    };

    let mut ctx = TransferStrategyContext::minimal(date);
    ctx.request = Some(&req);
    ctx.board_dossier = Some(&dossier);
    ctx.scout_confidence = Some(0.85);
    ctx.scout_assessed_ability = Some(138);
    ctx.scout_assessed_potential = Some(158);

    let score = s.evaluate_interest(&player, &ctx);
    assert!(
        matches!(score.decision, TransferInterestDecision::Pursue),
        "expected Pursue, got {:?} (score={})",
        score.decision,
        score.score
    );
    assert!(
        score
            .reasons
            .contains(&TransferInterestReason::PriorityRequest),
        "priority should be cited as a reason"
    );
}

#[test]
fn back_compat_calculate_initial_offer_produces_finite_offer() {
    // The old single-signature API must still produce a usable
    // offer so existing call sites and downstream tests keep working.
    let date = d(2026, 7, 1);
    let player = make_player(
        12,
        d(2000, 1, 1),
        PlayerPositionType::DefenderCenter,
        130,
        140,
        Some(d(2029, 6, 30)),
    );
    let s = ClubTransferStrategy::new(1);
    let asking = CurrencyValue {
        amount: 2_000_000.0,
        currency: Currency::Usd,
    };
    let offer = s.calculate_initial_offer(&player, &asking, date);
    assert!(offer.base_fee.amount > 0.0);
}

#[test]
fn selling_decision_rejects_rival_for_homegrown_key_player() {
    let date = d(2026, 7, 1);
    let mut player = make_player(
        13,
        d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        160,
        165,
        Some(d(2030, 6, 30)),
    );
    player.statuses.add(date, PlayerStatusType::HG);

    let mut s = ClubTransferStrategy::new(1);
    s.selling.rival_resistance = 0.9;
    s.selling.keep_homegrown_bias = 0.8;
    s.selling.willingness_baseline = 0.4;

    let decision = s.evaluate_sale(&player, date, true, 1);
    assert_eq!(decision, SellingDecision::Reject);
}

#[test]
fn selling_decision_encourages_when_listed_and_aging_with_depth() {
    let date = d(2026, 7, 1);
    // 32-year-old aging backup.
    let mut player = make_player(
        14,
        d(1994, 1, 1),
        PlayerPositionType::MidfielderCenter,
        125,
        125,
        Some(d(2026, 12, 31)),
    );
    player.statuses.add(date, PlayerStatusType::Lst);

    let mut s = ClubTransferStrategy::new(1);
    s.selling.willingness_baseline = 0.5;
    s.selling.sell_aging_bias = 0.8;
    s.selling.sell_surplus_bias = 0.8;
    s.selling.cash_pressure = 0.5;

    let decision = s.evaluate_sale(&player, date, false, 4);
    assert_eq!(decision, SellingDecision::Encourage);
}
