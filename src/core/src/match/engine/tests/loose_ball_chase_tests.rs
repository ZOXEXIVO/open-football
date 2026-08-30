//! **Chasing a loose ball.**
//!
//! Reported as: *defenders with `TakeBall` don't intercept the ball, they
//! run parallel with it* — and, separately, *intercepting defenders don't
//! try to take the ball either*.
//!
//! Both states aimed at where the ball WAS. `LooseBallChase::aim` steered
//! with `SteeringBehavior::Pursuit` at `positions.ball.position`, the
//! three `Intercepting` states did the same, and the goalkeeper's
//! `TakeBall` used a bare `Seek`. `Pursuit` is the one that was supposed
//! to lead a moving target, and its lead is `velocity × intercept_time`
//! with the time clamped to five TICKS — 50 ms, because the constant
//! reads as seconds and never was. So every chaser in the engine ran at
//! the ball's current position, and a runner aimed at where a ball is
//! turns to follow it as it goes past: the classic tail chase, at a
//! fixed gap, forever.
//!
//! It is not a close thing. A loose ball in this engine averages 0.892
//! u/tick against a 0.45-0.63 u/tick sprint, so the thing being chased is
//! normally FASTER than the man chasing it and running at it is a race
//! nobody can win.
//!
//! The tests below are about where the chaser ends up, not about what the
//! steering returns on one tick — a law that leads correctly for a tick
//! and still never arrives has not fixed anything. So each one flies the
//! real ball physics and integrates the real steering output, and the
//! headline case asserts the same chase FAILS under `OF_TAIL_CHASE`'s
//! model, because a test that only passes forwards cannot tell a fix from
//! a geometry that was always winnable.
//!
//! The lost cause — a ball whose cross-track speed alone beats the
//! chaser — used to be pinned here as deliberately conceded, because two
//! repairs had "measured worse" (both verdicts later found confounded;
//! see the history on `SteeringBehavior::Intercept`). It is now rescued:
//! when the achievable closing rate dies, the steering runs at
//! `LooseBallChase::earliest_meeting` — the first point of the decaying
//! roll the chaser can make — and the tests below demand the
//! interception instead of the concession.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::LooseBallChase;
use crate::r#match::engine::ball::ball::{BallRoll, CONTROL_DISTANCE, GROUND_FRICTION};
use crate::r#match::engine::result::Score;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayer, MatchPlayerCollection, SteeringBehavior,
};
use nalgebra::Vector3;

/// One outfielder, lifted out of the shared squad builder so his pace,
/// acceleration and agility are the ones the rest of the suite uses.
fn chaser() -> MatchPlayer {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let _ = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    field
        .players
        .iter()
        .find(|p| !p.tactical_position.current_position.is_goalkeeper())
        .expect("a squad has outfielders")
        .clone()
}

/// The ball's own ground physics, one tick of it: friction first, then
/// the position integration. Mirrors `Ball::update_velocity`'s rolling
/// branch and `apply_movement` — kept here so a test that claims to fly
/// the real ball is flying it, and so a change to either shows up as a
/// failure rather than as a quietly wrong prediction.
fn roll_one_tick(pos: &mut Vector3<f32>, vel: &mut Vector3<f32>) {
    let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
    if speed > BallRoll::STOPPED {
        vel.x *= 1.0 - GROUND_FRICTION;
        vel.y *= 1.0 - GROUND_FRICTION;
    }
    pos.x += vel.x;
    pos.y += vel.y;
}

