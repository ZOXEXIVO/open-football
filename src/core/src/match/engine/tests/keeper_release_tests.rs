//! A keeper who has chosen to play it short waits for somebody to come
//! free, for as long as his side's tempo and the referee's count allow,
//! and only then goes long.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::goalkeepers::states::common::KeeperRelease;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, PlayerSide, ResultMatchPositionData,
};
use nalgebra::Vector3;

const PARKED: Vector3<f32> = Vector3::new(-500.0, -500.0, 0.0);

/// The home keeper on the edge of his area with the ball in his gloves,
/// gathered `held_ticks` ago, looking for a short pass with every home
/// outfielder off the pitch. Returns his id.
fn keeper_looking_short(field: &mut MatchField, context: &MatchContext, held_ticks: u64) -> u32 {
    let side = field.side_of(1);
    let edge_x = match side {
        PlayerSide::Left => 132.0,
        PlayerSide::Right => 840.0 - 132.0,
    };
    let keeper = field
        .players
        .iter()
        .find(|p| {
            p.team_id == 1
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
        })
        .map(|p| p.id)
        .unwrap();
    for p in field
        .players
        .iter_mut()
        .filter(|p| p.team_id == 1 && p.id != keeper)
    {
        p.off_pitch = true;
        p.position = PARKED;
        p.velocity = Vector3::zeros();
    }
    let at = Vector3::new(edge_x, 272.0, 0.0);
    let k = field.get_player_mut(keeper).unwrap();
    k.position = at;
    k.velocity = Vector3::zeros();
    k.skills.mental.decisions = 10.0;
    k.redirect_to_fresh(
        PlayerState::Goalkeeper(GoalkeeperState::Distributing),
        TransitionSource::Reset,
    );
    field.ball.position = at;
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = true;
    field.ball.hands_since_tick = context.current_tick() - held_ticks;
    keeper
}

fn tick(field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::empty();
    let mut tick_context = GameTickContext::new(field, &context.players);
    FootballEngine::<840, 545>::game_tick(field, context, &mut data, &mut tick_context);
    context.increment_time();
}

#[test]
fn he_waits_inside_his_patience_and_never_past_the_count() {
    assert!(KeeperRelease::may_wait(30, 0.5, Some(3_000), 10.0));
    assert!(
        !KeeperRelease::may_wait(150, 0.5, Some(3_000), 10.0),
        "patience spent"
    );
    assert!(
        !KeeperRelease::may_wait(10, 0.5, Some(6_800), 10.0),
        "too near the count"
    );
    // A side playing at full tempo looks for a second at most.
    assert!(!KeeperRelease::may_wait(50, 1.0, None, 10.0));
    for decisions in [1.0, 10.0, 20.0] {
        let last_held = (0..KeeperRelease::HANDS_LIMIT_MS)
            .step_by(10)
            .take_while(|&held| KeeperRelease::may_wait(0, 0.0, Some(held), decisions))
            .last()
            .unwrap();
        assert!(
            last_held + 500 <= KeeperRelease::HANDS_LIMIT_MS,
            "decisions {decisions} waits to {last_held} ms"
        );
    }
}

#[test]
fn a_short_option_who_comes_free_inside_the_window_gets_the_ball() {
    let (mut field, mut context) = kickoff();
    let keeper = keeper_looking_short(&mut field, &context, 0);

    // Well past the old fixed timeout of twenty decisions.
    for _ in 0..60 {
        tick(&mut field, &mut context);
    }
    assert_eq!(
        field.get_player(keeper).unwrap().state,
        PlayerState::Goalkeeper(GoalkeeperState::Distributing),
        "he gave up on the short pass"
    );
    assert_eq!(field.ball.current_owner, Some(keeper));

    let forward = match field.side_of(1) {
        PlayerSide::Left => 1.0,
        PlayerSide::Right => -1.0,
    };
    let keeper_at = field.get_player(keeper).unwrap().position;
    let mate = field
        .players
        .iter()
        .find(|p| {
            p.team_id == 1
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Midfielder
        })
        .map(|p| p.id)
        .unwrap();
    let m = field.get_player_mut(mate).unwrap();
    m.off_pitch = false;
    m.position = keeper_at + Vector3::new(forward * 160.0, 0.0, 0.0);

    let mut released_to = None;
    for _ in 0..10 {
        tick(&mut field, &mut context);
        if field.ball.current_owner != Some(keeper) {
            released_to = field.ball.pass_target_player_id;
            break;
        }
    }
    assert_eq!(
        released_to,
        Some(mate),
        "he did not play it to the man who came free"
    );
}

#[test]
fn with_nobody_free_he_goes_long_before_the_count() {
    let (mut field, mut context) = kickoff();
    // Six and a half seconds in his gloves already.
    let keeper = keeper_looking_short(&mut field, &context, 650);
    let count_ends = context.current_tick() + 150;

    while context.current_tick() < count_ends {
        tick(&mut field, &mut context);
        if field.ball.current_owner != Some(keeper) {
            break;
        }
    }
    assert_ne!(
        field.ball.current_owner,
        Some(keeper),
        "still holding at the count"
    );
    assert!(
        field.ball.awaiting_restart.is_none(),
        "the referee gave a corner for holding"
    );
}
