//! **Taking a pass that comes past you.**
//!
//! Reported as: *defenders fail to intercept passes — the ball simply
//! gets past them.* It did, by construction. A pass got ONE interception
//! roll, latched on the first tick anybody came within 5.5u of the ball,
//! and booked over 100 fixtures that tick was the kick itself: the roll
//! went to the man standing on the passer (mean 0.52 m away, p=0.046),
//! and the defender the ball then ran through downstream never got one.
//!
//! The contest is now per man, at his closest approach. These tests fly
//! a real ball past real players and ask the questions the report asks:
//! is the roll made where the ball is level with him, does a ball
//! through his feet usually stop there, does a stretch cost him, does
//! the man behind the ball get nothing, and does the second man on the
//! line get his own go when the first misses.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::result::Score;
use crate::r#match::events::EventCollection;
use crate::r#match::{Ball, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection};
use nalgebra::Vector3;

const PASSER: u32 = 105;
const RECEIVER: u32 = 106;
const DEFENDER: u32 = 205;
const SECOND: u32 = 206;

/// A pass along +x at `speed`, struck `since_strike` ticks ago, with
/// everybody parked in a corner except the men `place` moves.
fn flight(
    speed: f32,
    since_strike: u64,
    place: impl Fn(&mut MatchPlayer),
) -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    for p in field.players.iter_mut() {
        p.position = Vector3::new(5.0, 5.0, 0.0);
        place(p);
    }
    let ball: &mut Ball = &mut field.ball;
    ball.position = Vector3::new(300.0, 272.0, 0.0);
    ball.velocity = Vector3::new(speed, 0.0, 0.0);
    ball.previous_owner = Some(PASSER);
    ball.pass_target_player_id = Some(RECEIVER);
    ball.flags.in_flight_state = 400;
    ball.last_release_tick = 1000;
    ball.current_tick_cached = 1000 + since_strike;
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    (field, context)
}

/// Fly the ball until it is past `until_x` or somebody has it. Returns
/// the ball's x when the flight ended and who ended it.
fn fly(field: &mut MatchField, context: &MatchContext, until_x: f32) -> (f32, Option<u32>) {
    let mut events = EventCollection::new();
    while field.ball.position.x < until_x && field.ball.current_owner.is_none() {
        field
            .ball
            .try_intercept(context, &field.players, &mut events);
        if field.ball.current_owner.is_some() {
            break;
        }
        let v = field.ball.velocity;
        field.ball.position += v;
        field.ball.current_tick_cached += 1;
    }
    (field.ball.position.x, field.ball.current_owner)
}

fn slot_of(field: &MatchField, id: u32) -> u64 {
    1u64 << field.players.iter().position(|p| p.id == id).unwrap()
}

/// The roll is made where the ball is LEVEL with him, not at the far edge
/// of his reach. The old latch fired the moment anybody came within 5.5u,
/// which for a man in the lane is 5.5u short of him — proximity 0.3 — and
/// that was the pass's only roll.
#[test]
fn the_roll_is_made_where_the_ball_draws_level_with_him() {
    for _ in 0..20 {
        let (mut field, context) = flight(1.0, 100, |p| {
            if p.id == DEFENDER {
                p.position = Vector3::new(340.0, 272.0, 0.0);
            }
        });
        let bit = slot_of(&field, DEFENDER);
        let mut events = EventCollection::new();
        let mut rolled_at = None;
        while field.ball.position.x < 360.0 {
            field
                .ball
                .try_intercept(&context, &field.players, &mut events);
            if field.ball.intercept_rolled & bit != 0 {
                rolled_at = Some(field.ball.position.x);
                break;
            }
            field.ball.position.x += 1.0;
            field.ball.current_tick_cached += 1;
        }
        let x = rolled_at.expect("a man on the line is rolled for");
        assert!(
            (339.0..=340.5).contains(&x),
            "rolled at x={x}, he stands at 340"
        );
    }
}

