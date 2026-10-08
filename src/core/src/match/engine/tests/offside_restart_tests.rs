//! The offside restart, end to end.
//!
//! The rule itself is pinned by `OffsideLine`'s own tests; what is pinned
//! here is what the flag DOES to the pitch, and the bug is a visual one.
//! The award used to relocate two things on every one of the 9-12 offsides
//! a match: the ball, written back to where the receiver stood when the
//! pass was played — the whole length of his run onto a through-ball — and
//! the defender taking it, staged onto the spot by
//! `pending_set_piece_teleport`. Between them that was the largest ball
//! teleport left in a match.
//!
//! So the assertions are about what MOVES, not about whether the offside
//! was given. A test on the final state alone would pass against the
//! teleport.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{AwaitedRestart, OffsideSnapshot, PassOriginRestart};
use crate::r#match::engine::result::Score;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, events::EventCollection,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

/// How far behind the reception the receiver was standing when the pass was
/// played — i.e. the length of his run, and exactly what the old restart
/// dragged the ball back by.
const RUN_LENGTH: f32 = 160.0; // 20 m

/// Stage a through-ball that is about to be flagged: the Left side's
/// forward has run 20 m beyond the line and the ball has just reached him.
/// Returns `(receiver_id, ball_position)`.
fn a_through_ball_about_to_be_flagged(
    field: &mut MatchField,
    context: &MatchContext,
) -> (u32, Vector3<f32>) {
    let receiver = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("the home side has outfielders");
    let passer = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && p.id != receiver
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("the home side has more than one outfielder");

    // Left attacks the right-hand goal, so "beyond" is larger x. He is on
    // 700; everybody else is well behind him, so the second-last defender
    // is nowhere near.
    let reception = Vector3::new(700.0, 300.0, 0.30);
    for player in field.players.iter_mut() {
        player.position = match player.id {
            id if id == receiver => Vector3::new(reception.x, reception.y, 0.0),
            id if id == passer => Vector3::new(400.0, 300.0, 0.0),
            // The defence, and the man who will end up taking the free
            // kick: near enough to walk, far enough that a teleport shows.
            _ if player.side == Some(PlayerSide::Right) => Vector3::new(600.0, 260.0, 0.0),
            _ => Vector3::new(380.0, 200.0, 0.0),
        };
    }

    field.ball.position = reception;
    field.ball.velocity = Vector3::new(1.2, 0.0, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(passer);
    field.ball.pass_target_player_id = Some(receiver);
    // The claim that carries the offside check only runs for a LIVE pass
    // — see the `in_flight_state` gate at the top of `process_ownership`.
    field.ball.flags.in_flight_state = 30;
    field.ball.pass_origin_restart = PassOriginRestart::OpenPlay;
    // Where he WAS when it was played — 20 m back up the pitch, beyond a
    // line at 520 with the ball at 400. That spot is the one the old restart
    // used, and the distance the ball used to be dragged.
    field.ball.offside_snapshot = OffsideSnapshot::at_kick(
        PassOriginRestart::OpenPlay,
        passer,
        PlayerSide::Left,
        400.0,
        520.0,
        420.0,
        [(receiver, reception.x - RUN_LENGTH)].into_iter(),
        context.current_tick(),
    );
    assert!(field.ball.offside_snapshot.is_some());
    (receiver, reception)
}

/// **The flag does not move the ball.**
#[test]
fn the_free_kick_is_where_the_offence_was_and_the_ball_does_not_travel_to_it() {
    let (mut field, mut context) = kickoff();
    let (receiver, reception) = a_through_ball_about_to_be_flagged(&mut field, &context);

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    field.ball.update_light(&mut context, &players, &mut events);

    let moved = (field.ball.position - reception).magnitude();
    assert!(
        moved < 4.0,
        "the flag moved the ball {moved:.1} units — it is dead where the offence happened, \
         and the old restart dragged it {RUN_LENGTH} back to where the run started"
    );
    let waiting = field
        .ball
        .awaiting_restart
        .expect("the offside must set an awaited restart up");
    assert_eq!(waiting.origin, PassOriginRestart::IndirectFreeKick);
    assert_ne!(
        waiting.taker_id, receiver,
        "the offside player cannot take his own free kick"
    );
    assert!(
        (waiting.spot - reception).magnitude() < 4.0,
        "the restart spot must be where the offence was"
    );
}

/// **And it does not move the defender either.**
///
/// `handle_offside_event` used to stage `pending_set_piece_teleport` so the
/// nearest opponent — routinely tens of metres away — appeared on the spot.
/// He walks now, on the same `AwaitedRestart` the throw-in uses, and the
/// teleport survives only as the patience timeout.
#[test]
fn the_taker_walks_to_the_offside_free_kick() {
    let (mut field, mut context) = kickoff();
    a_through_ball_about_to_be_flagged(&mut field, &context);

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    field.ball.update_light(&mut context, &players, &mut events);

    let waiting = field
        .ball
        .awaiting_restart
        .expect("the offside must set an awaited restart up");
    let taker_start = players
        .iter()
        .find(|p| p.id == waiting.taker_id)
        .map(|p| p.position)
        .expect("the taker is on the pitch");
    assert!(
        (taker_start - waiting.spot).magnitude() > AwaitedRestart::REACH,
        "the fixture must place the taker far enough away for a teleport to be visible"
    );
    assert!(
        field.ball.pending_set_piece_teleport.is_none(),
        "the taker was teleported onto the spot instead of walking to it"
    );
    assert!(
        field.ball.current_owner.is_none(),
        "the ball is out of play until he gets there"
    );

    // …and it stays that way while he is on his way. He is not moved by
    // this test, so the ball has to sit there until the patience bound.
    for _ in 0..40 {
        context.total_match_time += 10;
        field.ball.update_light(&mut context, &players, &mut events);
        assert!(
            field.ball.pending_set_piece_teleport.is_none(),
            "the taker was teleported while the ball was still waiting"
        );
        assert!(field.ball.current_owner.is_none());
    }
}

/// Two Left attackers when a team-mate plays the ball from 400: one onside
/// on 500, one beyond a line on 520 on 700. Returns `(onside, offside)`.
fn two_runners_one_offside(field: &mut MatchField, context: &MatchContext) -> (u32, u32) {
    let mut attackers = field.players.iter().filter(|p| {
        p.side == Some(PlayerSide::Left) && !p.tactical_position.current_position.is_goalkeeper()
    });
    let passer = attackers.next().unwrap().id;
    let onside = attackers.next().unwrap().id;
    let offside = attackers.next().unwrap().id;
    field.ball.offside_snapshot = OffsideSnapshot::at_kick(
        PassOriginRestart::OpenPlay,
        passer,
        PlayerSide::Left,
        400.0,
        520.0,
        420.0,
        [(onside, 500.0), (offside, 700.0)].into_iter(),
        context.current_tick(),
    );
    (onside, offside)
}

/// `player` has just gained the ball where he stands.
fn gains_it(field: &mut MatchField, context: &mut MatchContext, player: u32) {
    let at = field.get_player(player).unwrap().position;
    field.ball.position = Vector3::new(at.x, at.y, 0.1);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(player);
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    field.ball.update_light(context, &players, &mut events);
}

#[test]
fn whoever_was_offside_is_flagged_not_only_the_intended_man() {
    let (mut field, mut context) = kickoff();
    let (_, offside) = two_runners_one_offside(&mut field, &context);
    gains_it(&mut field, &mut context, offside);
    let restart = field.ball.awaiting_restart.expect("flagged");
    assert_eq!(restart.origin, PassOriginRestart::IndirectFreeKick);
}

#[test]
fn a_long_ball_in_the_air_is_still_judged_from_the_kick() {
    let (mut field, mut context) = kickoff();
    let (_, offside) = two_runners_one_offside(&mut field, &context);
    // Four seconds later — the old snapshot was gone after 2.2.
    context.total_match_time += 4_000;
    gains_it(&mut field, &mut context, offside);
    assert!(field.ball.awaiting_restart.is_some());
}

#[test]
fn a_rebound_off_the_keeper_to_the_offside_man_is_offside() {
    let (mut field, mut context) = kickoff();
    let (_, offside) = two_runners_one_offside(&mut field, &context);
    let keeper = field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Right) && p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .unwrap();
    let tick = context.current_tick();
    field.ball.record_touch(keeper, 2, tick, false);
    gains_it(&mut field, &mut context, offside);
    assert!(field.ball.awaiting_restart.is_some());
}

#[test]
fn a_defender_playing_it_deliberately_ends_the_offside() {
    let (mut field, mut context) = kickoff();
    let (_, offside) = two_runners_one_offside(&mut field, &context);
    let defender = field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Right) && !p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .unwrap();
    gains_it(&mut field, &mut context, defender);
    assert!(field.ball.offside_snapshot.is_none());
    gains_it(&mut field, &mut context, offside);
    assert!(field.ball.awaiting_restart.is_none());
}

#[test]
fn the_onside_man_gaining_it_is_play_on() {
    let (mut field, mut context) = kickoff();
    let (onside, _) = two_runners_one_offside(&mut field, &context);
    gains_it(&mut field, &mut context, onside);
    assert!(field.ball.awaiting_restart.is_none());
    assert!(field.ball.offside_snapshot.is_none());
}
