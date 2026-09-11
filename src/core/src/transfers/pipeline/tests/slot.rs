//! Moved verbatim out of `helpers.rs` — see that file's `mod slot_need_tests`.

use crate::club::team::squad::SquadAssetClass;
use crate::transfers::pipeline::processor::SquadPlayerInfo;
use crate::transfers::squad::SquadReviewPass;
use crate::transfers::squad::bands::TierBands;
use crate::transfers::squad::{GroupNeed, GroupNeedScan, NeedKind};
use crate::{
    MatchTacticType, PlayerFieldPositionGroup, PlayerPositionType, RoleFamiliarity,
    TACTICS_POSITIONS,
};
use std::collections::HashMap;

struct SlotFx;

impl SlotFx {
    /// Continental tier — the band the calibration tests already use.
    const REP_SCORE: f32 = 0.725;

    fn formation(tactic: MatchTacticType) -> &'static [PlayerPositionType; 11] {
        let (_, positions) = TACTICS_POSITIONS
            .iter()
            .find(|(t, _)| *t == tactic)
            .expect("tactic must exist");
        positions
    }

    fn tolerance() -> i16 {
        TierBands::tier_quality_tolerance_score(Self::REP_SCORE)
    }

    fn baseline(group: PlayerFieldPositionGroup) -> u8 {
        TierBands::tier_starter_ca_score(Self::REP_SCORE, group)
    }

    fn player(id: u32, roles: &[PlayerPositionType], ca: u8) -> SquadPlayerInfo {
        let mut levels: HashMap<PlayerPositionType, u8> = HashMap::new();
        for role in roles {
            levels.insert(*role, 20);
        }
        SquadPlayerInfo {
            player_id: id,
            primary_position: roles[0],
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

    /// Production's slot assignment: best remaining man per shirt, graded
    /// on the strongest role he holds inside that shirt's group.
    fn coverage(
        squad: &[SquadPlayerInfo],
        formation: &[PlayerPositionType; 11],
    ) -> Vec<(PlayerPositionType, Option<u32>, u8)> {
        let mut used: Vec<u32> = Vec::new();
        let mut out = Vec::new();
        for &slot in formation.iter() {
            let group = slot.position_group();
            let pick = squad
                .iter()
                .filter(|p| !used.contains(&p.player_id))
                .filter_map(|p| {
                    let in_group = p
                        .position_levels
                        .iter()
                        .filter(|(pos, _)| pos.position_group() == group)
                        .map(|(_, level)| *level)
                        .max()
                        .unwrap_or(0);
                    if in_group == 0 {
                        return None;
                    }
                    let effective = RoleFamiliarity::effective_ability(p.current_ability, in_group);
                    Some((p.player_id, effective))
                })
                .max_by_key(|&(_, effective)| effective);
            match pick {
                Some((id, quality)) => {
                    used.push(id);
                    out.push((slot, Some(id), quality));
                }
                None => out.push((slot, None, 0)),
            }
        }
        out
    }

    /// A squad comfortably at tier standard everywhere except the forward
    /// line, which carries one very good man and two who are nowhere near
    /// it — the shape a `max` over the group cannot see.
    fn lopsided_front_two() -> Vec<SquadPlayerInfo> {
        let at_tier = Self::baseline(PlayerFieldPositionGroup::Midfielder) + 40;
        let mut squad = vec![
            Self::player(1, &[PlayerPositionType::Goalkeeper], at_tier),
            Self::player(2, &[PlayerPositionType::Goalkeeper], at_tier),
        ];
        for i in 0..6u32 {
            squad.push(Self::player(
                10 + i,
                &[PlayerPositionType::DefenderCenter],
                at_tier,
            ));
        }
        for i in 0..6u32 {
            squad.push(Self::player(
                20 + i,
                &[PlayerPositionType::MidfielderCenter],
                at_tier,
            ));
        }
        squad.push(Self::player(30, &[PlayerPositionType::Striker], at_tier));
        squad.push(Self::player(31, &[PlayerPositionType::Striker], 90));
        squad.push(Self::player(32, &[PlayerPositionType::Striker], 88));
        squad
    }

    fn needs(squad: &[SquadPlayerInfo], tactic: MatchTacticType) -> Vec<GroupNeed> {
        let formation = Self::formation(tactic);
        GroupNeedScan::needs(
            squad,
            &Self::coverage(squad, formation),
            formation,
            Self::REP_SCORE,
            Self::tolerance(),
        )
    }
}

/// The bug this replaced: a group's quality was `max(CA)` over everyone
/// wearing its label, so one outstanding forward answered the question for
/// a whole front two and the shirt nobody could fill stayed invisible.
#[test]
fn one_star_forward_no_longer_hides_the_other_shirt() {
    let squad = SlotFx::lopsided_front_two();

    let forward_best = squad
        .iter()
        .filter(|p| p.primary_position.position_group() == PlayerFieldPositionGroup::Forward)
        .map(|p| p.current_ability)
        .max()
        .unwrap();
    assert!(
        (forward_best as i16)
            >= SlotFx::baseline(PlayerFieldPositionGroup::Forward) as i16 - SlotFx::tolerance(),
        "precondition: the OLD max-over-the-group rule saw no need here"
    );

    let needs = SlotFx::needs(&squad, MatchTacticType::T442);
    let forward: Vec<&GroupNeed> = needs
        .iter()
        .filter(|n| n.group == PlayerFieldPositionGroup::Forward)
        .collect();
    assert_eq!(forward.len(), 1, "expected one forward need, got {needs:?}");
    assert_eq!(forward[0].kind, NeedKind::QualityUpgrade);
    assert!(
        forward[0].representative_pos.is_forward(),
        "the request must name the shirt that is short"
    );
}

/// …and it does not simply fire everywhere: a front two both at tier
/// standard is a front two the club is entitled to be happy with.
#[test]
fn a_front_line_at_tier_standard_raises_nothing() {
    let mut squad = SlotFx::lopsided_front_two();
    let at_tier = SlotFx::baseline(PlayerFieldPositionGroup::Forward) + 40;
    for player in squad.iter_mut() {
        if player.primary_position.is_forward() {
            player.current_ability = at_tier;
        }
    }

    let needs = SlotFx::needs(&squad, MatchTacticType::T442);
    assert!(
        !needs
            .iter()
            .any(|n| n.group == PlayerFieldPositionGroup::Forward),
        "no forward need expected, got {needs:?}"
    );
}

/// A man filling a neighbouring shirt inside his own group is still the
/// player the club has. Reading him as out of position would have every
/// side in the world shopping to fix its own formation.
#[test]
fn covering_a_neighbouring_shirt_is_not_a_recruitment_problem() {
    // Six central midfielders in a 4-4-2, which asks for two wide ones.
    let at_tier = SlotFx::baseline(PlayerFieldPositionGroup::Midfielder) + 40;
    let mut squad = SlotFx::lopsided_front_two();
    squad.retain(|p| !p.primary_position.is_midfielder());
    for i in 0..6u32 {
        squad.push(SlotFx::player(
            40 + i,
            &[PlayerPositionType::MidfielderCenter],
            at_tier,
        ));
    }

    let needs = SlotFx::needs(&squad, MatchTacticType::T442);
    assert!(
        !needs
            .iter()
            .any(|n| n.group == PlayerFieldPositionGroup::Midfielder),
        "central midfielders covering the flanks are not a shortage, got {needs:?}"
    );
}

/// A nominal second position must not paper over a missing role: a
/// midfielder who lists centre-forward at an unconvincing familiarity is
/// not the centre-forward the club is short of.
#[test]
fn a_nominal_forward_does_not_fill_the_forward_shirt() {
    let at_tier = SlotFx::baseline(PlayerFieldPositionGroup::Midfielder) + 40;
    let mut squad = SlotFx::lopsided_front_two();
    squad.retain(|p| !p.primary_position.is_forward());
    // Two midfielders who "can play up front" — on paper. A shade below
    // their pure-midfield team-mates so the four midfield shirts go to
    // those, leaving these two as the only bodies for the front line:
    // otherwise the side has literally nobody up front and the need is
    // raised as an outright formation gap instead.
    for i in 0..2u32 {
        let mut dabbler =
            SlotFx::player(50 + i, &[PlayerPositionType::MidfielderCenter], at_tier - 2);
        dabbler
            .position_levels
            .insert(PlayerPositionType::Striker, 8);
        squad.push(dabbler);
    }

    let needs = SlotFx::needs(&squad, MatchTacticType::T442);
    assert!(
        needs
            .iter()
            .any(|n| n.group == PlayerFieldPositionGroup::Forward
                && n.kind == NeedKind::QualityUpgrade),
        "a squad with no real forward must be shopping for one, got {needs:?}"
    );
}

/// Forward used to be the one group a loan-out could strip back to the
/// bare formation count, on the mistaken grounds that the wide forwards
/// were counted in it — they group under Midfielder. The floors are now
/// consistent for every group.
#[test]
fn every_group_keeps_a_rotation_cushion_over_its_formation_footprint() {
    for tactic in [
        MatchTacticType::T442,
        MatchTacticType::T433,
        MatchTacticType::T4231,
        MatchTacticType::T451,
    ] {
        let formation = SlotFx::formation(tactic);
        for group in PlayerFieldPositionGroup::ALL {
            let slots = formation
                .iter()
                .filter(|p| p.position_group() == group)
                .count();
            let floor = SquadReviewPass::group_min_needed(group, formation);
            assert!(
                floor > slots,
                "{tactic:?}/{group:?}: floor {floor} leaves no cover for {slots} shirts"
            );
            assert!(
                floor <= GroupNeedScan::depth_requirement(formation, group),
                "{tactic:?}/{group:?}: the loan-out floor must not exceed the depth target"
            );
        }
    }
}

/// The specific regression: a lone-striker shape must still keep a deputy.
#[test]
fn a_lone_striker_shape_keeps_a_deputy_centre_forward() {
    let formation = SlotFx::formation(MatchTacticType::T4231);
    assert_eq!(
        SquadReviewPass::group_min_needed(PlayerFieldPositionGroup::Forward, formation),
        2,
        "one striker on the teamsheet still means two on the roster"
    );
}