/// How near a chaser gets to a rolling ball over `ticks`, steering with
/// `behaviour` and integrating what it returns exactly as
/// `MatchPlayer::move_to` does.
///
/// Returns `(closest approach, gap at the end, first tick within
/// control range)` in game units / ticks — the third is the number that
/// decides whether a ball rolling for the line is met while it is still
/// in play.
fn chase(
    mut player: MatchPlayer,
    mut ball_pos: Vector3<f32>,
    mut ball_vel: Vector3<f32>,
    ticks: usize,
    tail_chase: bool,
) -> (f32, f32, Option<usize>) {
    let mut closest = f32::MAX;
    let mut reached = None;
    for tick in 0..ticks {
        let steering = if tail_chase {
            // The model as it stood: run at where the ball IS, with
            // `Pursuit`'s five-tick lead. This is what `OF_TAIL_CHASE`
            // restores at runtime, and it is the control.
            SteeringBehavior::Pursuit {
                target: ball_pos,
                target_velocity: ball_vel,
            }
        } else {
            SteeringBehavior::Intercept {
                target: ball_pos,
                target_velocity: ball_vel,
            }
        };
        player.velocity = steering.calculate(&player).velocity;
        player.position += player.velocity;

        roll_one_tick(&mut ball_pos, &mut ball_vel);

        let gap = (Vector3::new(ball_pos.x, ball_pos.y, 0.0)
            - Vector3::new(player.position.x, player.position.y, 0.0))
        .norm();
        closest = closest.min(gap);
        if reached.is_none() && gap <= CONTROL_DISTANCE {
            reached = Some(tick);
        }
    }
    let gap = (Vector3::new(ball_pos.x, ball_pos.y, 0.0)
        - Vector3::new(player.position.x, player.position.y, 0.0))
    .norm();
    (closest, gap, reached)
}

/// The reported picture, as geometry: a ball crossing in front of a
/// defender, moving faster than he can run.
///
/// He starts 60u (7.5 m) off it and it rolls across his front at a
/// speed he can just about live with. There is exactly one way to reach
/// it, which is to run at the point where it WILL be rather than at the
/// point where it is — and the tail chase, given the identical
/// geometry, has to do worse, or this proves nothing.
#[test]
fn a_defender_cuts_off_a_ball_crossing_in_front_of_him() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 200.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(420.0, 260.0, 0.0);
    // 0.35 u/tick across his front, against a top speed near 0.47. Slow
    // enough that an interception line exists — quick enough that
    // running at where it is loses the race.
    let ball_vel = Vector3::new(0.35, 0.0, 0.0);

    let (closest, _, _) = chase(player.clone(), ball_pos, ball_vel, 400, false);
    let (tail_closest, _, _) = chase(player, ball_pos, ball_vel, 400, true);

    assert!(
        closest <= CONTROL_DISTANCE,
        "a defender who reads the ball should get to within control range; \
         closest approach was {closest:.1}u"
    );
    assert!(
        tail_closest > closest,
        "the tail chase must be the WORSE of the two, or this geometry \
         proves nothing: intercept {closest:.1}u vs tail chase {tail_closest:.1}u"
    );
}

