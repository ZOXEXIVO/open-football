use super::strategy::*;
use crate::club::board::{
    ClubVision, FinancialStance, SigningPreference, VisionPlayingStyle, VisionYouthFocus,
};
use crate::club::player::builder::PlayerBuilder;
use crate::shared::fullname::FullName;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::{
    TransferApproach, TransferNeedPriority, TransferNeedReason, TransferRequest,
};
use crate::{
    ClubPhilosophy, PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, PlayerStatusType,
};
use chrono::NaiveDate;

// ============================================================
// Test fixtures
// ============================================================

/// Fixtures for the strategy tests: a date, a player, a board vision, a
/// strategy context, and a way to pull one clause off an offer.
struct Fx;

impl Fx {
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
            ..Default::default()
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
}

// ============================================================
// Scenario tests
// ============================================================

#[test]
fn cash_rich_elite_club_offers_more_upfront_with_fewer_clauses() {
    // Same player, same asking price; an ambitious elite buyer
    // should put more cash on the table and attach fewer
    // installment-style clauses than an austerity buyer.
    let date = Fx::d(2026, 7, 1);
    let player = Fx::make_player(
        1,
        Fx::d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        140,
        150,
        Some(Fx::d(2029, 6, 30)),
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
        &Fx::vision(FinancialStance::Ambitious),
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
        &Fx::vision(FinancialStance::Austerity),
        0.35,
    );

    let mut ctx = Fx::ctx_for(date, 50_000_000.0);
    ctx.available_budget = 100_000_000.0;

    let amb_offer = ambitious.calculate_initial_offer_with_context(&player, &asking, &ctx);
    let aus_offer = austerity.calculate_initial_offer_with_context(&player, &asking, &ctx);

    assert!(
        Fx::money_amount(&amb_offer.base_fee) > Fx::money_amount(&aus_offer.base_fee),
        "ambitious offer ({:?}) should be larger than austerity ({:?})",
        amb_offer.base_fee,
        aus_offer.base_fee
    );
    // Austerity attaches an installments clause when fee ≥ 1.5M
    // and installment_preference is high; ambitious club skips it.
    assert!(
        Fx::find_clause(&aus_offer, "installments").is_some(),
        "austerity club should propose installments"
    );
    assert!(
        Fx::find_clause(&amb_offer, "installments").is_none(),
        "ambitious club should not propose installments"
    );
}

#[test]
fn develop_and_sell_attaches_sell_on_for_young_high_upside() {
    let date = Fx::d(2026, 7, 1);
    // 20yo prospect with 50pt potential gap.
    let player = Fx::make_player(
        3,
        Fx::d(2006, 1, 1),
        PlayerPositionType::MidfielderCenter,
        110,
        160,
        Some(Fx::d(2029, 6, 30)),
    );
    let asking = CurrencyValue {
        amount: 5_000_000.0,
        currency: Currency::Usd,
    };

    let mut v = Fx::vision(FinancialStance::Balanced);
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

    let mut ctx = Fx::ctx_for(date, 10_000_000.0);
    // The upside is the SCOUTS' belief — clubs can't read hidden PA, so
    // the pursuing club knows about the 50pt gap via its dossier.
    ctx.scout_assessed_ability = Some(110);
    ctx.scout_assessed_potential = Some(160);
    let offer = s.calculate_initial_offer_with_context(&player, &asking, &ctx);

    assert!(
        Fx::find_clause(&offer, "sell_on").is_some(),
        "develop-and-sell club should attach sell-on clause"
    );
    // Young prospect → 5-year contract for resale-value protection.
    assert_eq!(offer.contract_length_years, Some(5));
}

#[test]
fn loan_focused_club_skips_long_contract_under_loan_approach() {
    let date = Fx::d(2026, 7, 1);
    let player = Fx::make_player(
        4,
        Fx::d(2002, 1, 1),
        PlayerPositionType::MidfielderCenter,
        130,
        140,
        Some(Fx::d(2029, 6, 30)),
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
        &Fx::vision(FinancialStance::Conservative),
        0.4,
    );

    let mut ctx = Fx::ctx_for(date, 500_000.0);
    ctx.approach = TransferApproach::LoanWithOption;
    let offer = s.calculate_initial_offer_with_context(&player, &asking, &ctx);

    // Loan path: strategy returns the single-year placeholder
    // and leaves loan-specific clauses to the pipeline.
    assert_eq!(offer.contract_length_years, Some(1));
}

#[test]
fn loan_fee_stays_far_below_permanent_price() {
    let date = Fx::d(2026, 7, 1);
    // A valuable midfielder in his prime.
    let player = Fx::make_player(
        7,
        Fx::d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        150,
        160,
        Some(Fx::d(2029, 6, 30)),
    );

    let s = ClubTransferStrategy::from_club_context(
        1,
        Some(CurrencyValue {
            amount: 200_000_000.0,
            currency: Currency::Usd,
        }),
        85,
        vec![PlayerPositionType::MidfielderCenter],
        &ClubPhilosophy::Balanced,
        &Fx::vision(FinancialStance::Balanced),
        0.6,
    );

    // Permanent move: the buyer pays around the full asking.
    let permanent_asking = CurrencyValue {
        amount: 40_000_000.0,
        currency: Currency::Usd,
    };
    let perm_ctx = Fx::ctx_for(date, 100_000_000.0);
    let permanent = s.calculate_initial_offer_with_context(&player, &permanent_asking, &perm_ctx);

    // Loan move: the pipeline hands the strategy a loan FEE (a few percent
    // of the player's value), not his full price. The old anchor floored
    // the offer at 85% of full value, ballooning the loan fee up to nearly
    // the permanent price — this guards against that regression.
    let loan_fee_asking = CurrencyValue {
        amount: permanent_asking.amount * 0.06,
        currency: Currency::Usd,
    };
    let mut loan_ctx = Fx::ctx_for(date, 100_000_000.0);
    loan_ctx.approach = TransferApproach::Loan;
    let loan = s.calculate_initial_offer_with_context(&player, &loan_fee_asking, &loan_ctx);

    assert!(
        Fx::money_amount(&loan.base_fee) < Fx::money_amount(&permanent.base_fee) * 0.25,
        "loan fee {} must stay far below the permanent fee {}",
        Fx::money_amount(&loan.base_fee),
        Fx::money_amount(&permanent.base_fee),
    );
    // And it should track the advertised loan fee, not re-inflate past it.
    assert!(
        Fx::money_amount(&loan.base_fee) <= loan_fee_asking.amount * 1.1,
        "loan fee {} must stay near the advertised loan fee {}",
        Fx::money_amount(&loan.base_fee),
        loan_fee_asking.amount,
    );
}

