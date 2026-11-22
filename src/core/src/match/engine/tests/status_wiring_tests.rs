//! **Every player state occurs in real matches or is retired.** The states
//! that were statically reachable and never entered, each pinned at the
//! situation that should send him there.

#![cfg(test)]

use super::goal_celebration_tests::{squad, squad_in};
use crate::MatchTacticType;
use crate::PlayerPositionType;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::DefenderRead;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::engine::wide::Flank;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::MidfielderRead;
use crate::r#match::player::state::PlayerState;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, StateProcessingContext,
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

// ── A tired midfielder eases off in his place ───────────────────────────

/// The away side on the ball 40 m from a home central midfielder standing on
/// his place: the phase he reads at `condition`.
fn phase_of_a_midfielder_at(condition: i16) -> MidfielderState {
    let (mut field, context) = pitch();
    let id = field
        .players
        .iter()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position.is_central_midfielder())
        .expect("a home central midfielder")
        .id;
    let place = field.get_player(id).expect("him").start_position;
    for (index, player) in field.players.iter_mut().enumerate() {
        player.position = Vector3::new(820.0, 20.0 + index as f32 * 22.0, 0.0);
        player.velocity = Vector3::zeros();
    }
    field.get_player_mut(id).expect("him").position = place;
    let carrier = field
        .players
        .iter()
        .find(|p| p.team_id == 2 && p.tactical_position.current_position.is_forward())
        .expect("an away forward")
        .id;
    let ball_at = place + Vector3::new(0.0, if place.y < 272.0 { 320.0 } else { -320.0 }, 0.0);
    field.get_player_mut(carrier).expect("him").position = ball_at;
    field.ball.position = ball_at;
    field.ball.current_owner = Some(carrier);
    field
        .get_player_mut(id)
        .expect("him")
        .player_attributes
        .condition = condition;
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 40,
        player: field.get_player(id).expect("him"),
        context: &context,
        tick_context: &tick,
    };
    MidfielderRead::phase(&ctx)
}

#[test]
fn a_tired_midfielder_is_relaxed_where_a_fresh_one_is_alert() {
    assert_eq!(
        phase_of_a_midfielder_at(9000),
        MidfielderState::HoldingShape
    );
    assert_eq!(phase_of_a_midfielder_at(5500), MidfielderState::Walking);
}

// ── A tired defender eases off when he can ──────────────────────────────

/// The away side on the ball 50 m from the home goal, the home back line
/// on its slots: the phase a home centre-back at `condition` reads.
fn phase_of_a_defender_at(condition: i16) -> DefenderState {
    let (mut field, context) = pitch();
    let home_left = field.side_of(1) == crate::r#match::PlayerSide::Left;
    let x = |from_home_goal: f32| {
        if home_left {
            from_home_goal
        } else {
            840.0 - from_home_goal
        }
    };
    for (index, player) in field.players.iter_mut().enumerate() {
        player.position = Vector3::new(x(760.0), 20.0 + index as f32 * 22.0, 0.0);
        player.velocity = Vector3::zeros();
    }
    for player in field
        .players
        .iter_mut()
        .filter(|p| p.team_id == 1 && p.tactical_position.current_position.is_defender())
    {
        player.position = Vector3::new(x(120.0), player.start_position.y, 0.0);
    }
    let carrier = field
        .players
        .iter()
        .find(|p| p.team_id == 2 && p.tactical_position.current_position.is_forward())
        .expect("an away forward")
        .id;
    field.get_player_mut(carrier).expect("him").position = Vector3::new(x(400.0), 272.0, 0.0);
    field.ball.position = Vector3::new(x(400.0), 272.0, 0.0);
    field.ball.current_owner = Some(carrier);
    let id = field
        .players
        .iter()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position.is_central_defender())
        .expect("a home centre-back")
        .id;
    field
        .get_player_mut(id)
        .expect("him")
        .player_attributes
        .condition = condition;
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 40,
        player: field.get_player(id).expect("him"),
        context: &context,
        tick_context: &tick,
    };
    DefenderRead::phase(&ctx)
}

#[test]
fn a_tired_defender_stands_down_where_a_fresh_one_is_alert() {
    assert_eq!(phase_of_a_defender_at(9000), DefenderState::HoldingLine);
    assert_eq!(phase_of_a_defender_at(5500), DefenderState::Walking);
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
