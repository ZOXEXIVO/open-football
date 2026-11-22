//! A coach changes the plan at a moment, each side on its own clock, and a
//! change of formation lays the side out on the new one, played at the
//! familiarity the side's record gives it; partnerships, a substitute's
//! among them, come from the record too.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::r#match::defenders::states::shared::LineStep;
use crate::r#match::engine::flow::arena::formation_variant::{FormationLine, FormationVariant};
use crate::r#match::engine::substitutions::Substitutions;
use crate::r#match::engine::teamplay::plans::line_step::LineStepCall;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::{
    ChemistryMap, CoachCard, FootballEngine, MatchContext, MatchField, MatchPlayer, Outlook,
    PlanChange, PlayerSide, ReadCounters, Situation, SquadFit, TacticalFamiliarity,
};
use crate::{FormationFamiliarity, MatchTacticType, PlayerPositionType, RoleLevels, Tactics};

const HOME: u32 = 1;

fn roles(field: &MatchField, team_id: u32) -> Vec<PlayerPositionType> {
    let mut roles: Vec<PlayerPositionType> = field
        .players
        .iter()
        .filter(|p| p.team_id == team_id && !p.off_pitch)
        .map(|p| p.tactical_position.current_position)
        .collect();
    roles.sort_by_key(|r| *r as u8);
    roles
}

#[test]
fn a_back_four_to_a_back_five_puts_a_man_in_every_new_place() {
    let (mut field, _) = kickoff();
    let back_five = Tactics::new(MatchTacticType::T352);
    let mut wanted = back_five.positions().to_vec();
    wanted.sort_by_key(|r| *r as u8);

    field.change_formation(HOME, back_five, FormationLine::Attack);

    assert_eq!(roles(&field, HOME), wanted, "each new place taken once");
    for gone in [
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderRight,
    ] {
        assert!(
            !roles(&field, HOME).contains(&gone),
            "{gone:?} is still held"
        );
    }
    let side = field.side_of(HOME);
    for p in field
        .players
        .iter()
        .filter(|p| p.team_id == HOME && !p.off_pitch)
    {
        let slot = FormationVariant::slot(p.tactical_position.current_position, side);
        assert_eq!(
            Some(p.start_position),
            slot,
            "{:?}",
            p.tactical_position.current_position
        );
    }
    let tactics = if side == PlayerSide::Left {
        &field.left_team_tactics
    } else {
        &field.right_team_tactics
    };
    assert_eq!(tactics.tactic_type, MatchTacticType::T352);
}

/// Level, with each coach reading his side on top: making chances and
/// conceding none.
fn both_on_top(context: &mut MatchContext, minute: f32) {
    let at = Situation {
        minutes_left: 90.0 - minute,
        ..Situation::default()
    };
    let read = ReadCounters {
        xg_for: 0.08 * minute,
        ticks: minute as u32 * 6_000,
        ..ReadCounters::default()
    };
    context.coach_home.observe(at, minute, read);
    context.coach_away.observe(at, minute, read);
}

#[test]
fn the_two_sides_change_their_plans_on_their_own_clocks() {
    let (mut field, mut context) = kickoff();
    for coach in [&mut context.coach_home, &mut context.coach_away] {
        coach.on_kickoff(
            CoachCard::neutral(),
            Outlook::default(),
            &SquadFit::default(),
            None,
        );
    }
    for minute in 50..=62 {
        both_on_top(&mut context, minute as f32);
        context.total_match_time = minute * 60_000;
        if minute == 60 {
            context.coach_home.on_forced_change();
        }
        if minute == 62 {
            context.coach_away.on_forced_change();
        }
        FootballEngine::<840, 545>::decide_plans(&mut field, &mut context);
    }
    let minutes = |changes: &[PlanChange]| changes.iter().map(|c| c.minute).collect::<Vec<u8>>();
    assert_eq!(minutes(&context.coach_home.record.changes), vec![60]);
    assert_eq!(minutes(&context.coach_away.record.changes), vec![62]);
}

#[test]
fn a_shape_is_open_only_when_every_man_can_play_his_place_in_it() {
    let (mut field, _) = kickoff();
    for p in field.players.iter_mut().filter(|p| p.team_id == HOME) {
        p.role_levels = RoleLevels::natural(p.tactical_position.current_position);
    }
    let own = Tactics::new(MatchTacticType::T442);
    assert!(FormationVariant::can_fill(
        &field,
        HOME,
        own.positions(),
        FormationLine::Attack
    ));

    let keeper_up_front = field
        .players
        .iter_mut()
        .find(|p| p.team_id == HOME && p.tactical_position.current_position.is_forward())
        .unwrap();
    keeper_up_front.role_levels = RoleLevels::natural(PlayerPositionType::Goalkeeper);
    assert!(!FormationVariant::can_fill(
        &field,
        HOME,
        own.positions(),
        FormationLine::Attack
    ));
}