#[test]
fn older_player_gets_shorter_contract_and_appearance_clause() {
    let date = Fx::d(2026, 7, 1);
    // 32-year-old veteran with a few goals.
    let mut player = Fx::make_player(
        5,
        Fx::d(1994, 1, 1),
        PlayerPositionType::Striker,
        140,
        140,
        Some(Fx::d(2027, 6, 30)),
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
        &Fx::vision(FinancialStance::Balanced),
        0.6,
    );

    let ctx = Fx::ctx_for(date, 10_000_000.0);
    let offer = s.calculate_initial_offer_with_context(&player, &asking, &ctx);

    // 32yo → short contract.
    assert_eq!(offer.contract_length_years, Some(1));
    assert!(
        Fx::find_clause(&offer, "appearance").is_some(),
        "veteran should get an appearance-fee clause"
    );
    assert!(
        Fx::find_clause(&offer, "goals").is_some(),
        "scoring forward should get a goal-bonus clause"
    );
}

#[test]
fn critical_request_pushes_offer_higher_than_optional() {
    // Same strategy, same player, same asking price — only the
    // request priority differs. Critical request should produce
    // a meaningfully larger offer.
    let date = Fx::d(2026, 7, 1);
    let player = Fx::make_player(
        6,
        Fx::d(2000, 1, 1),
        PlayerPositionType::DefenderCenter,
        135,
        140,
        Some(Fx::d(2029, 6, 30)),
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
        &Fx::vision(FinancialStance::Balanced),
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

    let mut critical_ctx = Fx::ctx_for(date, 20_000_000.0);
    critical_ctx.request = Some(&critical_req);
    let mut optional_ctx = Fx::ctx_for(date, 20_000_000.0);
    optional_ctx.request = Some(&optional_req);

    let critical_offer = s.calculate_initial_offer_with_context(&player, &asking, &critical_ctx);
    let optional_offer = s.calculate_initial_offer_with_context(&player, &asking, &optional_ctx);

    assert!(
        Fx::money_amount(&critical_offer.base_fee) > Fx::money_amount(&optional_offer.base_fee),
        "critical request ({}) should outbid optional ({})",
        Fx::money_amount(&critical_offer.base_fee),
        Fx::money_amount(&optional_offer.base_fee),
    );
}

#[test]
fn low_scout_confidence_reduces_offer_amount() {
    let date = Fx::d(2026, 7, 1);
    let player = Fx::make_player(
        7,
        Fx::d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        135,
        145,
        Some(Fx::d(2029, 6, 30)),
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
        &Fx::vision(FinancialStance::Balanced),
        0.5,
    );

    let mut high_ctx = Fx::ctx_for(date, 6_000_000.0);
    high_ctx.scout_confidence = Some(0.85);
    let mut low_ctx = Fx::ctx_for(date, 6_000_000.0);
    low_ctx.scout_confidence = Some(0.15);

    let high_offer = s.calculate_initial_offer_with_context(&player, &asking, &high_ctx);
    let low_offer = s.calculate_initial_offer_with_context(&player, &asking, &low_ctx);

    assert!(
        Fx::money_amount(&low_offer.base_fee) < Fx::money_amount(&high_offer.base_fee),
        "low scout confidence should reduce offer; high={}, low={}",
        Fx::money_amount(&high_offer.base_fee),
        Fx::money_amount(&low_offer.base_fee),
    );
}

#[test]
fn expiring_contract_and_listed_status_lower_offer() {
    let date = Fx::d(2026, 7, 1);
    let asking = CurrencyValue {
        amount: 6_000_000.0,
        currency: Currency::Usd,
    };

    // Baseline: long contract, not listed.
    let baseline_player = Fx::make_player(
        8,
        Fx::d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        135,
        140,
        Some(Fx::d(2029, 6, 30)),
    );
    // Distressed: 5 months left + transfer-listed.
    let mut distressed_player = Fx::make_player(
        9,
        Fx::d(2000, 1, 1),
        PlayerPositionType::MidfielderCenter,
        135,
        140,
        Some(Fx::d(2026, 12, 1)),
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
        &Fx::vision(FinancialStance::Balanced),
        0.55,
    );

    let ctx = Fx::ctx_for(date, 10_000_000.0);
    let baseline_offer = s.calculate_initial_offer_with_context(&baseline_player, &asking, &ctx);
    let distressed_offer =
        s.calculate_initial_offer_with_context(&distressed_player, &asking, &ctx);

    assert!(
        Fx::money_amount(&distressed_offer.base_fee) < Fx::money_amount(&baseline_offer.base_fee),
        "distressed seller ({}) should fetch less than baseline ({})",
        Fx::money_amount(&distressed_offer.base_fee),
        Fx::money_amount(&baseline_offer.base_fee),
    );
}
