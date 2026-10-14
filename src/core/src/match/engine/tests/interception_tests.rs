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
use super::substitution_break_tests::kickoff;
use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::ball::ball::contest::pass_block::{
    BlockGeometry, PassBlock, PassBlockCommit, StrikeLine,
};
use crate::r#match::engine::ball::ball::{GROUND_FRICTION, LOOSE_CLAIM_DISTANCE};
use crate::r#match::engine::flow::rng::MatchRng;
use crate::r#match::engine::result::Score;
use crate::r#match::events::{Event, EventCollection};
use crate::r#match::player::strategies::passing::{CrossModel, CrossType, PassEvaluator};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    Ball, GameTickContext, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection,
    MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    StateProcessor,
};
use nalgebra::Vector3;
use std::cell::Cell;

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

/// Step the ball a unit at a time through the interception contest and
/// then the receiver's claim, in the order a tick runs them, until
/// somebody has it or it is past `until_x`.
fn fly_to_the_receiver(field: &mut MatchField, context: &MatchContext, until_x: f32) {
    let mut events = EventCollection::new();
    while field.ball.position.x < until_x && field.ball.current_owner.is_none() {
        field
            .ball
            .try_intercept(context, &field.players, &mut events);
        if field.ball.current_owner.is_none() {
            field
                .ball
                .process_ownership(context, &field.players, &mut events);
        }
        if field.ball.current_owner.is_some() {
            break;
        }
        field.ball.position.x += 1.0;
        field.ball.current_tick_cached += 1;
    }
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
///
/// The ORDERING is the claim. The bands only bound the shape — that it
/// is a real chance and not a certainty — because the LEVEL belongs to
/// [`InterceptionContest::GAIN`], which is titrated on the level sweep
/// and moves whenever the contest gains or loses an encounter. Pinned
/// tightly, this test fails every re-titration and says nothing about
/// the football; that is the defect the next test's doc names.
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
    assert!((25..=250).contains(&feet), "through his feet: {feet}/300");
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

/// **The last stride and a half is a contest, not a gift.**
///
/// The receiver takes the ball where his foot meets it, so a defender
/// standing on the line a stride in front of him is reached first and
/// contested — with the receiver's own claim running in the same tick.
/// When the receiver was given the ball a stride and a half out, the
/// defender's crossing was never reached at all.
#[test]
fn a_defender_inside_the_receivers_last_stride_still_contests() {
    let mut rolled = 0;
    for _ in 0..40 {
        // Receiver a stride past the defender, both on the line.
        let (mut field, context) = flight(1.0, 100, |p| {
            if p.id == DEFENDER {
                p.position = Vector3::new(340.0, 272.0, 0.0);
            } else if p.id == RECEIVER {
                p.position = Vector3::new(346.0, 272.0, 0.0);
            }
        });
        let bit = slot_of(&field, DEFENDER);
        fly_to_the_receiver(&mut field, &context, 350.0);
        if field.ball.intercept_rolled & bit != 0 {
            rolled += 1;
        }
    }
    assert_eq!(
        rolled, 40,
        "the man the ball was rolled through was never contested"
    );
}

/// …and the receiver still keeps what is genuinely his: a defender
/// FURTHER from the ball than the man it is going to does not get to reach
/// through him for it, and the receiver has it where his foot meets it.
#[test]
fn the_receiver_keeps_the_ball_a_trailing_defender_cannot_reach_first() {
    for seed in 0..20u64 {
        let (mut field, mut context) = flight(1.0, 100, |p| {
            if p.id == DEFENDER {
                // On the line, but a full reach off the ball when it arrives.
                p.position = Vector3::new(346.0, 279.0, 0.0);
            } else if p.id == RECEIVER {
                p.position = Vector3::new(347.0, 272.0, 0.0);
            }
        });
        context.rng = MatchRng::from_seed(seed);
        let bit = slot_of(&field, DEFENDER);
        fly_to_the_receiver(&mut field, &context, 350.0);
        assert_eq!(
            field.ball.intercept_rolled & bit,
            0,
            "a defender further from the ball than its receiver took the contest"
        );
        if field.ball.current_owner == Some(RECEIVER) {
            assert!(
                347.0 - field.ball.position.x <= LOOSE_CLAIM_DISTANCE,
                "the receiver was given the ball {:.1}u short of his feet",
                347.0 - field.ball.position.x
            );
        }
    }
}

/// Fly a pass through `update_light` until it has an owner, a failed
/// first touch or `ticks` have gone. Every tick before then is checked for
/// a ball nobody has touched: on its own line, slowed by friction alone.
/// Returns the tick's ball state before the contact and the events of the
/// tick it happened on, if it did.
fn fly_untouched(
    field: &mut MatchField,
    context: &mut MatchContext,
    ticks: usize,
) -> Option<(Vector3<f32>, Vec<Event>)> {
    let players = field.players.clone();
    for tick in 0..ticks {
        let at = field.ball.position;
        let pace = field.ball.velocity.x.hypot(field.ball.velocity.y);
        let mut events = EventCollection::new();
        field.ball.update_light(context, &players, &mut events);
        let events: Vec<Event> = events.drain().collect();
        let touched = field.ball.current_owner.is_some()
            || events
                .iter()
                .any(|e| matches!(e, Event::BallEvent(BallEvent::FirstTouchFailed(..))));
        if touched {
            return Some((at, events));
        }
        let now = field.ball.velocity.x.hypot(field.ball.velocity.y);
        assert_eq!(
            field.ball.velocity.y, 0.0,
            "tick {tick}: the delivery was turned off its line"
        );
        assert!(
            (now - pace * (1.0 - GROUND_FRICTION)).abs() < 1e-4,
            "tick {tick}: the delivery's pace went {pace:.4} -> {now:.4}, more than friction"
        );
    }
    None
}

/// **A pass through his feet is untouched until it reaches him.** Nothing
/// steers a delivery because its man is near: it keeps its own pace and
/// line until it is within a stride of him, and on that tick it is his,
/// credited, and moving at his pace — or, off a bad touch, loose from
/// where it was met.
#[test]
fn a_pass_through_his_feet_keeps_its_pace_and_line_until_he_meets_it() {
    for seed in 0..12u64 {
        let (mut field, mut context) = flight(0.8, 100, |p| {
            if p.id == RECEIVER {
                p.position = Vector3::new(346.0, 272.0, 0.0);
                p.velocity = Vector3::zeros();
            }
        });
        context.rng = MatchRng::from_seed(seed);
        let (met, events) =
            fly_untouched(&mut field, &mut context, 120).expect("the pass never reached him");
        let gap = (346.0 - met.x).hypot(272.0 - met.y);
        assert!(
            gap <= LOOSE_CLAIM_DISTANCE,
            "the pass was taken {gap:.1}u from him"
        );
        if field.ball.current_owner == Some(RECEIVER) {
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    Event::BallEvent(BallEvent::PassCompleted(RECEIVER, PASSER))
                )),
                "his on the contact tick but not credited"
            );
            let pace = field.ball.velocity.x.hypot(field.ball.velocity.y);
            assert!(
                pace <= 0.15,
                "a standing man's touch left the ball at {pace:.2} u/tick"
            );
        } else {
            let off = field.ball.position - field.ball.velocity;
            assert!(
                (off - met).xy().norm() < 1e-3,
                "the failed touch ran on from {off:?}, not from where it was met ({met:?})"
            );
        }
    }
}