/// A ball played straight through a reader's feet is often stopped
/// there; one he has to stretch a metre and a quarter for rarely is.
/// The bands are wide because the levels are calibration (see
/// `InterceptionContest::GAIN`) — the ORDERING is the claim.
#[test]
fn through_his_feet_is_often_taken_and_a_stretch_rarely_is() {
    let taken = |offset: f32| {
        let mut count = 0;
        for _ in 0..300 {
            let (mut field, context) = flight(1.0, 100, |p| {
                if p.id == DEFENDER {
                    p.position = Vector3::new(340.0, 272.0 + offset, 0.0);
                }
            });
            if fly(&mut field, &context, 380.0).1 == Some(DEFENDER) {
                count += 1;
            }
        }
        count
    };
    let feet = taken(0.0);
    let stretch = taken(10.0);
    assert!((50..=220).contains(&feet), "through his feet: {feet}/300");
    assert!(
        (0..=80).contains(&stretch),
        "a 1.25 m stretch: {stretch}/300"
    );
    assert!(feet > stretch * 2, "{feet} vs {stretch}");
}

/// A driven ball is harder to take than a rolled one. Asserted as a
/// RATIO: the pace term is a multiplier, so an absolute gap between the
/// two counts moves with `InterceptionContest::GAIN` and a test written
/// that way fails the next time the rate is calibrated.
#[test]
fn a_driven_ball_is_harder_to_take() {
    let taken = |speed: f32| {
        let mut count = 0;
        for _ in 0..400 {
            let (mut field, context) = flight(speed, 100, |p| {
                if p.id == DEFENDER {
                    p.position = Vector3::new(340.0, 272.0, 0.0);
                }
            });
            if fly(&mut field, &context, 380.0).1 == Some(DEFENDER) {
                count += 1;
            }
        }
        count
    };
    let rolled = taken(0.4);
    let driven = taken(3.0);
    assert!(
        rolled as f32 > driven as f32 * 1.3,
        "rolled {rolled} vs driven {driven} of 400"
    );
}

/// The man the ball is played AWAY from — the presser standing on the
/// passer as he strikes it — never crosses it, so never rolls. This was
/// the man the old latch spent the pass's one roll on.
#[test]
fn the_man_behind_the_ball_gets_nothing() {
    for _ in 0..50 {
        let (mut field, context) = flight(1.0, 0, |p| {
            if p.id == DEFENDER {
                p.position = Vector3::new(297.0, 273.0, 0.0);
            }
        });
        let (_, owner) = fly(&mut field, &context, 340.0);
        assert_eq!(owner, None);
        assert_eq!(field.ball.intercept_rolled, 0, "nobody was rolled for");
    }
}

/// A ball that has only just left the boot is past the man beside the
/// passer before he can move: the roll is made, at almost nothing.
#[test]
fn a_ball_just_struck_is_gone_before_the_man_beside_the_passer_moves() {
    let mut count = 0;
    for _ in 0..300 {
        let (mut field, context) = flight(1.0, 0, |p| {
            if p.id == DEFENDER {
                p.position = Vector3::new(304.0, 274.0, 0.0);
            }
        });
        if fly(&mut field, &context, 340.0).1 == Some(DEFENDER) {
            count += 1;
        }
    }
    assert!(
        count <= 45,
        "taken {count}/300 within 4 ticks of the strike"
    );
}

/// Two men on the line: when the first misses, the second gets his own
/// roll. Under the old latch the second man never existed.
#[test]
fn every_man_the_ball_passes_gets_his_own_roll() {
    let mut reached_second = 0;
    for _ in 0..60 {
        let (mut field, context) = flight(1.0, 100, |p| {
            if p.id == DEFENDER {
                p.position = Vector3::new(340.0, 272.0, 0.0);
            } else if p.id == SECOND {
                p.position = Vector3::new(380.0, 272.0, 0.0);
            }
        });
        let first = slot_of(&field, DEFENDER);
        let second = slot_of(&field, SECOND);
        let (x, owner) = fly(&mut field, &context, 400.0);
        let mask = field.ball.intercept_rolled;
        if x > 341.0 {
            assert!(mask & first != 0, "the ball passed the first man unrolled");
        }
        if x > 381.0 {
            assert!(
                mask & second != 0,
                "the ball passed the second man unrolled"
            );
            reached_second += 1;
        }
        if let Some(id) = owner {
            assert!(id == DEFENDER || id == SECOND);
        }
    }
    assert!(
        reached_second > 0,
        "the first man never missed in 60 flights"
    );
}