/// The same defender, and the ball rolling straight away from him faster
/// than he can run. There is nothing to cut off, so the right answer is
/// the plain sprint — and the interception law has to reduce to it rather
/// than inventing a lead out of a cross-track component that is zero.
#[test]
fn a_ball_rolling_straight_away_is_chased_straight() {
    let mut player = chaser();
    player.position = Vector3::new(300.0, 272.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(360.0, 272.0, 0.0);
    let ball_vel = Vector3::new(0.9, 0.0, 0.0);

    let steering = SteeringBehavior::Intercept {
        target: ball_pos,
        target_velocity: ball_vel,
    }
    .calculate(&player)
    .velocity;

    assert!(
        steering.x > 0.0 && steering.y.abs() < 1e-3,
        "with nothing across the line of sight he should run straight at \
         it, got {steering:?}"
    );
}

/// A ball at rest is the degenerate case, and it has to come out as an
/// ordinary arrival: straight at it, and stopped once he is on it. If the
/// cross-track term leaked anything here, every player collecting a still
/// ball would drift off it.
#[test]
fn a_ball_at_rest_is_simply_run_at_and_stopped_on() {
    let mut player = chaser();
    player.position = Vector3::new(300.0, 272.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(360.0, 272.0, 0.0);

    let (closest, gap, _) = chase(player, ball_pos, Vector3::zeros(), 400, false);
    assert!(
        closest < 1.0,
        "he should reach a stationary ball, got {closest:.2}u"
    );
    assert!(
        gap < 1.0,
        "and stay on it rather than overrunning; ended {gap:.2}u away"
    );
}

/// The roll predictor has to agree with the ball, not with itself. This
/// is the guard that matters: `BallRoll` is a closed form standing in for
/// `Ball::update_velocity`, so the moment the physics changes underneath
/// it every chaser in the engine starts running to the wrong place, and
/// nothing else in the suite would notice.
#[test]
fn the_roll_prediction_matches_the_ball_physics() {
    for speed in [0.2f32, 0.5, 0.9, 2.0] {
        let mut pos = Vector3::zeros();
        let mut vel = Vector3::new(speed, 0.0, 0.0);
        for tick in 1..=600usize {
            roll_one_tick(&mut pos, &mut vel);
            let predicted = BallRoll::distance(speed, tick as f32);
            let error = (predicted - pos.x).abs();
            assert!(
                error < 0.5,
                "at {speed} u/tick, tick {tick}: predicted {predicted:.2}u, \
                 ball actually reached {:.2}u",
                pos.x
            );
        }
    }
}

/// …and it must saturate rather than run away, because a chaser who
/// cannot close is deliberately handed an unbounded time horizon and
/// asked where the ball ends up.
#[test]
fn the_roll_prediction_saturates_at_the_resting_point() {
    let speed = 0.9f32;
    let range = BallRoll::range(speed);
    assert!(range > 0.0);
    for ticks in [1.0e4f32, 1.0e6, f32::MAX] {
        let d = BallRoll::distance(speed, ticks);
        assert!(
            d.is_finite() && d <= range + 1e-3,
            "an unbounded horizon must return the resting point, got {d}"
        );
    }
    assert!(
        BallRoll::distance(speed, 1.0e6) > range - 1e-2,
        "and it must actually reach it"
    );
}

/// The meeting point has to LEAD the ball, or
/// `Intercepting::can_reach_before_opponent` is racing everybody to a
/// spot the ball will never be at — which is what it did when the lead
/// was `distance / (pace + ball_speed)` and `pace` was a 1-20 skill.
#[test]
fn the_meeting_point_leads_a_rolling_ball() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 200.0, 0.0);
    let ball_pos = Vector3::new(420.0, 260.0, 0.0);
    let ball_vel = Vector3::new(0.35, 0.0, 0.0);

    let (m, _) = LooseBallChase::earliest_meeting(
        player.position,
        player.max_speed_with_condition_cached(),
        ball_pos,
        ball_vel,
    );
    let lead = m.x - ball_pos.x;
    assert!(
        lead > 20.0,
        "a ball rolling at 0.35 u/tick across a 60u gap needs metres of          lead, not centimetres; got {lead:.1}u"
    );
    // And the two have to be able to be in that place at the same time.
    let his_time = (m - player.position).norm() / player.max_speed_with_condition_cached();
    let ball_time = {
        let (mut pos, mut vel) = (ball_pos, ball_vel);
        let mut t = 0usize;
        while (pos.x - m.x).abs() > 1.0 && t < 5000 {
            roll_one_tick(&mut pos, &mut vel);
            t += 1;
        }
        t as f32
    };
    assert!(
        (his_time - ball_time).abs() < ball_time * 0.35,
        "he should arrive about when the ball does: him {his_time:.0} ticks,          ball {ball_time:.0}"
    );
}

