//! A carrier who has the ball taken off him goes straight back at it, or
//! lets it go, by his own attributes and the team's press.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::player::counter_press::CounterPress;
use crate::r#match::engine::player::events::players::{PlayerEvent, PlayerEventDispatcher};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{GameTickContext, MatchContext, MatchField, ResultMatchPositionData};
use nalgebra::Vector3;

fn midfielder(field: &MatchField, team_id: u32) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.team_id == team_id
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Midfielder
        })
        .map(|p| p.id)
        .unwrap()
}

/// A home midfielder on the ball in the centre circle with an away
/// midfielder a stride off, who then tackles him. Returns both ids.
fn tackled(
    field: &mut MatchField,
    context: &mut MatchContext,
    work_rate: f32,
    condition: i16,
) -> (u32, u32) {
    let carrier = midfielder(field, 1);
    let tackler = midfielder(field, 2);
    let centre = Vector3::new(420.0, 272.0, 0.0);
    let toe_poke = centre + Vector3::new(6.0, 0.0, 0.0);
    for (id, at) in [(carrier, centre), (tackler, toe_poke)] {
        let p = field.get_player_mut(id).unwrap();
        p.position = at;
        p.velocity = Vector3::zeros();
        p.redirect_to_fresh(
            PlayerState::Midfielder(MidfielderState::Running),
            TransitionSource::Reset,
        );
    }
    let p = field.get_player_mut(carrier).unwrap();
    p.skills.mental.work_rate = work_rate;
    p.skills.mental.aggression = work_rate;
    p.skills.mental.determination = work_rate;
    p.player_attributes.condition = condition;
    field.ball.position = centre;
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(carrier);
    field.ball.previous_owner = None;

    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(
        PlayerEvent::TacklingBall(tackler),
        field,
        context,
        &mut data,
    );
    // He poked it away to his own feet, out of the carrier's reach — the
    // challenge itself may as easily knock it loose, and that is not what
    // these tests are about.
    field.ball.current_owner = Some(tackler);
    field.ball.position = toe_poke;
    (carrier, tackler)
}

fn decide(field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::empty();
    for _ in 0..2 {
        let mut tick_context = GameTickContext::new(field, &context.players);
        FootballEngine::<840, 545>::game_tick(field, context, &mut data, &mut tick_context);
        context.increment_time();
    }
}

#[test]
fn a_tackled_carrier_presses_the_new_owner_and_the_pressure_counts() {
    let (mut field, mut context) = kickoff();
    let (carrier, tackler) = tackled(&mut field, &mut context, 17.0, 10_000);
    assert!(field.get_player(carrier).unwrap().lost_ball_at.is_some());

    decide(&mut field, &mut context);

    let state = field.get_player(carrier).unwrap().state;
    assert!(
        matches!(
            state,
            PlayerState::Midfielder(MidfielderState::Pressing | MidfielderState::Tackling)
        ),
        "he went to {state:?}"
    );
    assert!(field.get_player(carrier).unwrap().lost_ball_at.is_none());

    // The new owner plays it on with him still at his shoulder.
    let from = field.get_player(tackler).unwrap().position;
    PlayerEventDispatcher::credit_pressures_on_pass(tackler, from, 2, &mut field, &context);
    assert_eq!(field.get_player(carrier).unwrap().statistics.pressures, 1);
}

#[test]
fn a_spent_unwilling_carrier_lets_it_go() {
    let (mut field, mut context) = kickoff();
    let (carrier, _) = tackled(&mut field, &mut context, 3.0, 2_500);
    let urge = CounterPress::urge(field.get_player(carrier).unwrap(), 0.5);
    assert!(urge < 0.25, "urge {urge}");

    decide(&mut field, &mut context);

    let player = field.get_player(carrier).unwrap();
    assert!(
        player.lost_ball_at.is_none(),
        "the reaction lapsed undecided"
    );
    assert!(
        !matches!(
            player.state,
            PlayerState::Midfielder(MidfielderState::Pressing)
        ),
        "he pressed with nothing in his legs"
    );
}

#[test]
fn a_steal_at_the_ball_layer_is_the_same_fact() {
    let (mut field, mut context) = kickoff();
    let loser = midfielder(&field, 1);
    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(
        PlayerEvent::Dispossessed(loser),
        &mut field,
        &mut context,
        &mut data,
    );
    assert_eq!(
        field.get_player(loser).unwrap().lost_ball_at,
        Some(context.current_tick())
    );
}
