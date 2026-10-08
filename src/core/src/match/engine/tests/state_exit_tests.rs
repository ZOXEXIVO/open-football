//! States leave on a football reason rather than a tick count: a resting
//! forward when his legs are back or the play wants him, a closed-down
//! carrier when the pressure says release it.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerFieldPositionGroup;
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::{ForwardRestingState, ForwardState};
use crate::r#match::midfielders::states::{MidfielderRunningState, MidfielderState};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, PlayerSide, StateProcessingHandler,
    StateProcessingResult, StateProcessor,
};
use nalgebra::Vector3;

fn first_of(field: &MatchField, team_id: u32, group: PlayerFieldPositionGroup) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.team_id == team_id && p.tactical_position.current_position.position_group() == group
        })
        .map(|p| p.id)
        .unwrap()
}

fn decide<H: StateProcessingHandler>(
    field: &mut MatchField,
    context: &MatchContext,
    id: u32,
    in_state_time: u64,
    handler: H,
) -> StateProcessingResult {
    let tick = GameTickContext::new(field, &context.players);
    let player = field.players.iter_mut().find(|p| p.id == id).unwrap();
    StateProcessor::new(in_state_time, player, context, &tick).process_inner(handler)
}

fn place(field: &mut MatchField, id: u32, at: Vector3<f32>) {
    let p = field.get_player_mut(id).unwrap();
    p.position = at;
    p.velocity = Vector3::zeros();
}

/// The away keeper on the ball at his own goal, the home forward resting
/// deep in his own half, far from it.
fn resting_forward(field: &mut MatchField) -> u32 {
    let forward = first_of(field, 1, PlayerFieldPositionGroup::Forward);
    let keeper = first_of(field, 2, PlayerFieldPositionGroup::Goalkeeper);
    let (own_x, their_goal_x) = match field.side_of(1) {
        PlayerSide::Left => (150.0, 800.0),
        PlayerSide::Right => (690.0, 40.0),
    };
    place(field, forward, Vector3::new(own_x, 272.0, 0.0));
    place(field, keeper, Vector3::new(their_goal_x, 272.0, 0.0));
    field.ball.position = Vector3::new(their_goal_x, 272.0, 0.0);
    field.ball.current_owner = Some(keeper);
    field.get_player_mut(forward).unwrap().redirect_to_fresh(
        PlayerState::Forward(ForwardState::Resting),
        TransitionSource::Reset,
    );
    forward
}

#[test]
fn a_resting_forward_gets_up_when_his_legs_are_back_and_not_on_a_timer() {
    let (mut field, context) = kickoff();
    let forward = resting_forward(&mut field);

    field
        .get_player_mut(forward)
        .unwrap()
        .player_attributes
        .condition = 5_900;
    let still = decide(
        &mut field,
        &context,
        forward,
        600,
        ForwardRestingState::default(),
    );
    assert_eq!(still.state, None, "the old 500-tick timeout is gone");

    field
        .get_player_mut(forward)
        .unwrap()
        .player_attributes
        .condition = 6_000;
    let up = decide(
        &mut field,
        &context,
        forward,
        10,
        ForwardRestingState::default(),
    );
    assert_eq!(up.state, Some(PlayerState::Forward(ForwardState::Walking)));
}

#[test]
fn a_resting_forward_answers_a_play_that_comes_to_him() {
    let (mut field, context) = kickoff();
    let forward = resting_forward(&mut field);
    field
        .get_player_mut(forward)
        .unwrap()
        .player_attributes
        .condition = 4_000;
    let at = field.get_player(forward).unwrap().position;
    field.ball.position = at + Vector3::new(0.0, 120.0, 0.0);

    let result = decide(
        &mut field,
        &context,
        forward,
        10,
        ForwardRestingState::default(),
    );
    assert_eq!(
        result.state,
        Some(PlayerState::Forward(ForwardState::Walking))
    );
}

/// A home midfielder on the ball in the centre circle, every away player
/// well away — unless `presser` puts one at his shoulder.
fn carrier(field: &mut MatchField, presser: bool) -> u32 {
    let mid = first_of(field, 1, PlayerFieldPositionGroup::Midfielder);
    let centre = Vector3::new(420.0, 272.0, 0.0);
    let away: Vec<u32> = field
        .players
        .iter()
        .filter(|p| p.team_id == 2)
        .map(|p| p.id)
        .collect();
    let far_x = match field.side_of(1) {
        PlayerSide::Left => 760.0,
        PlayerSide::Right => 80.0,
    };
    for (i, id) in away.iter().enumerate() {
        place(field, *id, Vector3::new(far_x, 40.0 + i as f32 * 40.0, 0.0));
    }
    if presser {
        place(field, away[5], centre + Vector3::new(10.0, 0.0, 0.0));
    }
    place(field, mid, centre);
    let m = field.get_player_mut(mid).unwrap();
    m.skills.mental.composure = 10.0;
    m.redirect_to_fresh(
        PlayerState::Midfielder(MidfielderState::Running),
        TransitionSource::Reset,
    );
    field.ball.position = centre;
    field.ball.current_owner = Some(mid);
    mid
}

fn forced_release(mut result: StateProcessingResult) -> bool {
    result.events.drain().any(|event| {
        matches!(
            event,
            Event::PlayerEvent(PlayerEvent::PassTo(ref pass))
                if pass.reason.starts_with("MID_RUNNING_ANTI_OSCILLATION")
        )
    })
}

#[test]
fn a_closed_down_carrier_releases_it_because_of_the_pressure() {
    let (mut field, context) = kickoff();
    let mid = carrier(&mut field, true);
    // 1.4 s on the ball: past what a middling composure shields for, well
    // inside the old 3 s timer.
    let result = decide(
        &mut field,
        &context,
        mid,
        70,
        MidfielderRunningState::default(),
    );
    assert!(forced_release(result), "pressed, he kept the ball");

    let (mut field, context) = kickoff();
    let mid = carrier(&mut field, false);
    let result = decide(
        &mut field,
        &context,
        mid,
        70,
        MidfielderRunningState::default(),
    );
    assert!(
        !forced_release(result),
        "left alone, he was forced to release it"
    );
}