fn refresh(field: &mut MatchField, context: &mut MatchContext) {
    context.invalidate_skill_aggregates();
    let possession = TeamPossession::read(field);
    FootballEngine::<840, 545>::refresh_tactical_states(field, context, possession, 1);
}

#[test]
fn a_shape_the_side_rarely_plays_is_played_at_a_lower_familiarity_and_its_line_steps_later() {
    let (mut field, mut context) = kickoff();
    context.familiarity_home = FormationFamiliarity::from_starts(
        std::iter::repeat_n((MatchTacticType::T442, 1.0), 9).chain([(MatchTacticType::T352, 1.0)]),
        10,
    );
    refresh(&mut field, &mut context);
    let before = context.tactical_familiarity_home;

    field.change_formation(
        HOME,
        Tactics::new(MatchTacticType::T352),
        FormationLine::Attack,
    );
    refresh(&mut field, &mut context);
    let after = context.tactical_familiarity_home;

    assert!(
        after.score < before.score,
        "{} against {}",
        after.score,
        before.score
    );
    let reaction = |familiarity: TacticalFamiliarity| {
        LineStep::reaction_ticks(12.0, familiarity.offside_trap_risk(), 0.5)
    };
    assert!(reaction(after) > reaction(before));
    assert!(
        LineStepCall::organisation(12.0, after.score, 0.5)
            < LineStepCall::organisation(12.0, before.score, 0.5)
    );
}

/// The kickoff chemistry of two home midfielders, with each one's record.
fn pair(first: Option<Vec<u32>>, second: Option<Vec<u32>>) -> f32 {
    let (field, _) = kickoff();
    let mut mids = field
        .players
        .iter()
        .filter(|p| p.team_id == HOME && p.tactical_position.current_position.is_midfielder());
    let (mut a, mut b) = (mids.next().unwrap().clone(), mids.next().unwrap().clone());
    a.started_with = first;
    b.started_with = second;
    let mut chemistry = ChemistryMap::default();
    chemistry.seed(&[&a, &b], 545.0);
    chemistry.get(a.id, b.id).unwrap()
}

#[test]
fn a_new_signing_s_partnerships_count_as_new() {
    let (field, _) = kickoff();
    let ids: Vec<u32> = field.players.iter().map(|p| p.id).collect();
    let settled = pair(Some(ids.clone()), Some(ids.clone()));
    assert!(pair(Some(Vec::new()), Some(ids)) < settled);
    assert_eq!(pair(None, None), settled, "no record reads as settled");
}

/// Take off a spent home midfielder with every man on the home bench
/// carrying `record`: the man who comes on, and his chemistry with each
/// team-mate on the pitch.
fn joined(record: Option<Vec<u32>>) -> (u32, Vec<(u32, Option<f32>)>) {
    let (mut field, mut context) = kickoff();
    for sub in field.substitutes.iter_mut().filter(|p| p.team_id == HOME) {
        sub.started_with = record.clone();
    }
    let spent = field
        .players
        .iter_mut()
        .find(|p| {
            p.team_id == HOME
                && p.tactical_position.current_position == PlayerPositionType::MidfielderLeft
        })
        .unwrap();
    spent.player_attributes.condition = 1200;
    let spent = spent.id;
    let today = context.today;
    Substitutions::process(&mut field, &mut context, 3, today);
    let came_on = context
        .substitution_break
        .as_ref()
        .and_then(|window| {
            window
                .changes()
                .iter()
                .find(|c| c.player_out_id == spent)
                .map(|c| c.player_in_id)
        })
        .expect("a man came on for him");
    let mates: Vec<&MatchPlayer> = field
        .players
        .iter()
        .filter(|p| p.team_id == HOME && !p.off_pitch && p.id != came_on)
        .collect();
    let pairs = mates
        .iter()
        .map(|m| (m.id, context.chemistry.get(came_on, m.id)))
        .collect();
    (came_on, pairs)
}

#[test]
fn a_substitute_joins_the_partnerships_on_entry_by_his_record() {
    let (_, new) = joined(Some(Vec::new()));
    let (_, known) = joined(None);
    assert!(
        new.iter().all(|(_, chem)| chem.is_some()),
        "every pair seeded: {new:?}"
    );
    for ((mate, new), (_, known)) in new.iter().zip(&known) {
        assert!(new.unwrap() < known.unwrap(), "with {mate}");
    }
}
