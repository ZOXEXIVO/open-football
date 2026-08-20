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
    field.ball.offside_snapshot = Some(OffsideSnapshot {
        origin: PassOriginRestart::OpenPlay,
        passer_id: passer,
        passer_side: PlayerSide::Left,
        receiver_id: receiver,
        ball_x_at_kick: 400.0,
        second_last_defender_x: 520.0,
        // Where he WAS when it was played — 20 m back up the pitch. This is
        // the spot the old restart used, and the distance the ball used to
        // be dragged.
        receiver_x_at_kick: reception.x - RUN_LENGTH,
        receiver_y_at_kick: reception.y,
        set_tick: context.current_tick(),
    });
    (receiver, reception)
}

/// **The flag does not move the ball.**
#[test]
fn the_free_kick_is_where_the_offence_was_and_the_ball_does_not_travel_to_it() {
    let (mut field, mut context) = kickoff();
    let (receiver, reception) = a_through_ball_about_to_be_flagged(&mut field, &mut context);

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
    a_through_ball_about_to_be_flagged(&mut field, &mut context);

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
