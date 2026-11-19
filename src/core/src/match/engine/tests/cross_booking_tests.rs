//! **A cross is booked by what it is and completed by its first contact.**
//! The left side attacks the goal at `x = 840`; its right wing runs along
//! the top touchline.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::player::events::players::PlayerEventDispatcher;
use crate::r#match::engine::result::Score;
use crate::r#match::passing::CrossModel;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, events::EventCollection,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
/// Wide on the right wing, deep in the final third.
const WIDE: Vector3<f32> = Vector3::new(730.0, 40.0, 0.0);

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

fn outfielder(field: &MatchField, side: PlayerSide, nth: usize) -> u32 {
    field
        .players
        .iter()
        .filter(|p| p.side == Some(side) && !p.tactical_position.current_position.is_goalkeeper())
        .nth(nth)
        .map(|p| p.id)
        .expect("a side has outfielders")
}

fn keeper(field: &MatchField, side: PlayerSide) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.side == Some(side) && p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .expect("a side has a keeper")
}

fn touch(field: &mut MatchField, context: &MatchContext, player: u32, controlled: bool) {
    let team = field
        .players
        .iter()
        .find(|p| p.id == player)
        .unwrap()
        .team_id;
    field
        .ball
        .record_touch(player, team, context.current_tick(), controlled);
}

/// The left side's crosser strikes a booked cross.
fn struck_cross(field: &mut MatchField, context: &MatchContext) -> u32 {
    let crosser = outfielder(field, PlayerSide::Left, 0);
    touch(field, context, crosser, true);
    let team = field
        .players
        .iter()
        .find(|p| p.id == crosser)
        .unwrap()
        .team_id;
    field.ball.on_cross_struck(crosser, team);
    crosser
}

#[test]
fn a_ground_ball_to_the_edge_of_the_area_is_a_pass() {
    let (_, context) = kickoff();
    let at_the_edge = Vector3::new(840.0 - 125.0, 272.0, 0.0);
    assert!(!CrossModel::books_as_cross(
        WIDE,
        at_the_edge,
        false,
        PlayerSide::Left,
        &context
    ));
}

/// Lofted into the box from wide, it is a cross — whoever's decision it
/// was, and wherever in the box it drops.
#[test]
fn a_lofted_ball_into_the_box_from_wide_is_a_cross() {
    let (_, context) = kickoff();
    for to in [
        Vector3::new(800.0, 255.0, 0.0),
        Vector3::new(840.0 - 125.0, 272.0, 0.0),
    ] {
        assert!(CrossModel::books_as_cross(
            WIDE,
            to,
            true,
            PlayerSide::Left,
            &context
        ));
    }
}

#[test]
fn a_ball_driven_across_the_box_is_a_cross_and_a_short_one_to_feet_is_a_pass() {
    let (_, context) = kickoff();
    assert!(CrossModel::books_as_cross(
        Vector3::new(790.0, 60.0, 0.0),
        Vector3::new(812.0, 300.0, 0.0),
        false,
        PlayerSide::Left,
        &context
    ));
    assert!(!CrossModel::books_as_cross(
        Vector3::new(800.0, 80.0, 0.0),
        Vector3::new(815.0, 140.0, 0.0),
        false,
        PlayerSide::Left,
        &context
    ));
}

#[test]
fn nothing_from_the_middle_or_short_of_the_box_is_a_cross() {
    let (_, context) = kickoff();
    let into_the_box = Vector3::new(800.0, 255.0, 0.0);
    assert!(!CrossModel::books_as_cross(
        Vector3::new(700.0, 272.0, 0.0),
        into_the_box,
        true,
        PlayerSide::Left,
        &context
    ));
    assert!(!CrossModel::books_as_cross(
        WIDE,
        Vector3::new(840.0 - 140.0, 272.0, 0.0),
        true,
        PlayerSide::Left,
        &context
    ));
}

#[test]
fn a_cross_won_in_the_air_is_completed() {
    let (mut field, context) = kickoff();
    let crosser = struck_cross(&mut field, &context);
    let header = outfielder(&field, PlayerSide::Left, 5);
    field.ball.position.z = 2.0;
    touch(&mut field, &context, header, false);
    assert_eq!(field.ball.completed_cross, Some(crosser));
}

#[test]
fn a_blocked_cross_that_runs_to_an_attacker_is_not_completed() {
    let (mut field, context) = kickoff();
    struck_cross(&mut field, &context);
    let closer = outfielder(&field, PlayerSide::Right, 0);
    touch(&mut field, &context, closer, false);
    let attacker = outfielder(&field, PlayerSide::Left, 5);
    touch(&mut field, &context, attacker, true);
    assert!(
        field.ball.completed_cross.is_none(),
        "the defender played it first"
    );
}

#[test]
fn a_claimed_cross_is_not_completed() {
    let (mut field, context) = kickoff();
    struck_cross(&mut field, &context);
    let keeper = keeper(&field, PlayerSide::Right);
    touch(&mut field, &context, keeper, true);
    assert!(field.ball.completed_cross.is_none());
}

#[test]
fn a_cross_that_goes_out_is_not_completed() {
    let (mut field, context) = kickoff();
    struck_cross(&mut field, &context);
    field.ball.position = Vector3::new(600.0, -1.0, 0.0);
    field.ball.velocity = Vector3::new(0.0, -1.4, 0.0);
    field.ball.current_owner = None;
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field.ball.check_throw_in(&context, &players, &mut events);
    let attacker = outfielder(&field, PlayerSide::Left, 5);
    touch(&mut field, &context, attacker, true);
    assert!(field.ball.completed_cross.is_none());
}

#[test]
fn no_cross_is_credited_twice() {
    let (mut field, context) = kickoff();
    let crosser = struck_cross(&mut field, &context);
    let first = outfielder(&field, PlayerSide::Left, 5);
    touch(&mut field, &context, first, true);
    assert_eq!(field.ball.completed_cross.take(), Some(crosser));
    let second = outfielder(&field, PlayerSide::Left, 6);
    touch(&mut field, &context, second, true);
    assert!(
        field.ball.completed_cross.is_none(),
        "decided once, at the first contact"
    );
}

/// The reception rule credits the pass and leaves the cross to its first
/// contact.
#[test]
fn a_received_cross_is_not_credited_by_the_reception() {
    let (mut field, context) = kickoff();
    let crosser = outfielder(&field, PlayerSide::Left, 0);
    let receiver = outfielder(&field, PlayerSide::Left, 5);
    field.ball.pending_pass_passer = Some(crosser);
    field.ball.pending_pass_origin = Some(WIDE);
    field.ball.pending_pass_target = Some(Vector3::new(800.0, 255.0, 0.0));
    field.ball.pending_pass_was_cross = true;
    PlayerEventDispatcher::credit_completed_pass(receiver, crosser, &mut field, &context);
    let stats = &field
        .players
        .iter()
        .find(|p| p.id == crosser)
        .unwrap()
        .statistics;
    assert_eq!(stats.passes_completed, 1);
    assert_eq!(stats.crosses_completed, 0);
}