/// **A pass beside him is not his until he gets to it.** A delivery whose
/// line runs a stride and a half from a man standing still is never turned
/// toward him and never credited: it runs on, and he has to go and get it.
#[test]
fn a_pass_beside_him_is_never_turned_and_never_credited() {
    let (mut field, mut context) = flight(0.8, 100, |p| {
        if p.id == RECEIVER {
            p.position = Vector3::new(346.0, 279.0, 0.0);
            p.velocity = Vector3::zeros();
        }
    });
    assert!(
        fly_untouched(&mut field, &mut context, 100).is_none(),
        "a pass seven units from a man standing still was taken by him"
    );
    assert!(field.ball.position.x > 346.0, "the pass never got past him");
}

/// **An interception on the run leaves with the man who made it.** A
/// defender crossing the lane at 5 m/s takes the pass at his own pace: on
/// the next tick the ball is moving with him, neither lying where it was
/// met nor running on along the pass.
#[test]
fn an_interception_on_the_run_carries_the_ball_away_with_him() {
    let across = Vector3::new(0.0, 0.4, 0.0);
    for _ in 0..300 {
        let (mut field, context) = flight(1.0, 100, |p| {
            if p.id == DEFENDER {
                p.position = Vector3::new(340.0, 271.0, 0.0);
                p.velocity = across;
            }
        });
        let (_, owner) = fly(&mut field, &context, 345.0);
        if owner != Some(DEFENDER) {
            continue;
        }
        let players = field.players.clone();
        field.ball.move_to_with_players(&players);
        let v = field.ball.velocity;
        assert!(
            (v.x - across.x).abs() <= Ball::CARRY_BOUND + 1e-4
                && (v.y - across.y).abs() <= Ball::CARRY_BOUND + 1e-4,
            "the intercepted ball moves at {v:?}, not with him"
        );
        return;
    }
    panic!("a defender crossing the lane at the ball never intercepted it");
}

