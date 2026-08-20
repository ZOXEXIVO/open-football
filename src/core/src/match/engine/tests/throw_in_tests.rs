//! The touchline restart, end to end.
//!
//! Two things are being pinned here and they are different in kind.
//!
//! **Who gets it** is a rule, and rules get tested by construction: the
//! side that did NOT put the ball out throws it in. The engine decides
//! that from `Ball::last_touch_player_id`, which every path that plays the
//! ball is supposed to stamp — so the interesting cases are the ones where
//! a touch and an ownership disagree, and those are the ones below.
//!
//! **How it is taken** is behaviour, and the bug it exists to catch is a
//! visual one: the taker used to be TELEPORTED onto the ball, measured at
//! 100% of throw-ins and a mean of 21.5 m, so a player materialised on the
//! touchline every forty seconds of watched football. The assertions are
//! therefore about where he is over TIME — a test on the final state alone
//! would pass against the teleport.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{AwaitedRestart, RunOff};
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

/// An outfielder of `side`, for staging a touch.
fn outfielder(field: &MatchField, side: PlayerSide) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.side == Some(side) && !p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .expect("a side has outfielders")
}

/// Roll the ball over the top touchline with `toucher` as the last man on
/// it, and run the restart check the engine runs.
fn put_it_out(field: &mut MatchField, context: &MatchContext, toucher: u32, owner: Option<u32>) {
    let team = field
        .players
        .iter()
        .find(|p| p.id == toucher)
        .map(|p| p.team_id)
        .unwrap();
    field.ball.position = Vector3::new(420.0, -1.0, 0.0);
    field.ball.velocity = Vector3::new(0.0, -1.4, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = owner;
    field
        .ball
        .record_touch(toucher, team, context.current_tick(), true);

    let mut events = EventCollection::with_capacity(4);
    let players = field.players.clone();
    field.ball.check_throw_in(context, &players, &mut events);
}

/// Whose throw is it?
fn awarded_to(field: &MatchField) -> Option<PlayerSide> {
    let taker = field.ball.awaiting_restart?.taker_id;
    field
        .players
        .iter()
        .find(|p| p.id == taker)
        .and_then(|p| p.side)
}

/// **The rule.** Whoever put it out does not get it back.
#[test]
fn the_throw_goes_to_the_side_that_did_not_put_it_out() {
    for (out_by, expected) in [
        (PlayerSide::Left, PlayerSide::Right),
        (PlayerSide::Right, PlayerSide::Left),
    ] {
        let (mut field, context) = kickoff();
        let toucher = outfielder(&field, out_by);
        put_it_out(&mut field, &context, toucher, None);
        assert_eq!(
            awarded_to(&field),
            Some(expected),
            "{out_by:?} put it out, so {expected:?} throws it in"
        );
    }
}

/// **The deflection.** A defender who blocks a cross out concedes the
/// throw, even though the ball was the attacking side's a moment earlier —
/// `previous_owner` still names the crosser and `last_touch_player_id` is
/// the only field that knows what actually happened last. This is the same
/// distinction the corner-vs-goal-kick resolver documents, and getting it
/// backwards is precisely "the throw-in is for the team that put it out".
#[test]
fn a_deflection_off_the_defender_is_the_attackers_throw() {
    let (mut field, context) = kickoff();
    let attacker = outfielder(&field, PlayerSide::Right);
    let defender = outfielder(&field, PlayerSide::Left);
    // The attacking side had it; the DEFENDER got the last touch.
    put_it_out(&mut field, &context, defender, Some(attacker));
    assert_eq!(
        awarded_to(&field),
        Some(PlayerSide::Right),
        "the defender deflected it out — the throw is the attackers'"
    );
}

/// **The taker walks.** The ball waits on the line, unowned, while he runs
/// to it; he does not appear beside it.
///
/// Asserted as a sequence rather than an end state, because a teleport
/// produces the same end state one tick later — which is exactly how the
/// original behaviour passed for as long as it did.
#[test]
fn the_ball_waits_on_the_line_and_nobody_owns_it() {
    let (mut field, context) = kickoff();
    let toucher = outfielder(&field, PlayerSide::Left);
    put_it_out(&mut field, &context, toucher, None);

    let awaited = field
        .ball
        .awaiting_restart
        .expect("a throw-in is a restart somebody has to come and take");
    assert!(
        field.ball.current_owner.is_none(),
        "an out-of-play ball belongs to nobody until it is picked up"
    );

    let taker = field
        .players
        .iter()
        .find(|p| p.id == awaited.taker_id)
        .expect("the taker is on the pitch");
    assert!(
        (taker.position - awaited.spot).magnitude() > AwaitedRestart::REACH,
        "the taker was placed ON the ball — that is the teleport this \
         restart exists to remove"
    );
    assert_eq!(
        taker.side,
        Some(PlayerSide::Right),
        "and he is one of the side that gets the throw"
    );
}

/// …but a restart that never happens would stall the match, so the walk is
/// bounded. Past the patience window the taker is placed, exactly as he
/// always was — the teleport survives as the backstop, not the behaviour.
#[test]
fn a_taker_who_never_arrives_is_placed_rather_than_waited_for() {
    let (mut field, mut context) = kickoff();
    let toucher = outfielder(&field, PlayerSide::Left);
    put_it_out(&mut field, &context, toucher, None);
    let awaited = field.ball.awaiting_restart.expect("armed");
    // The throw is taken from the touchline whatever the ball does next —
    // the backstop consumes `take_from` first, so this is the placement,
    // not the run-out's resting place.
    let throw_from = awaited
        .take_from
        .expect("a throw-in is taken from where the ball crossed the line");

    // Nobody moves. Run past the patience bound — which is no longer a
    // constant: the ball spends the first fraction of a second running out
    // of play, and `tick_run_out` re-derives the wait against the distance
    // the taker is left with once it stops.
    let mut events = EventCollection::with_capacity(4);
    let bound = AwaitedRestart::CEILING + AwaitedRestart::PATIENCE_TICKS;
    for _ in 0..bound {
        if field.ball.awaiting_restart.is_none() {
            break;
        }
        context.increment_time();
        let players = field.players.clone();
        field
            .ball
            .tick_awaited_restart(&context, &players, &mut events);
    }

    assert!(
        field.ball.awaiting_restart.is_none(),
        "the restart must resolve rather than hold the match up"
    );
    assert_eq!(field.ball.current_owner, Some(awaited.taker_id));
    assert_eq!(
        field.ball.pending_set_piece_teleport,
        Some((awaited.taker_id, throw_from)),
        "and the backstop is the placement it always was — on the line, \
         not out in the run-off where the ball came to rest"
    );
}

/// **The ball goes OUT, comes to rest out there, and then does not move
/// again** — and nothing may take it off the man it was awarded to at any
/// point.
///
/// The reported artefact is in the first tick: the award used to write the
/// ball 2 u back INSIDE the touchline and zero its velocity, so a ball
/// that had gone out of play stopped dead on the line. It now keeps the
/// pace it crossed with, runs across the run-off, and is stopped by the
/// hoardings — which is where a real one goes and where the thrower has to
/// go and get it.
#[test]
fn a_ball_put_out_runs_off_the_pitch_and_stays_there() {
    let (mut field, mut context) = kickoff();
    let toucher = outfielder(&field, PlayerSide::Left);
    put_it_out(&mut field, &context, toucher, None);

    let mut events = EventCollection::with_capacity(4);
    let mut furthest = field.ball.position.y;
    let mut settled_at: Option<Vector3<f32>> = None;
    for _ in 0..300 {
        let before = field.ball.position;
        context.increment_time();
        let players = field.players.clone();
        field.ball.update_light(&mut context, &players, &mut events);
        let step = (field.ball.position - before).magnitude();
        assert!(
            step < 3.0,
            "the ball jumped {step:.1}u in one tick — a run-out is travel, not a teleport"
        );
        assert!(
            field.ball.current_owner.is_none(),
            "somebody claimed a ball that is out of play"
        );
        furthest = furthest.min(field.ball.position.y);
        if let Some(rest) = settled_at {
            assert!(
                (field.ball.position - rest).magnitude() < 1.0e-3,
                "the ball moved again after coming to rest"
            );
        } else if field
            .ball
            .awaiting_restart
            .is_some_and(|restart| restart.settled)
        {
            settled_at = Some(field.ball.position);
        }
    }

    let rest = settled_at.expect("the run-out has to finish inside three seconds");
    assert!(
        rest.y < 0.0,
        "the ball has to end up OFF the pitch, got y={:.1}",
        rest.y
    );
    assert!(
        furthest >= -RunOff::SIDE - 1.0e-3,
        "and the hoardings have to stop it, got y={furthest:.1} against a \
         perimeter at {:.1}",
        -RunOff::SIDE
    );
}

/// …while the throw itself is still taken from the touchline, where the
/// ball crossed. Law 15, and the reason the restart carries two points
/// rather than one.
#[test]
fn the_throw_is_still_taken_from_the_point_it_crossed() {
    let (mut field, context) = kickoff();
    let toucher = outfielder(&field, PlayerSide::Left);
    put_it_out(&mut field, &context, toucher, None);

    let take_from = field
        .ball
        .awaiting_restart
        .expect("armed")
        .take_from
        .expect("a run-out restart is taken from the crossing point");
    assert!(
        take_from.y > 0.0 && take_from.y < 8.0,
        "the throw is taken on the touchline, got y={:.1}",
        take_from.y
    );
    assert!(
        (take_from.x - 420.0).abs() < 8.0,
        "and at the point the ball crossed it, got x={:.1} against 420",
        take_from.x
    );
}
