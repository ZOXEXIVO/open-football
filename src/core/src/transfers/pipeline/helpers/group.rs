//! Moved verbatim out of `helpers.rs` — see that file's `mod group_need_tests`.

use crate::club::team::squad::SquadAssetClass;
use crate::transfers::gate::fit::SquadFitSnapshot;
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::pipeline::processor::SquadPlayerInfo;
use crate::transfers::squad::{
    GroupNeed, GroupNeedScan, NeedKind, SuccessionAudit, SuccessionUrgency,
};
use crate::{MatchTacticType, PlayerFieldPositionGroup, PlayerPositionType, TACTICS_POSITIONS};
use std::collections::HashMap;

/// Squads, formations and tier anchors the group-need tests build on.
struct GroupFx;

impl GroupFx {
    fn t442_positions() -> &'static [PlayerPositionType; 11] {
        let (_, positions) = TACTICS_POSITIONS
            .iter()
            .find(|(t, _)| *t == MatchTacticType::T442)
            .expect("T442 tactic must exist");
        positions
    }

    fn squad_player(id: u32, primary: PlayerPositionType, ca: u8) -> SquadPlayerInfo {
        let mut levels: HashMap<PlayerPositionType, u8> = HashMap::new();
        levels.insert(primary, 20);
        SquadPlayerInfo {
            player_id: id,
            primary_position: primary,
            current_ability: ca,
            estimated_potential: ca,
            potential_confidence: 0.5,
            age: 26,
            position_levels: levels,
            appearances: 10,
            official_appearances: 10,
            is_injured: false,
            recovery_days: 0,
            injury_days: 0,
            asset_class: SquadAssetClass::UnknownNeedsEvaluation,
            contract_months_remaining: Some(24),
        }
    }

    /// Build position_coverage with each formation slot covered by the
    /// best-fit squad player. Mirrors the production logic enough to
    /// drive the detector deterministically.
    fn coverage_from_squad(
        squad: &[SquadPlayerInfo],
        formation: &[PlayerPositionType; 11],
    ) -> Vec<(PlayerPositionType, Option<u32>, u8)> {
        let mut used: Vec<u32> = Vec::new();
        let mut out = Vec::new();
        for &slot in formation.iter() {
            let pick = squad
                .iter()
                .filter(|p| !used.contains(&p.player_id))
                .filter(|p| p.primary_position.position_group() == slot.position_group())
                .max_by_key(|p| p.current_ability);
            match pick {
                Some(p) => {
                    used.push(p.player_id);
                    out.push((slot, Some(p.player_id), p.current_ability));
                }
                None => out.push((slot, None, 0)),
            }
        }
        out
    }

    fn continental_score() -> f32 {
        0.725
    }

    fn continental_tolerance() -> i16 {
        PipelineProcessor::tier_quality_tolerance_score(Self::continental_score())
    }

    fn aged_player(
        id: u32,
        primary: PlayerPositionType,
        ca: u8,
        age: u8,
        potential: u8,
    ) -> SquadPlayerInfo {
        let mut p = Self::squad_player(id, primary, ca);
        p.age = age;
        p.estimated_potential = potential;
        p
    }

    /// The ageing centre-back every succession test is written around.
    fn aging_incumbent() -> SquadPlayerInfo {
        Self::aged_player(1, PlayerPositionType::DefenderCenterLeft, 140, 32, 140)
    }
}

// ── Succession audit ────────────────────────────────────────

#[test]
fn succession_career_end_is_position_aware() {
    assert!(
        SuccessionAudit::career_end_age(PlayerFieldPositionGroup::Forward)
            < SuccessionAudit::career_end_age(PlayerFieldPositionGroup::Goalkeeper),
        "keeper careers run longer, so their succession horizon starts later"
    );
}

/// The horizon escalates instead of latching. A keeper who sails past
/// the old fixed trigger age and keeps playing used to read exactly
/// like one who had just reached it; now the club's urgency grows as
/// the career it depends on runs out.
#[test]
fn succession_urgency_escalates_with_the_years_left() {
    let keeper = |age: u8| GroupFx::aged_player(9, PlayerPositionType::Goalkeeper, 130, age, 130);
    assert_eq!(SuccessionAudit::urgency(&keeper(30)), None);
    assert_eq!(
        SuccessionAudit::urgency(&keeper(33)),
        Some(SuccessionUrgency::Watch)
    );
    assert_eq!(
        SuccessionAudit::urgency(&keeper(35)),
        Some(SuccessionUrgency::Pressing)
    );
    assert_eq!(
        SuccessionAudit::urgency(&keeper(40)),
        Some(SuccessionUrgency::Critical),
        "a forty-year-old first choice is the most urgent succession a club can have"
    );
}