/// **One flight, one interception roll per man** — even while the block
/// channel is also running. The two are separate ledgers: sharing one
/// lets the block candidacy window, which reaches 40u up the lane,
/// consume a man's encounter long before the ball gets to his feet.
#[test]
fn the_block_channel_does_not_consume_his_interception() {
    let (mut field, context) = flight(1.0, 100, |p| {
        if p.id == DEFENDER {
            p.position = Vector3::new(340.0, 272.0, 0.0);
        }
    });
    let bit = slot_of(&field, DEFENDER);
    let mut events = EventCollection::new();
    while field.ball.position.x < 400.0 && field.ball.current_owner.is_none() {
        field
            .ball
            .try_block_pass(&context, &field.players, &mut events);
        field
            .ball
            .try_intercept(&context, &field.players, &mut events);
        field.ball.position.x += 1.0;
        field.ball.current_tick_cached += 1;
    }
    assert!(
        field.ball.intercept_rolled & bit != 0,
        "the block channel swallowed his interception roll"
    );
}

/// **The passer prices the leg as well as the take.** A man beside the
/// line, too far off it to take the ball but inside a lunge, is a risk
/// the pass runs (`try_block_pass`), and the passer has to see it — at
/// exactly the chance the contest will roll for him.
#[test]
fn the_passer_prices_a_block_he_cannot_intercept() {
    let price = |defender_y: f32| {
        let home = squad(1, 100);
        let away = squad(2, 200);
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let mut field = MatchField::new(840, 545, home, away);
        for p in field.players.iter_mut() {
            p.position = Vector3::new(5.0, 5.0, 0.0);
            match p.id {
                PASSER => p.position = Vector3::new(720.0, 272.0, 0.0),
                RECEIVER => p.position = Vector3::new(810.0, 272.0, 0.0),
                DEFENDER => p.position = Vector3::new(736.0, defender_y, 0.0),
                _ => {}
            }
        }
        let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
        let tick = GameTickContext::new(&field, &context.players);
        let passer = field.players.iter().find(|p| p.id == PASSER).unwrap();
        let ctx = StateProcessingContext {
            in_state_time: 0,
            player: passer,
            context: &context,
            tick_context: &tick,
        };
        let target = Vector3::new(810.0, 272.0, 0.0);
        let risk = PassEvaluator::lane_risk(&ctx, passer, target);
        let defender = field.players.iter().find(|p| p.id == DEFENDER).unwrap();
        let line = StrikeLine {
            from: passer.position,
            direction: Vector3::new(1.0, 0.0, 0.0),
            pace: Ball::pass_pace(90.0, &context.conditions),
            lift: 0.0,
            delivery: sc::passing_execution(passer, 0),
            defending_side: defender.side.unwrap(),
        };
        let lite = MatchPlayerLite {
            id: defender.id,
            position: defender.position,
            tactical_positions: defender.tactical_position.current_position,
        };
        let block = PassBlock::priced(&line, &lite, defender, 0, 840.0);
        (risk, block)
    };

    let (beside, block) = price(272.0 + 12.0);
    assert!(
        block > 0.0,
        "a man 1.5 m off the line inside his own third can lunge at it"
    );
    assert!(
        (beside - block).abs() < 1e-6,
        "the price must be the contest's own chance: {beside} vs {block}"
    );
    let (clear, _) = price(272.0 + 30.0);
    assert_eq!(clear, 0.0, "nobody within reach of the line is no risk");
}

/// **One chance per man per flight** — a man whose block has been rolled
/// for this flight is never committed again, however squarely the ball
/// is then struck into him.
#[test]
fn a_man_already_rolled_is_not_rolled_again() {
    let (mut field, context) = flight(1.6, 0, |p| {
        if p.id == DEFENDER {
            p.position = Vector3::new(310.0, 272.0, 0.0);
        }
    });
    field.ball.pass_block_rolled = slot_of(&field, DEFENDER);
    let mut events = EventCollection::new();
    while field.ball.position.x < 320.0 {
        field
            .ball
            .try_block_pass(&context, &field.players, &mut events);
        assert!(field.ball.pass_blocked_by.is_none());
        field.ball.position.x += 1.6;
        field.ball.current_tick_cached += 1;
    }
    assert_ne!(field.ball.last_touch_player_id, Some(DEFENDER));
}