/// A ball at rest has nothing to lead, and a meeting point that drifted
/// off it would send every player collecting a still ball past it.
#[test]
fn the_meeting_point_of_a_still_ball_is_the_ball() {
    let mut player = chaser();
    player.position = Vector3::new(300.0, 272.0, 0.0);
    let ball_pos = Vector3::new(360.0, 272.0, 0.0);
    let (m, when) = LooseBallChase::earliest_meeting(
        player.position,
        player.max_speed_with_condition_cached(),
        ball_pos,
        Vector3::zeros(),
    );
    assert!((m - ball_pos).norm() < 1e-3, "got {m:?}");
    assert_eq!(when, 0.0, "nothing to wait for either");
}

/// The lost cause is exactly where the estimate this solver replaced
/// went to the RESTING point: no closing rate, an unbounded horizon,
/// "run to where it stops". The earliest meeting must sit well upstream
/// of the rest for a chaser this fast — that upstream margin is the
/// ball met INSIDE the pitch rather than fetched off the boards — and
/// it must be a genuine appointment: both of them there at the same
/// time.
#[test]
fn the_earliest_meeting_sits_upstream_of_the_resting_point() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 200.0, 0.0);
    let ball_pos = Vector3::new(420.0, 260.0, 0.0);
    let ball_vel = Vector3::new(0.9, 0.0, 0.0);
    let speed = player.max_speed_with_condition_cached();

    let (m, when) = LooseBallChase::earliest_meeting(player.position, speed, ball_pos, ball_vel);
    assert!(when > 0.0, "a meeting downstream takes time to happen");
    let rest_x = ball_pos.x + BallRoll::range(0.9);
    assert!(
        m.x < rest_x - 20.0,
        "the roll dies at x {rest_x:.0}; a chaser reading it meets it \
         sooner than that, got x {:.0}",
        m.x
    );

    let his_time = (m - player.position).norm() / speed;
    let ball_time = {
        let (mut pos, mut vel) = (ball_pos, ball_vel);
        let mut t = 0usize;
        while (pos.x - m.x).abs() > 1.0 && t < 5000 {
            roll_one_tick(&mut pos, &mut vel);
            t += 1;
        }
        t as f32
    };
    assert!(
        (his_time - ball_time).abs() < ball_time * 0.10,
        "an appointment, not a guess: him {his_time:.0} ticks, ball {ball_time:.0}"
    );
}

/// THE LOST CAUSE, RESCUED — inside the commitment horizon. This test
/// used to pin the opposite (`…is_conceded`), because two repairs had
/// "measured worse". Both of those verdicts were confounded (built on an
/// `aim` whose aerial branch was broken; history on the `Intercept`
/// variant), and its own note said to rewrite it the day a change made
/// him close here, alongside the census and the goals line.
///
/// When the ball's cross-track speed alone beats the chaser there is no
/// bearing to hold: the root in `SteeringBehavior::Intercept` is zero,
/// and the old law spent everything sideways — the reported frame, a
/// defender running PARALLEL to a ball he was never getting nearer.
/// Now the closing rate dying hands the chase to
/// `LooseBallChase::earliest_meeting`: let the roll die, take the
/// straight line to the first point of it he can make.
#[test]
fn a_ball_crossing_faster_than_he_can_run_is_still_cut_off() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 236.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(420.0, 260.0, 0.0);
    // Quicker than his ~0.47 sprint, all of it across his line, 24u
    // (3 m) off it — the shape a viewer means by "he could have
    // intercepted that": makeable inside a few seconds by a man who
    // reads the roll, unreachable forever for one who holds the bearing.
    let ball_vel = Vector3::new(0.55, 0.0, 0.0);

    // The first stride is already the tell. The collapse spent his whole
    // speed cross-track — pure +x, exactly parallel to the ball's
    // travel, the gap never once shrinking. The read cuts downstream
    // AND in.
    let first = SteeringBehavior::Intercept {
        target: ball_pos,
        target_velocity: ball_vel,
    }
    .calculate(&player)
    .velocity;
    assert!(first.x > 0.0, "downstream, with the roll: {first:?}");
    assert!(
        first.y > 0.01,
        "and IN toward the ball's line, not parallel to it: {first:?}"
    );

    let (closest, _, reached) = chase(player, ball_pos, ball_vel, 600, false);
    assert!(
        reached.is_some(),
        "friction gives this ball back within seconds, and a chaser who \
         read the roll is there when it does; closest approach {closest:.1}u"
    );
}