/// The test that matters for the Juventus case: two career deputies
/// four years younger than a forty-year-old incumbent are not a
/// succession plan, and must not cancel the search.
#[test]
fn ageing_deputies_do_not_count_as_the_heir() {
    let squad = vec![
        GroupFx::aged_player(1, PlayerPositionType::Goalkeeper, 130, 40, 130),
        GroupFx::aged_player(2, PlayerPositionType::Goalkeeper, 120, 29, 122),
        GroupFx::aged_player(3, PlayerPositionType::Goalkeeper, 118, 29, 120),
    ];
    let incumbent = GroupFx::aged_player(1, PlayerPositionType::Goalkeeper, 130, 40, 130);
    assert!(
        !SuccessionAudit::heir_in_place(&squad, &incumbent),
        "peers who will retire alongside him are not successors"
    );
}

#[test]
fn a_genuine_young_heir_still_blocks_the_search() {
    let squad = vec![
        GroupFx::aged_player(1, PlayerPositionType::Goalkeeper, 130, 38, 130),
        GroupFx::aged_player(2, PlayerPositionType::Goalkeeper, 108, 21, 132),
    ];
    let incumbent = GroupFx::aged_player(1, PlayerPositionType::Goalkeeper, 130, 38, 130);
    assert!(
        SuccessionAudit::heir_in_place(&squad, &incumbent),
        "a 21-year-old assessed to reach the incumbent's level is exactly the heir"
    );
}

#[test]
fn heir_already_at_level_blocks_succession_shopping() {
    let squad = vec![
        GroupFx::aging_incumbent(),
        // A 24-year-old already within touching distance of the level.
        GroupFx::aged_player(2, PlayerPositionType::DefenderCenterRight, 130, 24, 138),
    ];
    assert!(SuccessionAudit::heir_in_place(
        &squad,
        &GroupFx::aging_incumbent()
    ));
}

#[test]
fn heir_by_assessed_potential_counts() {
    let squad = vec![
        GroupFx::aging_incumbent(),
        // Raw today, but the scouts assess him as growing into it.
        GroupFx::aged_player(2, PlayerPositionType::DefenderCenterRight, 118, 22, 145),
    ];
    assert!(SuccessionAudit::heir_in_place(
        &squad,
        &GroupFx::aging_incumbent()
    ));
}

#[test]
fn no_heir_when_cover_is_old_or_below_level() {
    let squad = vec![
        GroupFx::aging_incumbent(),
        // Same age band — a peer, not a successor.
        GroupFx::aged_player(2, PlayerPositionType::DefenderCenterRight, 138, 30, 138),
        // Young but nowhere near the level, and not assessed to reach it.
        GroupFx::aged_player(3, PlayerPositionType::DefenderCenterLeft, 100, 21, 120),
    ];
    assert!(!SuccessionAudit::heir_in_place(
        &squad,
        &GroupFx::aging_incumbent()
    ));
}

