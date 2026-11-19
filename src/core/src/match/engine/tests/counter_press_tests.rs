//! A carrier who has the ball taken off him goes straight back at it, or
//! lets it go, by his own attributes and the team's press; a team-mate near
//! him joins him as far as he presses with his team.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::{CounterPress, CounterPressCall, LostBall};
use crate::r#match::engine::engine::FootballEngine;
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
    assert!(field.get_player(carrier).unwrap().lost_ball.is_some());

    decide(&mut field, &mut context);

    let state = field.get_player(carrier).unwrap().state;
    assert!(
        matches!(
            state,
            PlayerState::Midfielder(
                MidfielderState::Pressing | MidfielderState::Jockeying | MidfielderState::Tackling
            )
        ),
        "he went to {state:?}"
    );
    assert!(field.get_player(carrier).unwrap().lost_ball.is_none());

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
    // The new owner is at his shoulder, so the read may still have him
    // contain the man by contact; what his reaction decides is whether he
    // goes after the ball himself.
    let tick_context = GameTickContext::new(&field, &context.players);
    let player = field.get_player(carrier).unwrap();
    assert_eq!(
        CounterPress::call(player, player.lost_ball.unwrap(), &context, &tick_context),
        CounterPressCall::LetGo,
        "he pressed with nothing in his legs"
    );

    decide(&mut field, &mut context);

    assert!(
        field.get_player(carrier).unwrap().lost_ball.is_none(),
        "the reaction lapsed undecided"
    );
}

#[test]
fn a_steal_at_the_ball_layer_is_the_same_fact() {
    let (mut field, mut context) = kickoff();
    let loser = midfielder(&field, 1);
    let taker = midfielder(&field, 2);
    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(
        PlayerEvent::Dispossessed(loser, taker),
        &mut field,
        &mut context,
        &mut data,
    );
    assert_eq!(
        field.get_player(loser).unwrap().lost_ball,
        Some(LostBall {
            at: context.current_tick(),
            himself: true
        })
    );
}

/// A second home midfielder five metres from where the tackle will put the
/// ball, with an ordinary mind for the game apart from `work_rate`, and
/// `condition`.
fn team_mate_near(field: &mut MatchField, work_rate: f32, condition: i16) -> u32 {
    let carrier = midfielder(field, 1);
    let id = field
        .players
        .iter()
        .find(|p| {
            p.team_id == 1
                && p.id != carrier
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Midfielder
        })
        .map(|p| p.id)
        .unwrap();
    let p = field.get_player_mut(id).unwrap();
    p.position = Vector3::new(426.0, 312.0, 0.0);
    p.velocity = Vector3::zeros();
    p.redirect_to_fresh(
        PlayerState::Midfielder(MidfielderState::Running),
        TransitionSource::Reset,
    );
    let m = &mut p.skills.mental;
    for skill in [
        &mut m.decisions,
        &mut m.positioning,
        &mut m.anticipation,
        &mut m.concentration,
        &mut m.teamwork,
        &mut m.aggression,
        &mut m.determination,
    ] {
        *skill = 12.0;
    }
    m.work_rate = work_rate;
    p.player_attributes.condition = condition;
    id
}

/// Whether the team-mate goes when his side presses hard.
fn joins(work_rate: f32, condition: i16) -> CounterPressCall {
    let (mut field, mut context) = kickoff();
    let mate = team_mate_near(&mut field, work_rate, condition);
    tackled(&mut field, &mut context, 12.0, 10_000);
    context.tactical_home.press_intensity = 0.8;
    for p in field.players.iter_mut() {
        p.read_adherence(1.0);
    }
    let player = field.get_player(mate).unwrap();
    let lost = player.lost_ball.expect("a team-mate near it is asked");
    assert!(!lost.himself);
    let tick_context = GameTickContext::new(&field, &context.players);
    CounterPress::call(player, lost, &context, &tick_context)
}

#[test]
fn a_team_mate_near_the_turnover_joins_the_press() {
    assert!(matches!(joins(16.0, 10_000), CounterPressCall::Press(_)));
}

#[test]
fn the_press_fades_with_the_legs() {
    assert!(matches!(joins(16.0, 10_000), CounterPressCall::Press(_)));
    assert_eq!(joins(16.0, 2_500), CounterPressCall::LetGo);
}

#[test]
fn work_rate_keeps_a_player_in_the_press() {
    assert!(matches!(joins(18.0, 10_000), CounterPressCall::Press(_)));
    assert_eq!(joins(3.0, 10_000), CounterPressCall::LetGo);
}

#[test]
fn a_team_mate_far_from_it_is_not_asked() {
    let (mut field, mut context) = kickoff();
    let mate = team_mate_near(&mut field, 18.0, 10_000);
    field.get_player_mut(mate).unwrap().position = Vector3::new(200.0, 100.0, 0.0);
    tackled(&mut field, &mut context, 12.0, 10_000);
    assert_eq!(field.get_player(mate).unwrap().lost_ball, None);
}