/// A ball at 1.9 m level with the man who won the roll: chest-high to a
/// body it was struck into, over the leg of one who lunged.
fn contact_at_chest(reaction: f32) -> Option<u32> {
    let (mut field, context) = flight(1.6, 0, |p| {
        if p.id == DEFENDER {
            p.position = Vector3::new(299.5, 272.0, 0.0);
        }
    });
    field.ball.position.z = 1.9;
    field.ball.pass_blocked_by = Some(PassBlockCommit {
        blocker_id: DEFENDER,
        outcome_roll: 0.99,
        reaction,
    });
    let mut events = EventCollection::new();
    field
        .ball
        .try_block_pass(&context, &field.players, &mut events);
    field.ball.last_touch_player_id
}

#[test]
fn a_charge_down_meets_a_ball_a_lunge_cannot_reach() {
    assert_eq!(contact_at_chest(1.0), Some(DEFENDER));
    assert_ne!(contact_at_chest(0.0), Some(DEFENDER));
}

/// The ball's velocity after a won block resolved at the body, for a ball
/// travelling along `heading` through `at`, drawn `samples` times off one
/// match's stream.
fn deflections(at: Vector3<f32>, heading: Vector3<f32>, reaction: f32) -> Vec<Vector3<f32>> {
    let (mut field, context) = flight(1.6, 0, |_| {});
    (0..40)
        .map(|_| {
            for p in field.players.iter_mut() {
                if p.id == DEFENDER {
                    p.position = at - heading * 0.5;
                }
            }
            field.ball.position = at;
            field.ball.velocity = heading * 1.6;
            field.ball.current_owner = None;
            field.ball.flags.in_flight_state = 400;
            field.ball.previous_owner = Some(PASSER);
            field.ball.pass_blocked_by = Some(PassBlockCommit {
                blocker_id: DEFENDER,
                outcome_roll: 0.99,
                reaction,
            });
            let mut events = EventCollection::new();
            field
                .ball
                .try_block_pass(&context, &field.players, &mut events);
            assert_eq!(field.ball.last_touch_player_id, Some(DEFENDER));
            field.ball.velocity
        })
        .collect()
}

/// The defending side here defends the goal at x = 840. A ball charged
/// down on its way toward that goal line carries on toward it.
#[test]
fn a_ball_charged_down_keeps_heading_for_the_goal_line() {
    let heading = Vector3::new(0.8, 0.6, 0.0);
    for velocity in deflections(Vector3::new(800.0, 120.0, 0.0), heading, 1.0) {
        assert!(
            velocity.x > 0.0,
            "turned back off a body it hit: {velocity:?}"
        );
    }
}

/// A defender who had time to read the pass sends it away from his own
/// goal. Struck from 30 m out, past the depth where a block goes behind.
#[test]
fn a_read_block_is_sent_away_from_goal() {
    let heading = Vector3::new(1.0, 0.0, 0.0);
    for velocity in deflections(Vector3::new(600.0, 272.0, 0.0), heading, 0.0) {
        assert!(
            velocity.x < 0.0,
            "a read block came off toward his goal: {velocity:?}"
        );
    }
}

/// **The striker prices what the contest measures.** A ball launched
/// along the line a striker priced is, at the strike, the same geometry
/// to the contest as it was to him — so the two read one chance.
#[test]
fn the_striker_and_the_contest_measure_one_geometry() {
    let direction = Vector3::new(0.8, 0.6, 0.0);
    let across = Vector3::new(-0.6, 0.8, 0.0);
    let from = Vector3::new(700.0, 200.0, 0.0);
    let (mut field, _) = flight(1.6, 0, |p| {
        if p.id == DEFENDER {
            p.position = from + direction * 16.0 + across * 4.0;
        }
    });
    let (pace, lift) = (1.6, 0.06);
    let line = StrikeLine {
        from,
        direction,
        pace,
        lift,
        delivery: 0.6,
        defending_side: PlayerSide::Right,
    };
    field.ball.position = from;
    field.ball.velocity = Vector3::new(direction.x * pace, direction.y * pace, lift);
    field.ball.last_release_tick = field.ball.current_tick_cached;
    let defender = field.players.iter().find(|p| p.id == DEFENDER).unwrap();

    let priced = BlockGeometry::at_strike(&line, defender.position, 840.0).unwrap();
    let measured = BlockGeometry::in_flight(
        &field.ball,
        defender.position,
        pace,
        PassBlock::danger(from.x, PlayerSide::Right, 840.0),
    );
    for (a, b) in [
        (priced.perp, measured.perp),
        (priced.along, measured.along),
        (priced.ticks_to_him, measured.ticks_to_him),
        (priced.height, measured.height),
        (priced.danger, measured.danger),
    ] {
        assert!((a - b).abs() < 1e-4, "{priced:?} vs {measured:?}");
    }
    let priced_chance = PassBlock::chance(&priced, 0.6, 0.6);
    assert!(priced_chance > 0.0);
    assert!((priced_chance - PassBlock::chance(&measured, 0.6, 0.6)).abs() < 1e-6);
}

