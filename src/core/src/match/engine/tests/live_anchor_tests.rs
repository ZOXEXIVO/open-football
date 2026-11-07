//! **"How far have I strayed" is measured from where the shape wants him.**
//! Each scenario puts the kickoff dot and the live anchor somewhere different
//! and asks which one the decision followed.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::forwarders::states::pressing::ForwardPressingState;
use crate::r#match::player::strategies::midfielders::states::guarding::MidfielderGuardingState;
use crate::r#match::{
    DefensivePlan, GameTickContext, MatchContext, MatchField, MatchPlayerCollection,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// A pitch with both plans refreshed, and the home side's defensive plan
/// emptied so every man's anchor is his place in the shape.
fn shaped_pitch(owner_away_nth: usize) -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    let owner = field
        .players
        .iter()
        .filter(|p| p.team_id == 2)
        .nth(owner_away_nth)
        .expect("an away player")
        .id;
    field.ball.current_owner = Some(owner);
    field.ball.position = field.get_player(owner).expect("owner").position;
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    let possession = TeamPossession::read(&field);
    FootballEngine::<840, 545>::refresh_tactical_states(&field, &mut context, possession, 1);
    context.defence_home = DefensivePlan::idle();
    (field, context)
}

fn home_man(field: &MatchField, matches_role: impl Fn(&PlayerState) -> bool) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.team_id == 1 && matches_role(&p.state))
        .expect("a home player in that role")
        .id
}

fn anchor_of(field: &MatchField, context: &MatchContext, id: u32) -> Vector3<f32> {
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 10,
        player: field.get_player(id).expect("a player"),
        context,
        tick_context: &tick,
    };
    ctx.team().my_anchor()
}

/// Stand him at `offset` from his anchor with the kickoff dot `dot_gap` off
/// it, and an opponent beside him to guard.
fn place_guard(field: &mut MatchField, context: &MatchContext, id: u32, offset: f32, dot_gap: f32) {
    let anchor = anchor_of(field, context, id);
    let at = anchor + Vector3::new(offset, 0.0, 0.0);
    let man = field
        .players
        .iter()
        .filter(|p| p.team_id == 2 && field.ball.current_owner != Some(p.id))
        .last()
        .expect("an opponent")
        .id;
    field.get_player_mut(man).expect("a man").position = at + Vector3::new(4.0, 6.0, 0.0);
    // Close enough that "the ball is far away, stop guarding" does not answer
    // first, and too far for him to go and press it.
    let carrier = field.ball.current_owner.expect("an owner");
    field.ball.position = at + Vector3::new(100.0, 80.0, 0.0);
    field.get_player_mut(carrier).expect("a carrier").position = field.ball.position;
    let guard = field.get_player_mut(id).expect("a guard");
    guard.position = at;
    guard.start_position = anchor - Vector3::new(dot_gap, 0.0, 0.0);
}

fn midfielder_decision(field: &MatchField, context: &MatchContext, id: u32) -> Option<PlayerState> {
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 40,
        player: field.get_player(id).expect("a player"),
        context,
        tick_context: &tick,
    };
    MidfielderGuardingState::default()
        .process(&ctx)
        .and_then(|change| change.state)
}

#[test]
fn a_midfielder_who_moved_with_the_block_keeps_his_man() {
    let (mut field, context) = shaped_pitch(1);
    let id = home_man(&field, |s| matches!(s, PlayerState::Midfielder(_)));
    // More than 19 m (152 u) from where he stood at kickoff, and right where
    // the shape wants him.
    place_guard(&mut field, &context, id, 10.0, 200.0);
    assert_eq!(midfielder_decision(&field, &context, id), None);
}

#[test]
fn a_man_genuinely_out_of_position_still_goes_back() {
    let (mut field, context) = shaped_pitch(1);
    let id = home_man(&field, |s| matches!(s, PlayerState::Midfielder(_)));
    // Beyond his guarding range of the live anchor, with the kickoff dot
    // beside him: the old read would have let him carry on.
    place_guard(&mut field, &context, id, 200.0, -200.0);
    assert_eq!(
        midfielder_decision(&field, &context, id),
        Some(PlayerState::Midfielder(MidfielderState::Returning)),
    );
}

#[test]
fn a_forward_pressing_high_with_his_team_pushed_up_keeps_pressing() {
    let (mut field, context) = shaped_pitch(1);
    let id = home_man(&field, |s| matches!(s, PlayerState::Forward(_)));
    let anchor = anchor_of(&field, &context, id);
    // In the opposition half, on his anchor, and far past the dot's 25 m.
    field.ball.position = anchor + Vector3::new(40.0, 0.0, 0.0);
    let owner = field.ball.current_owner.expect("an owner");
    field.get_player_mut(owner).expect("owner").position = field.ball.position;
    let forward = field.get_player_mut(id).expect("a forward");
    forward.position = anchor;
    forward.start_position = anchor - Vector3::new(300.0, 0.0, 0.0);

    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 10,
        player: field.get_player(id).expect("a player"),
        context: &context,
        tick_context: &tick,
    };
    assert_ne!(
        ForwardPressingState::default()
            .process(&ctx)
            .and_then(|change| change.state),
        Some(PlayerState::Forward(ForwardState::Returning)),
    );
}
