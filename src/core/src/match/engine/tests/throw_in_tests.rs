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
use crate::r#match::engine::ball::ball::AwaitedRestart;
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

    // Nobody moves. Run past the patience bound.
    let mut events = EventCollection::with_capacity(4);
    for _ in 0..(AwaitedRestart::PATIENCE_TICKS + 2) {
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
        Some((awaited.taker_id, awaited.spot)),
        "and the backstop is the placement it always was"
    );
}

/// The ball does not move while it is out of play, and nothing may take it
/// off the man it was awarded to. Both were true of the old restart only
/// because it lasted a single tick.
#[test]
fn an_out_of_play_ball_neither_moves_nor_changes_hands() {
    let (mut field, mut context) = kickoff();
    let toucher = outfielder(&field, PlayerSide::Left);
    put_it_out(&mut field, &context, toucher, None);
    let spot = field.ball.awaiting_restart.expect("armed").spot;

    let mut events = EventCollection::with_capacity(4);
    for _ in 0..60 {
        context.increment_time();
        let players = field.players.clone();
        field.ball.update_light(&mut context, &players, &mut events);
        assert!(
            (field.ball.position - spot).magnitude() < 1.0e-3,
            "the ball drifted off the line while out of play"
        );
        assert!(
            field.ball.current_owner.is_none(),
            "somebody claimed a ball that is out of play"
        );
    }
}