/// Reads one decision off a live context.
struct Probe<'a, T: Copy> {
    read: fn(&StateProcessingContext) -> T,
    out: &'a Cell<Option<T>>,
}

impl<T: Copy> StateProcessingHandler for Probe<'_, T> {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        self.out.set(Some((self.read)(ctx)));
        None
    }
}

fn probe<T: Copy>(
    field: &mut MatchField,
    context: &MatchContext,
    id: u32,
    read: fn(&StateProcessingContext) -> T,
) -> T {
    let tick = GameTickContext::new(field, &context.players);
    let out = Cell::new(None);
    let player = field.players.iter_mut().find(|p| p.id == id).unwrap();
    StateProcessor::new(0, player, context, &tick).process_inner(Probe { read, out: &out });
    out.get().unwrap()
}

/// Six metres out from the goal the crosser attacks, level with it.
fn six_metres_out(ctx: &StateProcessingContext) -> Vector3<f32> {
    let goal = ctx.player().opponent_goal_position();
    let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
    Vector3::new(goal.x - forward * 48.0, goal.y, 0.0)
}

/// A home winger on the ball by the byline, a home forward attacking the
/// box, the away keeper in his goal and everybody else parked. With
/// `closer`, the away full-back stands two metres in front of the winger
/// on the line to six metres out. Returns the winger's id.
fn crossing(closer: bool) -> (MatchField, MatchContext, u32) {
    let (mut field, context) = kickoff();
    let right = field.side_of(1) == PlayerSide::Left;
    let x = |at: f32| if right { at } else { 840.0 - at };
    let first = |field: &MatchField, team: u32, group: PlayerFieldPositionGroup| {
        field
            .players
            .iter()
            .find(|p| {
                p.team_id == team && p.tactical_position.current_position.position_group() == group
            })
            .unwrap()
            .id
    };
    let winger = first(&field, 1, PlayerFieldPositionGroup::Midfielder);
    let runner = first(&field, 1, PlayerFieldPositionGroup::Forward);
    let keeper = first(&field, 2, PlayerFieldPositionGroup::Goalkeeper);
    let fullback = first(&field, 2, PlayerFieldPositionGroup::Defender);
    let at = Vector3::new(x(780.0), 60.0, 0.0);
    let aim = Vector3::new(x(792.0), 272.0, 0.0);
    for p in field.players.iter_mut() {
        p.velocity = Vector3::zeros();
        p.position = if p.id == winger {
            at
        } else if p.id == runner {
            Vector3::new(x(800.0), 272.0, 0.0)
        } else if p.id == keeper {
            Vector3::new(x(834.0), 272.0, 0.0)
        } else if p.id == fullback && closer {
            at + (aim - at).normalize() * 16.0
        } else {
            Vector3::new(x(300.0), 500.0, 0.0)
        };
    }
    field.ball.position = at;
    field.ball.current_owner = Some(winger);
    (field, context, winger)
}

#[test]
fn a_cross_past_a_closer_is_still_possible_with_less_appetite() {
    let quality = |ctx: &StateProcessingContext| CrossModel::pick_rated(ctx).map(|(_, q)| q);
    let (mut field, context, winger) = crossing(false);
    let free = probe(&mut field, &context, winger, quality).unwrap();
    let (mut field, context, winger) = crossing(true);
    let pressed = probe(&mut field, &context, winger, quality).unwrap();
    assert!(
        pressed > 0.0,
        "a man in front prices the cross, he does not forbid it"
    );
    assert!(
        pressed < free,
        "{pressed} with a man in front vs {free} without"
    );
}

#[test]
fn the_ball_over_him_beats_the_ball_through_him() {
    let past = |ctx: &StateProcessingContext| {
        let aim = six_metres_out(ctx);
        (
            CrossModel::gets_past(ctx, CrossType::FloatedFarPost, aim, 0.6, 60),
            CrossModel::gets_past(ctx, CrossType::WhippedNearPost, aim, 0.6, 60),
        )
    };
    let (mut field, context, winger) = crossing(true);
    let (floated, whipped) = probe(&mut field, &context, winger, past);
    assert!(floated > whipped, "floated {floated} vs whipped {whipped}");
    let (mut field, context, winger) = crossing(false);
    assert_eq!(probe(&mut field, &context, winger, past), (1.0, 1.0));
}