/// …AND THE MARATHON IS STILL CONCEDED, DELIBERATELY. 0.9 u/tick is the
/// measured population MEAN for a loose ball — about twice a sprint —
/// and crossing 60u (7.5 m) off his line its earliest meeting sits
/// 900+ ticks of roll downstream. No real player makes that run, and
/// paying it was measured at **+0.54 goals/match of pure shot volume**
/// (3×300 fixtures against `OF_CONCEDE`, the whole rise in throughput,
/// none in quality): rescued marathons kept attacking sequences alive
/// everywhere on the pitch. `COMMIT_FAR` prices the read in time, so
/// past ten seconds of roll the law is the pre-rescue one.
///
/// If a later change makes him close HERE, that is the economy opening
/// again, not a win — re-measure goals/match against `OF_CONCEDE`
/// before believing it.
#[test]
fn a_marathon_cut_is_declined() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 200.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(420.0, 260.0, 0.0);
    let ball_vel = Vector3::new(0.9, 0.0, 0.0);

    let (closest, _, reached) = chase(player, ball_pos, ball_vel, 400, false);
    assert!(
        reached.is_none() && closest > CONTROL_DISTANCE,
        "a ball at twice his speed crossing 7.5 m away is conceded, not \
         chased across the pitch; closest approach {closest:.1}u"
    );
}

/// The reported frame, end to end: *"it ends up rolling out of bounds,
/// even though the defender could have intercepted it."* A ball he
/// cannot outsprint rolls across him with 30u in hand. Both models get
/// there EVENTUALLY — friction guarantees that much — but the ball is
/// usually over a line first, so the number that matters is WHEN. The
/// read has to be worth whole seconds over the stern chase, or it isn't
/// buying back any of those balls.
#[test]
fn a_lost_ball_is_met_where_it_slows_not_escorted_out() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 242.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(420.0, 272.0, 0.0);
    let ball_vel = Vector3::new(0.55, 0.0, 0.0);

    let (_, _, cut) = chase(player.clone(), ball_pos, ball_vel, 900, false);
    let (_, _, tail) = chase(player, ball_pos, ball_vel, 900, true);

    let cut = cut.expect("the read reaches the ball inside the horizon");
    if let Some(tail) = tail {
        assert!(
            (cut as f32) < tail as f32 * 0.8,
            "cutting the roll off must beat trailing it by a wide margin: \
             met at tick {cut} vs {tail}"
        );
    }
    // …and `tail == None` is the same verdict, louder: the stern chase
    // never arrived at all inside the horizon.
}

/// `rest_ticks` is the inverse of the decay `distance` sums, and the
/// horizon `earliest_meeting` trusts to prove a meeting exists — if it
/// drifts off `range`, that proof quietly breaks first.
#[test]
fn the_roll_rest_time_agrees_with_the_roll_range() {
    for speed in [0.2f32, 0.5, 0.9, 2.0] {
        let t = BallRoll::rest_ticks(speed);
        assert!(t > 0.0, "a moving ball takes time to die, speed {speed}");
        let there = BallRoll::distance(speed, t);
        let range = BallRoll::range(speed);
        assert!(
            (there - range).abs() < 1.0,
            "at {speed} u/tick the ball should be at its resting point \
             ({range:.1}u) after rest_ticks ({t:.0}), got {there:.1}u"
        );
    }
    assert_eq!(
        BallRoll::rest_ticks(BallRoll::STOPPED * 0.5),
        0.0,
        "a ball already under the stopping threshold has no roll left"
    );
}
