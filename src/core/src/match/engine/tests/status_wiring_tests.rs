//! **Every player state occurs in real matches or is retired.** The states
//! that were statically reachable and never entered, each pinned at the
//! situation that should send him there.

#![cfg(test)]

use super::goal_celebration_tests::{squad, squad_in};
use crate::MatchTacticType;
use crate::PlayerPositionType;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::DefensiveRest;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::engine::wide::Flank;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::MidfieldSupport;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::defenders::states::resting::DefenderRestingState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

fn pitch_of(home: MatchSquad, away: MatchSquad) -> (MatchField, MatchContext) {
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    field.ball.kickoff_taker = None;
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    (field, context)
}

fn pitch() -> (MatchField, MatchContext) {
    pitch_of(squad(1, 100), squad(2, 200))
}

fn in_role(field: &MatchField, wanted: fn(&PlayerState) -> bool) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.team_id == 1 && wanted(&p.state))
        .expect("a home player in that role")
        .id
}

/// Everybody a long way off, the ball with `owner` at the far end.
fn quiet_pitch(field: &mut MatchField, owner: u32) {
    for (index, player) in field.players.iter_mut().enumerate() {
        player.position = Vector3::new(780.0, 20.0 + index as f32 * 22.0, 0.0);
    }
    field.ball.current_owner = Some(owner);
    field.ball.position = field.get_player(owner).expect("owner").position;
}

// ── A midfielder with nothing to run for walks ──────────────────────────

#[test]
fn a_fresh_midfielder_with_nothing_to_run_for_walks() {
    let (mut field, context) = pitch();
    let owner = in_role(&field, |s| matches!(s, PlayerState::Forward(_)));
    let id = in_role(&field, |s| matches!(s, PlayerState::Midfielder(_)));
    quiet_pitch(&mut field, owner);
    let at = field.get_player(id).expect("him").start_position;
    let player = field.get_player_mut(id).expect("him");
    player.position = at;
    player.player_attributes.condition = 9500;

    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 40,
        player: field.get_player(id).expect("him"),
        context: &context,
        tick_context: &tick,
    };
    assert!(MidfieldSupport::walks(&ctx));
    assert_eq!(
        MidfieldSupport::wanted(&ctx),
        Some(MidfielderState::Walking),
        "the off-ball decision reads the walk gate, not only `Standing`",
    );
}

#[test]
fn a_midfielder_near_the_ball_does_not_walk() {
    let (mut field, context) = pitch();
    let owner = in_role(&field, |s| matches!(s, PlayerState::Forward(_)));
    let id = in_role(&field, |s| matches!(s, PlayerState::Midfielder(_)));
    quiet_pitch(&mut field, owner);
    let at = field.get_player(id).expect("him").start_position;
    field.get_player_mut(id).expect("him").position = at;
    field.ball.position = at + Vector3::new(120.0, 0.0, 0.0);
    field.get_player_mut(owner).expect("owner").position = field.ball.position;

    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 40,
        player: field.get_player(id).expect("him"),
        context: &context,
        tick_context: &tick,
    };
    assert!(!MidfieldSupport::walks(&ctx));
}

// ── A tired defender eases off when he can ──────────────────────────────

fn tired_defender(field: &mut MatchField) -> u32 {
    let id = in_role(field, |s| matches!(s, PlayerState::Defender(_)));
    let player = field.get_player_mut(id).expect("him");
    player.position = Vector3::new(60.0, 270.0, 0.0);
    player.player_attributes.condition = 5500;
    id
}

#[test]
fn a_tired_defender_eases_off_when_he_can() {
    let (mut field, context) = pitch();
    let owner = in_role(&field, |s| matches!(s, PlayerState::Forward(_)));
    quiet_pitch(&mut field, owner);
    let id = tired_defender(&mut field);
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 80,
        player: field.get_player(id).expect("him"),
        context: &context,
        tick_context: &tick,
    };
    assert!(DefensiveRest::may_stand_down(&ctx));
    assert!(
        !DefensiveRest::is_wanted(&ctx),
        "entry sits inside the exit"
    );
}

