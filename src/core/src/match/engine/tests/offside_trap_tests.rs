//! The offside trap: an organised back line steps up together and leaves
//! the runner offside; one man late plays him on, and the attempt is on
//! the record either way.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerPositionType;
use crate::r#match::defenders::states::common::LineStep;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent, PlayerEventDispatcher};
use crate::r#match::{
    DefenceRefreshInputs, DefensivePlan, GameTickContext, MatchContext, MatchField, PlayerSide,
    ResultMatchPositionData, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    StateProcessor,
};
use nalgebra::Vector3;
use std::cell::Cell;

const LINE_X: f32 = 300.0;
const BACK_FOUR: [PlayerPositionType; 4] = [
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
];

struct Picture {
    carrier: u32,
    runner: u32,
    back_four: [u32; 4],
}

fn holder(field: &MatchField, team_id: u32, position: PlayerPositionType) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.team_id == team_id && p.tactical_position.current_position == position)
        .map(|p| p.id)
        .unwrap()
}

fn place(field: &mut MatchField, id: u32, at: Vector3<f32>) {
    let p = field.get_player_mut(id).unwrap();
    p.position = at;
    p.velocity = Vector3::zeros();
}

/// The home back four flat on a line 37.5 m out with these readers of the
/// game in it, the away side on the ball 19 m in front of it, and an away
/// forward on the shoulder of the line, just onside.
fn picture(field: &mut MatchField, anticipation: [f32; 4]) -> Picture {
    assert_eq!(field.side_of(1), PlayerSide::Left);
    let back_four = BACK_FOUR.map(|position| holder(field, 1, position));
    let carrier = holder(field, 2, PlayerPositionType::MidfielderCenterLeft);
    let runner = holder(field, 2, PlayerPositionType::ForwardLeft);
    let ids: Vec<(u32, u32)> = field.players.iter().map(|p| (p.id, p.team_id)).collect();
    for (id, team_id) in ids {
        let y = field.get_player(id).unwrap().start_position.y;
        let x = if team_id == 1 { 560.0 } else { 760.0 };
        place(field, id, Vector3::new(x, y, 0.0));
    }
    place(
        field,
        holder(field, 1, PlayerPositionType::Goalkeeper),
        Vector3::new(20.0, 272.0, 0.0),
    );
    for (id, reading) in back_four.iter().zip(anticipation) {
        let y = field.get_player(*id).unwrap().start_position.y;
        place(field, *id, Vector3::new(LINE_X, y, 0.0));
        field
            .get_player_mut(*id)
            .unwrap()
            .skills
            .mental
            .anticipation = reading;
    }
    place(field, carrier, Vector3::new(LINE_X + 150.0, 272.0, 0.0));
    place(field, runner, Vector3::new(LINE_X + 5.0, 250.0, 0.0));
    field.ball.position = Vector3::new(LINE_X + 150.0, 272.0, 0.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(carrier);
    Picture {
        carrier,
        runner,
        back_four,
    }
}

fn call(field: &MatchField, context: &mut MatchContext) {
    DefensivePlan::refresh(
        &mut context.defence_home,
        &mut context.defence_away,
        &DefenceRefreshInputs {
            field,
            home_team_id: 1,
            away_team_id: 2,
            home_keeper_voice: 0.56,
            away_keeper_voice: 0.56,
        },
    );
    let tick = context.current_tick();
    context
        .defence_home
        .call_line_step(field, 1, 0.65, 0.56, tick);
}

struct Probe<'a>(&'a Cell<Option<Option<f32>>>);

impl StateProcessingHandler for Probe<'_> {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        self.0
            .set(Some(LineStep::depth(ctx, ctx.player.position.x)));
        None
    }
}

fn stepped_depth(field: &mut MatchField, context: &MatchContext, id: u32) -> Option<f32> {
    let tick = GameTickContext::new(field, &context.players);
    let out = Cell::new(None);
    let player = field.players.iter_mut().find(|p| p.id == id).unwrap();
    StateProcessor::new(0, player, context, &tick).process_inner(Probe(&out));
    out.get().unwrap()
}

/// Two thirds of a second after the call, every man who has read it goes
/// to where he steps to; the ball is then played to the runner.
fn play_it_in(field: &mut MatchField, context: &mut MatchContext, picture: &Picture) {
    context.total_match_time += 600;
    for id in picture.back_four {
        if let Some(x) = stepped_depth(field, context, id) {
            let y = field.get_player(id).unwrap().position.y;
            place(field, id, Vector3::new(x, y, 0.0));
        }
    }
    let target = field.get_player(picture.runner).unwrap().position;
    let pass = PassingEventContext {
        from_player_id: picture.carrier,
        to_player_id: picture.runner,
        pass_target: target,
        pass_force: 1.0,
        reason: "TEST_THROUGH_BALL",
        cross_type: None,
        target_is_space: false,
    };
    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(PlayerEvent::PassTo(pass), field, context, &mut data);
}

#[test]
fn an_organised_line_steps_up_together_and_leaves_the_runner_offside() {
    let (mut field, mut context) = kickoff();
    let picture = picture(&mut field, [18.0; 4]);
    call(&field, &mut context);
    let called = context
        .defence_home
        .line_step
        .expect("an organised line calls it");
    assert_eq!(called.runner, picture.runner);

    play_it_in(&mut field, &mut context, &picture);

    for id in picture.back_four {
        assert!(
            field.get_player(id).unwrap().position.x > LINE_X,
            "{id} never stepped"
        );
    }
    assert_eq!(
        context.tally.offside_traps,
        [1, 0],
        "the trap was not sprung"
    );
}

#[test]
fn one_man_late_plays_the_runner_on_and_the_trap_is_beaten() {
    let (mut field, mut context) = kickoff();
    let picture = picture(&mut field, [18.0, 18.0, 2.0, 18.0]);
    call(&field, &mut context);
    assert!(context.defence_home.line_step.is_some());

    play_it_in(&mut field, &mut context, &picture);

    let late = picture.back_four[2];
    assert_eq!(
        field.get_player(late).unwrap().position.x,
        LINE_X,
        "the slow reader went too"
    );
    assert_eq!(
        context.tally.offside_traps,
        [0, 1],
        "the failed trap is not on the record"
    );
}

#[test]
fn a_disorganised_line_does_not_try_it() {
    let (mut field, mut context) = kickoff();
    picture(&mut field, [3.0; 4]);
    call(&field, &mut context);
    assert_eq!(context.defence_home.line_step, None);
}