#[test]
fn weak_gk_at_continental_club_triggers_quality_upgrade() {
    // Continental tier squad: every outfield slot at-baseline,
    // GK well below tier baseline. Detector must produce exactly
    // one QualityUpgrade need targeting the goalkeeper group.
    let formation = GroupFx::t442_positions();
    let mut squad = Vec::new();
    squad.push(GroupFx::squad_player(
        1,
        PlayerPositionType::Goalkeeper,
        110,
    ));
    squad.push(GroupFx::squad_player(2, PlayerPositionType::Goalkeeper, 95));
    // Outfield: at-tier defenders / mids / forwards
    let outfield_positions = [
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    for (i, pos) in outfield_positions.iter().enumerate() {
        squad.push(GroupFx::squad_player(10 + i as u32, *pos, 132));
    }
    // Add a couple of bench outfielders so depth checks pass
    squad.push(GroupFx::squad_player(
        50,
        PlayerPositionType::DefenderCenterLeft,
        120,
    ));
    squad.push(GroupFx::squad_player(
        51,
        PlayerPositionType::DefenderCenterRight,
        120,
    ));
    squad.push(GroupFx::squad_player(
        52,
        PlayerPositionType::MidfielderCenterLeft,
        120,
    ));
    squad.push(GroupFx::squad_player(
        53,
        PlayerPositionType::MidfielderCenterRight,
        120,
    ));
    squad.push(GroupFx::squad_player(
        54,
        PlayerPositionType::ForwardLeft,
        118,
    ));

    let coverage = GroupFx::coverage_from_squad(&squad, formation);
    let needs: Vec<GroupNeed> = GroupNeedScan::needs(
        &squad,
        &coverage,
        formation,
        GroupFx::continental_score(),
        GroupFx::continental_tolerance(),
    );

    let gk_needs: Vec<&GroupNeed> = needs
        .iter()
        .filter(|n| n.group == PlayerFieldPositionGroup::Goalkeeper)
        .collect();
    assert_eq!(
        gk_needs.len(),
        1,
        "expected exactly one GK need, got {:?}",
        needs
    );
    assert_eq!(
        gk_needs[0].kind,
        NeedKind::QualityUpgrade,
        "expected QualityUpgrade for weak GK, got {:?}",
        gk_needs[0].kind
    );
}

#[test]
fn duplicate_formation_slots_emit_one_group_need() {
    // 4-back formation has four defender slots — if all are gaps,
    // detector must collapse to ONE FormationGap defender entry,
    // not four. This is the budget-distortion bug being pinned.
    let formation = GroupFx::t442_positions();
    let mut squad = Vec::new();
    squad.push(GroupFx::squad_player(
        1,
        PlayerPositionType::Goalkeeper,
        130,
    ));
    squad.push(GroupFx::squad_player(
        2,
        PlayerPositionType::Goalkeeper,
        125,
    ));
    // No defenders at all
    // At-tier mids / fwds
    for (i, pos) in [
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ]
    .iter()
    .enumerate()
    {
        squad.push(GroupFx::squad_player(20 + i as u32, *pos, 135));
    }

    let coverage = GroupFx::coverage_from_squad(&squad, formation);
    let needs = GroupNeedScan::needs(
        &squad,
        &coverage,
        formation,
        GroupFx::continental_score(),
        GroupFx::continental_tolerance(),
    );

    let defender_needs: Vec<&GroupNeed> = needs
        .iter()
        .filter(|n| n.group == PlayerFieldPositionGroup::Defender)
        .collect();
    assert_eq!(
        defender_needs.len(),
        1,
        "four empty defender slots must collapse to one need (got {})",
        defender_needs.len()
    );
    assert_eq!(defender_needs[0].kind, NeedKind::FormationGap);
}

#[test]
fn long_term_injury_stops_counting_toward_depth() {
    // Six healthy defenders exactly meet the 4-4-2 defender depth
    // requirement (4 slots + 2) → no need. Put one out long-term and the
    // club is genuinely short right now, so a defender need must appear.
    let formation = GroupFx::t442_positions();
    let make = |injure: bool| -> Vec<GroupNeed> {
        let mut squad = vec![
            GroupFx::squad_player(1, PlayerPositionType::Goalkeeper, 138),
            GroupFx::squad_player(2, PlayerPositionType::Goalkeeper, 130),
        ];
        let defs = [
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
        ];
        for (i, pos) in defs.iter().enumerate() {
            let mut p = GroupFx::squad_player(10 + i as u32, *pos, 138);
            if injure && i == 0 {
                p.is_injured = true;
                p.recovery_days = 60;
            }
            squad.push(p);
        }
        let mids = [
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
        ];
        for (i, pos) in mids.iter().enumerate() {
            squad.push(GroupFx::squad_player(30 + i as u32, *pos, 138));
        }
        for (i, pos) in [
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
            PlayerPositionType::Striker,
        ]
        .iter()
        .enumerate()
        {
            squad.push(GroupFx::squad_player(50 + i as u32, *pos, 138));
        }
        let coverage = GroupFx::coverage_from_squad(&squad, formation);
        GroupNeedScan::needs(
            &squad,
            &coverage,
            formation,
            GroupFx::continental_score(),
            GroupFx::continental_tolerance(),
        )
    };
    let has_def_need = |needs: &[GroupNeed]| {
        needs
            .iter()
            .any(|n| n.group == PlayerFieldPositionGroup::Defender)
    };
    assert!(
        !has_def_need(&make(false)),
        "six healthy defenders → no need"
    );
    assert!(
        has_def_need(&make(true)),
        "a long-term-injured defender drops available depth below requirement"
    );
}

#[test]
fn fully_at_tier_squad_yields_no_needs() {
    // A balanced squad at-tier in every group: no FormationGap,
    // no QualityUpgrade, no DepthCover. Universal calibration
    // sanity — over-firing here would create phantom requests.
    let formation = GroupFx::t442_positions();
    let mut squad = Vec::new();
    squad.push(GroupFx::squad_player(
        1,
        PlayerPositionType::Goalkeeper,
        130,
    ));
    squad.push(GroupFx::squad_player(
        2,
        PlayerPositionType::Goalkeeper,
        125,
    ));
    let outfield = [
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    for (i, pos) in outfield.iter().enumerate() {
        squad.push(GroupFx::squad_player(10 + i as u32, *pos, 138));
    }
    // Bench depth so depth-cover doesn't fire
    squad.push(GroupFx::squad_player(
        40,
        PlayerPositionType::DefenderCenterLeft,
        130,
    ));
    squad.push(GroupFx::squad_player(
        41,
        PlayerPositionType::DefenderCenterRight,
        130,
    ));
    squad.push(GroupFx::squad_player(
        42,
        PlayerPositionType::MidfielderCenterLeft,
        130,
    ));
    squad.push(GroupFx::squad_player(
        43,
        PlayerPositionType::MidfielderCenterRight,
        130,
    ));
    squad.push(GroupFx::squad_player(
        44,
        PlayerPositionType::ForwardLeft,
        128,
    ));

    let coverage = GroupFx::coverage_from_squad(&squad, formation);
    let needs = GroupNeedScan::needs(
        &squad,
        &coverage,
        formation,
        GroupFx::continental_score(),
        GroupFx::continental_tolerance(),
    );

    assert!(
        needs.is_empty(),
        "balanced at-tier squad should not generate any need (got {:?})",
        needs
    );
}

#[test]
fn sweep_realistic_continental_acceptance_and_realism_gates() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    // Continental club — Spartak-like context.
    let buyer = |open_request: bool, weak_group: bool| BuyerContext {
        buyer_rep_score: 0.72,
        buyer_world_rep: 5800,
        buyer_league_reputation: 5500,
        buyer_total_wages: 30_000_000,
        buyer_wage_budget: 60_000_000,
        plan_total_budget: 30_000_000.0,
        max_recommend_value: 60_000_000.0,
        // Weak group: starter at 105 (under tier baseline).
        // Otherwise: starter at 130 (tier baseline).
        buyer_best_in_group: if weak_group { 105 } else { 130 },
        has_open_request: open_request,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    // Mikhailov-class candidate: 14M, CA 130, listed, age 25.
    let mikhailov_class = ListedTargetView {
        nationality_country_id: 0,
        ability: 130,
        estimated_potential: 138,
        age: 25,
        estimated_value: 14_000_000.0,
        position_group: PlayerFieldPositionGroup::Forward,
        is_listed: false,
        is_transfer_requested: true,
        is_unhappy: true,
        world_reputation: 5200,
        current_reputation: 5000,
        ambition: 0.7,
        parent_club_score: 0.40, // smaller club
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    // Acceptance: weak group + an actual upgrade
    let v = ListedTargetScreen::evaluate(&mikhailov_class, &buyer(false, true));
    match v {
        ListedTargetVerdict::Accept(score) => {
            assert!(score > 10.0, "expected meaningful score, got {}", score);
        }
        ListedTargetVerdict::Reject(r) => panic!("expected Accept, got Reject({:?})", r),
    }

    // Open request also unlocks the path even when group is at-tier
    let v2 = ListedTargetScreen::evaluate(&mikhailov_class, &buyer(true, false));
    assert!(matches!(v2, ListedTargetVerdict::Accept(_)));

    // No need + only a marginal upgrade → NotAnUpgrade reject
    let mut marginal = mikhailov_class;
    marginal.ability = 132;
    let buyer_no_need = buyer(false, false); // best=130
    let v3 = ListedTargetScreen::evaluate(&marginal, &buyer_no_need);
    assert_eq!(
        v3,
        ListedTargetVerdict::Reject(ListedRejectReason::NotAnUpgrade)
    );

    // Squad-fit gate: the same otherwise-acceptable candidate is
    // rejected when the buyer's own surplus maths would list him —
    // well below the squad average (155 avg, gap 20 → bar 135) even
    // though the position group itself is weak.
    let mut surplus_buyer = buyer(false, true);
    surplus_buyer.fit = SquadFitSnapshot {
        foreign_slots_free: None,
        club_country_id: 0,
        squad_avg_ability: 155,
        quality_gap: 20,
        group_size: 0,
        group_cap: usize::MAX,
        group_cap_bar: 0,
        prospect_desk_full: false,
    };
    let v4 = ListedTargetScreen::evaluate(&mikhailov_class, &surplus_buyer);
    assert_eq!(
        v4,
        ListedTargetVerdict::Reject(ListedRejectReason::WouldBeSurplus)
    );

    // Depth-cap arm: a full group whose cap-th best (140) outranks the
    // candidate (130) → he'd be demoted by the weekly rebalance.
    let mut full_group_buyer = buyer(false, true);
    full_group_buyer.fit = SquadFitSnapshot {
        foreign_slots_free: None,
        club_country_id: 0,
        squad_avg_ability: 0,
        quality_gap: 0,
        group_size: 6,
        group_cap: 6,
        group_cap_bar: 140,
        prospect_desk_full: false,
    };
    let v5 = ListedTargetScreen::evaluate(&mikhailov_class, &full_group_buyer);
    assert_eq!(
        v5,
        ListedTargetVerdict::Reject(ListedRejectReason::WouldBeSurplus)
    );
}

#[test]
fn sweep_rejects_unaffordable_fee() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    let small_buyer = BuyerContext {
        buyer_rep_score: 0.40,
        buyer_world_rep: 2400,
        buyer_league_reputation: 3000,
        buyer_total_wages: 1_000_000,
        buyer_wage_budget: 1_500_000,
        plan_total_budget: 500_000.0,
        max_recommend_value: 1_000_000.0,
        buyer_best_in_group: 75,
        has_open_request: true,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    // Asking 5M when budget allows ~700k → UnaffordableFee
    let pricey = ListedTargetView {
        nationality_country_id: 0,
        ability: 95,
        estimated_potential: 100,
        age: 26,
        estimated_value: 5_000_000.0,
        position_group: PlayerFieldPositionGroup::Midfielder,
        is_listed: true,
        is_transfer_requested: false,
        is_unhappy: false,
        world_reputation: 2500,
        current_reputation: 1500,
        ambition: 0.5,
        parent_club_score: 0.55,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };
    assert_eq!(
        ListedTargetScreen::evaluate(&pricey, &small_buyer),
        ListedTargetVerdict::Reject(ListedRejectReason::UnaffordableFee)
    );
}

#[test]
fn sweep_rejects_unaffordable_wage_when_headroom_is_exhausted() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    // Wage budget barely above current spend → almost no headroom.
    // Even an at-tier player at this club would exceed the wage cap.
    let cap_strapped = BuyerContext {
        buyer_rep_score: 0.40,
        buyer_world_rep: 2400,
        buyer_league_reputation: 3000,
        buyer_total_wages: 1_000_000,
        buyer_wage_budget: 1_010_000, // 10k headroom × 1.3 = 13k cap
        plan_total_budget: 5_000_000.0,
        max_recommend_value: 10_000_000.0,
        buyer_best_in_group: 75,
        has_open_request: true,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    let in_tier_listed = ListedTargetView {
        nationality_country_id: 0,
        ability: 90,
        estimated_potential: 95,
        age: 27,
        estimated_value: 200_000.0, // fee comfortably affordable
        position_group: PlayerFieldPositionGroup::Midfielder,
        is_listed: true,
        is_transfer_requested: false,
        is_unhappy: false,
        world_reputation: 2200,
        current_reputation: 800,
        ambition: 0.5,
        parent_club_score: 0.55,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    assert_eq!(
        ListedTargetScreen::evaluate(&in_tier_listed, &cap_strapped),
        ListedTargetVerdict::Reject(ListedRejectReason::UnaffordableWage)
    );
}

#[test]
fn sweep_rejects_world_class_target_for_local_club() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    let local_buyer = BuyerContext {
        buyer_rep_score: 0.20,
        buyer_world_rep: 1500,
        buyer_league_reputation: 2000,
        buyer_total_wages: 200_000,
        buyer_wage_budget: 600_000,
        plan_total_budget: 300_000.0,
        max_recommend_value: 600_000.0,
        buyer_best_in_group: 60,
        has_open_request: true, // even with explicit demand, world-class is out of reach
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    let world_class = ListedTargetView {
        nationality_country_id: 0,
        ability: 175,
        estimated_potential: 180,
        age: 28,
        estimated_value: 200_000.0, // dirt-cheap to bypass fee gate
        position_group: PlayerFieldPositionGroup::Forward,
        is_listed: true,
        is_transfer_requested: false,
        is_unhappy: false,
        world_reputation: 9500,
        current_reputation: 9000,
        ambition: 0.7,
        parent_club_score: 0.85,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    let v = ListedTargetScreen::evaluate(&world_class, &local_buyer);
    // Tier window or reputation gap blocks well before scoring.
    match v {
        ListedTargetVerdict::Reject(
            ListedRejectReason::OutOfTierWindow | ListedRejectReason::ReputationGapTooLarge,
        ) => {}
        other => panic!("expected window / rep-gap reject, got {:?}", other),
    }
}

#[test]
fn sweep_rejects_when_no_need_and_no_request() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    // Continental club, perfectly fine in this group, no aging
    // starter, no open request — sweep must NOT add filler.
    let buyer = BuyerContext {
        buyer_rep_score: 0.72,
        buyer_world_rep: 5500,
        buyer_league_reputation: 5500,
        buyer_total_wages: 20_000_000,
        buyer_wage_budget: 50_000_000,
        plan_total_budget: 25_000_000.0,
        max_recommend_value: 50_000_000.0,
        buyer_best_in_group: 135, // above tier baseline
        has_open_request: false,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    let modest_listed = ListedTargetView {
        nationality_country_id: 0,
        ability: 128,
        estimated_potential: 130,
        age: 26,
        estimated_value: 8_000_000.0,
        position_group: PlayerFieldPositionGroup::Midfielder,
        is_listed: true,
        is_transfer_requested: false,
        is_unhappy: false,
        world_reputation: 4500,
        current_reputation: 4000,
        ambition: 0.5,
        parent_club_score: 0.55,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    let v = ListedTargetScreen::evaluate(&modest_listed, &buyer);
    assert_eq!(
        v,
        ListedTargetVerdict::Reject(ListedRejectReason::NoSquadNeed),
        "club with no need must not add filler — got {:?}",
        v
    );
}

#[test]
fn sweep_rejects_player_without_listing_status() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    let buyer = BuyerContext {
        buyer_rep_score: 0.72,
        buyer_world_rep: 5500,
        buyer_league_reputation: 5500,
        buyer_total_wages: 20_000_000,
        buyer_wage_budget: 50_000_000,
        plan_total_budget: 25_000_000.0,
        max_recommend_value: 50_000_000.0,
        buyer_best_in_group: 105,
        has_open_request: true,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    let happy_player = ListedTargetView {
        nationality_country_id: 0,
        ability: 130,
        estimated_potential: 135,
        age: 25,
        estimated_value: 8_000_000.0,
        position_group: PlayerFieldPositionGroup::Forward,
        is_listed: false,
        is_transfer_requested: false,
        is_unhappy: false,
        world_reputation: 5000,
        current_reputation: 4500,
        ambition: 0.5,
        parent_club_score: 0.50,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    // The sweep is the listed-star path — players without any
    // public listing flag aren't routed through it.
    assert_eq!(
        ListedTargetScreen::evaluate(&happy_player, &buyer),
        ListedTargetVerdict::Reject(ListedRejectReason::NotListed)
    );
}

#[test]
fn depth_requirement_scales_with_formation_footprint() {
    // The bench-depth helper is pure and used by the detector.
    // Pin its calibration so behaviour stays stable.
    let t442 = GroupFx::t442_positions();
    assert_eq!(
        GroupNeedScan::depth_requirement(t442, PlayerFieldPositionGroup::Goalkeeper),
        2,
        "GK depth is fixed at 2 regardless of formation"
    );
    // 4-4-2 has 4 defenders → 4+2 = 6
    assert_eq!(
        GroupNeedScan::depth_requirement(t442, PlayerFieldPositionGroup::Defender),
        6
    );
    // 4-4-2 has 4 mids → 4+1 = 5
    assert_eq!(
        GroupNeedScan::depth_requirement(t442, PlayerFieldPositionGroup::Midfielder),
        5
    );
    // 4-4-2 has 2 forwards → 2+1 = 3
    assert_eq!(
        GroupNeedScan::depth_requirement(t442, PlayerFieldPositionGroup::Forward),
        3
    );
}

#[test]
fn stale_market_opportunity_unlocks_signing_without_open_request() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    // Continental club, well-stocked at the position (best above the
    // tier baseline → not weak), no open request, no aging starter —
    // so there is no conventional squad need. A FRESH listing here is
    // correctly rejected as filler.
    let buyer = BuyerContext {
        buyer_rep_score: 0.72,
        buyer_world_rep: 5800,
        buyer_league_reputation: 5500,
        buyer_total_wages: 20_000_000,
        buyer_wage_budget: 60_000_000,
        plan_total_budget: 40_000_000.0,
        max_recommend_value: 0.0,
        buyer_best_in_group: 135,
        has_open_request: false,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    let mut player = ListedTargetView {
        nationality_country_id: 0,
        ability: 130,
        estimated_potential: 138,
        age: 25,
        estimated_value: 8_000_000.0,
        position_group: PlayerFieldPositionGroup::Forward,
        is_listed: false,
        is_transfer_requested: true,
        is_unhappy: true,
        world_reputation: 5200,
        current_reputation: 5000,
        ambition: 0.6,
        parent_club_score: 0.40,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    // Fresh: no need + only a sideways move → rejected. The
    // opportunity route is gated on staleness, so a brand-new listing
    // never bypasses the need check (existing behaviour preserved).
    assert_eq!(
        ListedTargetScreen::evaluate(&player, &buyer),
        ListedTargetVerdict::Reject(ListedRejectReason::NoSquadNeed)
    );

    // After months on the market, barely featuring, with dry scans
    // behind him, the same affordable, in-tier player becomes a
    // genuine depth/resale opportunity even without an open request.
    player.days_available = 120;
    player.low_usage = true;
    player.failed_scans = 4;
    assert!(
        matches!(
            ListedTargetScreen::evaluate(&player, &buyer),
            ListedTargetVerdict::Accept(_)
        ),
        "a stale, affordable, in-tier available player must become a market opportunity"
    );
}

#[test]
fn staleness_softening_makes_borderline_fee_reachable() {
    use crate::PlayerFieldPositionGroup;
    use crate::transfers::pipeline::advice::{
        BuyerContext, ListedRejectReason, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
    };

    // Open request so the need / upgrade gates pass — we want to
    // isolate the FEE gate and its staleness-driven softening.
    let buyer = BuyerContext {
        buyer_rep_score: 0.72,
        buyer_world_rep: 5800,
        buyer_league_reputation: 5500,
        buyer_total_wages: 10_000_000,
        buyer_wage_budget: 100_000_000,
        plan_total_budget: 10_000_000.0, // reach = 14M
        max_recommend_value: 0.0,
        buyer_best_in_group: 110,
        has_open_request: true,
        has_aging_starter: false,
        form_discovery_mode: false,
        fit: SquadFitSnapshot::disabled(),
    };

    let mut player = ListedTargetView {
        nationality_country_id: 0,
        ability: 128,
        estimated_potential: 132,
        age: 27,
        estimated_value: 17_000_000.0, // just beyond the fresh reach
        position_group: PlayerFieldPositionGroup::Forward,
        is_listed: true,
        is_transfer_requested: false,
        is_unhappy: false,
        world_reputation: 4800,
        current_reputation: 4500,
        ambition: 0.5,
        parent_club_score: 0.55,
        parent_club_in_debt: false,
        days_available: 5,
        contract_months_remaining: 24,
        low_usage: false,
        recent_interest_count: 0,
        failed_scans: 0,
        last_block: None,
        is_loan_listed: false,
        breakout_score: 0.0,
    };

    // Fresh: a 17M asking sits above the ~14M reach → unaffordable.
    assert_eq!(
        ListedTargetScreen::evaluate(&player, &buyer),
        ListedTargetVerdict::Reject(ListedRejectReason::UnaffordableFee)
    );

    // A year unsold with a dozen dry scans: the seller has quietly
    // dropped the asking enough to bring the deal into reach — but the
    // softening stays bounded, so it never becomes a giveaway.
    player.days_available = 365;
    player.failed_scans = 12;
    assert!(
        matches!(
            ListedTargetScreen::evaluate(&player, &buyer),
            ListedTargetVerdict::Accept(_)
        ),
        "asking-price softening after a long market failure must bring a borderline fee into reach"
    );
}