#[test]
fn a_fresh_defender_keeps_going() {
    let (mut field, context) = pitch();
    let owner = in_role(&field, |s| matches!(s, PlayerState::Forward(_)));
    quiet_pitch(&mut field, owner);
    let id = tired_defender(&mut field);
    field
        .get_player_mut(id)
        .expect("him")
        .player_attributes
        .condition = 9000;
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 80,
        player: field.get_player(id).expect("him"),
        context: &context,
        tick_context: &tick,
    };
    assert!(!DefensiveRest::may_stand_down(&ctx));
}

fn resting_decision(field: &MatchField, context: &MatchContext, id: u32) -> Option<PlayerState> {
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 80,
        player: field.get_player(id).expect("him"),
        context,
        tick_context: &tick,
    };
    DefenderRestingState::default()
        .process(&ctx)
        .and_then(|change| change.state)
}

#[test]
fn a_resting_defender_comes_back_for_the_ball() {
    let (mut field, context) = pitch();
    let owner = in_role(&field, |s| matches!(s, PlayerState::Forward(_)));
    quiet_pitch(&mut field, owner);
    let id = tired_defender(&mut field);
    field.get_player_mut(id).expect("him").transition_to(
        PlayerState::Defender(DefenderState::Resting),
        TransitionSource::Reset,
    );
    assert_eq!(
        resting_decision(&field, &context, id),
        None,
        "nothing wants him yet"
    );

    field.ball.position = Vector3::new(150.0, 270.0, 0.0);
    assert_eq!(
        resting_decision(&field, &context, id),
        Some(PlayerState::Defender(DefenderState::HoldingLine)),
    );
}

#[test]
fn a_resting_defender_comes_back_for_a_man() {
    let (mut field, context) = pitch();
    let owner = in_role(&field, |s| matches!(s, PlayerState::Forward(_)));
    quiet_pitch(&mut field, owner);
    let id = tired_defender(&mut field);
    let threat = field
        .players
        .iter()
        .find(|p| p.team_id == 2)
        .expect("an opponent")
        .id;
    field.get_player_mut(threat).expect("him").position = Vector3::new(100.0, 270.0, 0.0);
    field.get_player_mut(id).expect("him").transition_to(
        PlayerState::Defender(DefenderState::Resting),
        TransitionSource::Reset,
    );
    assert_eq!(
        resting_decision(&field, &context, id),
        Some(PlayerState::Defender(DefenderState::HoldingLine)),
    );
}

// ── A front three holds the width ───────────────────────────────────────

const T433: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::ForwardRight,
];

/// Who the plan gives the two touchlines to, with the home side on the ball.
fn holders(field: &mut MatchField, context: &mut MatchContext) -> [PlayerPositionType; 2] {
    // Everybody equally able, so the slot decides.
    for player in field.players.iter_mut() {
        player.skills.technical.crossing = 12.0;
        player.skills.technical.technique = 12.0;
        player.skills.technical.dribbling = 12.0;
    }
    // The keeper has it: a man on the ball is not given a touchline.
    let carrier = in_role(field, |s| matches!(s, PlayerState::Goalkeeper(_)));
    field.ball.current_owner = Some(carrier);
    field.ball.position = field.get_player(carrier).expect("carrier").position;
    let possession = TeamPossession::read(field);
    FootballEngine::<840, 545>::refresh_tactical_states(field, context, possession, 1);
    Flank::ALL.map(|flank| {
        let id = context
            .attack_home
            .wide
            .holder(flank)
            .expect("a holder on each flank");
        field
            .get_player(id)
            .expect("him")
            .tactical_position
            .current_position
    })
}

#[test]
fn a_front_threes_outside_men_hold_the_touchlines() {
    let (mut field, mut context) = pitch_of(
        squad_in(1, 100, MatchTacticType::T433, &T433),
        squad_in(2, 200, MatchTacticType::T433, &T433),
    );
    for holder in holders(&mut field, &mut context) {
        assert!(
            matches!(
                holder,
                PlayerPositionType::ForwardLeft | PlayerPositionType::ForwardRight
            ),
            "{holder:?} holds a touchline in a 4-3-3"
        );
    }
}

#[test]
fn a_strike_pair_does_not_take_the_touchlines_off_the_wide_midfielders() {
    let (mut field, mut context) = pitch();
    for holder in holders(&mut field, &mut context) {
        assert!(
            matches!(
                holder,
                PlayerPositionType::MidfielderLeft | PlayerPositionType::MidfielderRight
            ),
            "{holder:?} holds a touchline in a 4-4-2"
        );
    }
}
