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
//!
//! Who is SENT is the other half. The election used to be by distance
//! to where the ball is, which for a rolling ball names the man behind
//! it — the one man who cannot reach it — and for a pass in flight
//! names a defender against a ball its receiver will collect first.
//! `ChasePath` prices the chase in time; the tests at the end pin the
//! election on it.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::{ChasePath, LooseBallChase};
use crate::r#match::engine::ball::ball::{
    Ball, BallRoll, CONTROL_DISTANCE, GROUND_FRICTION, LOOSE_CLAIM_DISTANCE,
};
use crate::r#match::engine::result::Score;
use crate::r#match::events::EventCollection;
use crate::r#match::midfielders::states::{MidfielderState, MidfielderTakeBallState};
use crate::r#match::player::state::PlayerState;
use crate::r#match::position_ball::BallFieldData;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection, PlayerSide,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
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

/// …AND A MEETING FAR DOWNSTREAM IS STILL RUN ON THE CONVERGING LINE.
/// 0.9 u/tick is the measured population MEAN for a loose ball — about
/// twice a sprint — and crossing 60u (7.5 m) off his line its earliest
/// meeting sits 900+ ticks of roll away. This used to be pinned as
/// DECLINED: past a commitment horizon the law reverted to the pure
/// cross-track run, which is a full sprint on a line that never
/// converges — the reported frame, by construction, for the whole
/// fast-ball population. Whether a man should be sent that far is the
/// election's question (`ChasePath`); the man who IS sent runs at where
/// the ball is going.
#[test]
fn a_far_meeting_is_still_run_on_the_converging_line() {
    let mut player = chaser();
    player.position = Vector3::new(420.0, 200.0, 0.0);
    player.velocity = Vector3::zeros();
    let ball_pos = Vector3::new(420.0, 260.0, 0.0);
    let ball_vel = Vector3::new(0.9, 0.0, 0.0);

    let first = SteeringBehavior::Intercept {
        target: ball_pos,
        target_velocity: ball_vel,
    }
    .calculate(&player)
    .velocity;
    assert!(first.x > 0.0, "downstream, with the roll: {first:?}");
    assert!(
        first.y > 0.01,
        "and IN toward the ball's line, never parallel to it: {first:?}"
    );

    // The cross-track run, integrated by hand: his whole speed spent
    // along the ball's travel, none of it across the gap.
    let speed = player.max_speed_with_condition_cached();
    let parallel_gap = {
        let (mut pos, mut vel) = (ball_pos, ball_vel);
        let mut him = player.position;
        for _ in 0..400 {
            him.x += speed;
            roll_one_tick(&mut pos, &mut vel);
        }
        (Vector3::new(pos.x, pos.y, 0.0) - Vector3::new(him.x, him.y, 0.0)).norm()
    };
    let (_, gap, _) = chase(player, ball_pos, ball_vel, 400, false);
    assert!(
        gap < parallel_gap,
        "four seconds in, the converging line must have him nearer than \
         the parallel one: {gap:.1}u vs {parallel_gap:.1}u"
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
    // ~16% faster than the chaser — the ratio the scenario has always
    // been about, not the absolute number. It was 0.55 against a fixture
    // whose `acceleration` and `agility` were 0.0 and whose top speed was
    // therefore 0.473 u/tick; with the fixture built as a footballer he
    // sprints at 0.526 and 0.55 is a ball he simply reels in, which tests
    // neither model. Far above the ratio is no better: the meeting point
    // runs hundreds of units downfield and both models converge on the
    // same straight line to it.
    let ball_vel = Vector3::new(0.61, 0.0, 0.0);

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

/// The ball for the election tests: a `BallFieldData` on the turf.
fn rolling(position: Vector3<f32>, velocity: Vector3<f32>) -> BallFieldData {
    BallFieldData {
        position,
        velocity,
        spin: Vector3::zeros(),
        landing_position: position,
    }
}

/// WHO IS SENT. A ball rolling away at twice a sprint: the man 5 m
/// behind it is the one man who can never reach it, and the man 30 m
/// downstream and 6 m off its line can step into it. Distance names
/// the first; time names the second.
#[test]
fn the_election_sends_the_man_who_meets_the_roll_first() {
    let ball_pos = Vector3::new(400.0, 272.0, 0.0);
    let behind = Vector3::new(360.0, 272.0, 0.0);
    let downstream = Vector3::new(640.0, 320.0, 0.0);
    let speed = 0.5;

    let fast = ChasePath::project(
        &rolling(ball_pos, Vector3::new(0.9, 0.0, 0.0)),
        840.0,
        545.0,
    );
    let t_behind = fast.time_to_reach(behind, speed, LOOSE_CLAIM_DISTANCE);
    let t_downstream = fast.time_to_reach(downstream, speed, LOOSE_CLAIM_DISTANCE);
    assert!(
        t_downstream < t_behind,
        "the man downstream meets a fast ball first: {t_downstream:.0} vs {t_behind:.0} ticks"
    );

    // …and the same two men, the ball at rest: now it is simply nearer
    // the man behind, and he is priced at distance over speed.
    let still = ChasePath::project(&rolling(ball_pos, Vector3::zeros()), 840.0, 545.0);
    let t_behind = still.time_to_reach(behind, speed, LOOSE_CLAIM_DISTANCE);
    let t_downstream = still.time_to_reach(downstream, speed, LOOSE_CLAIM_DISTANCE);
    assert!(t_behind < t_downstream);
    let expected = ((ball_pos - behind).norm() - LOOSE_CLAIM_DISTANCE) / speed;
    assert!(
        (t_behind - expected).abs() < 1e-3,
        "a still ball is distance over speed: {t_behind:.1} vs {expected:.1}"
    );
}

/// The projection has to agree with the ball: the time it names is one
/// at which a runner flat out on the straight is genuinely on the ball.
#[test]
fn the_election_time_is_a_real_appointment() {
    let ball_pos = Vector3::new(400.0, 272.0, 0.0);
    let ball_vel = Vector3::new(0.7, 0.0, 0.0);
    let him = Vector3::new(520.0, 340.0, 0.0);
    let speed = 0.5;
    let path = ChasePath::project(&rolling(ball_pos, ball_vel), 840.0, 545.0);
    let when = path.time_to_reach(him, speed, LOOSE_CLAIM_DISTANCE);

    let (mut pos, mut vel) = (ball_pos, ball_vel);
    for _ in 0..(when.round() as usize) {
        roll_one_tick(&mut pos, &mut vel);
    }
    let there = Vector3::new(pos.x, pos.y, 0.0);
    let gap = (there - him).norm();
    assert!(
        gap <= speed * when + LOOSE_CLAIM_DISTANCE + 2.0,
        "at tick {when:.0} the ball is {gap:.1}u from him and he has run {:.1}u",
        speed * when
    );
}

/// A ball in the air is priced at where it comes DOWN. The man standing
/// under the landing spot has nothing to run, whatever the ball's xy is
/// while it flies.
#[test]
fn a_ball_in_the_air_is_priced_at_its_landing_spot() {
    let landing = Vector3::new(500.0, 272.0, 0.0);
    let ball = BallFieldData {
        position: Vector3::new(400.0, 272.0, 6.0),
        velocity: Vector3::new(1.0, 0.0, 0.2),
        spin: Vector3::zeros(),
        landing_position: landing,
    };
    let path = ChasePath::project(&ball, 840.0, 545.0);
    assert_eq!(path.time_to_reach(landing, 0.5, LOOSE_CLAIM_DISTANCE), 0.0);
    let under_the_flight = Vector3::new(400.0, 272.0, 0.0);
    assert!(path.time_to_reach(under_the_flight, 0.5, LOOSE_CLAIM_DISTANCE) > 100.0);
}

/// One match, everybody parked in a corner except the men the test
/// places, and a tick context built off it.
fn election(
    place: impl Fn(&mut MatchPlayer),
    ball: impl Fn(&mut crate::r#match::Ball),
) -> (MatchField, MatchContext, GameTickContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    for p in field.players.iter_mut() {
        p.position = Vector3::new(5.0, 5.0, 0.0);
        place(p);
    }
    // A second into the flight, so everybody has read it and the election
    // is pure geometry — see `LooseBallChase::update`.
    field.ball.current_tick_cached = 100;
    ball(&mut field.ball);
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    let tick_context = GameTickContext::new(&field, &context.players);
    (field, context, tick_context)
}

/// A PASS IN FLIGHT ENDS WHERE ITS RECEIVER TAKES IT. The defending
/// side's man is priced — and steered — to the point the pass is
/// collected, never to a point on the roll beyond it that the ball will
/// not reach: he arrives on the receiver at his first touch instead of
/// running alongside the pass (measured: 45% of every tick anybody
/// spent in `TakeBall`, parallel to it on 61% of them) or standing off
/// it (a free first touch: shots with nobody inside 1.25 m, 51% → 59%).
/// A defender who genuinely gets to the ball first is priced ahead of
/// the receiver, because that is an interception.
#[test]
fn a_pass_in_flight_ends_where_its_receiver_takes_it() {
    let ball_pos = Vector3::new(400.0, 272.0, 0.0);
    let ball_vel = Vector3::new(0.8, 0.0, 0.0);
    let receiver_id = 205;
    let defender_id = 105;

    let run = |receiver_at: Vector3<f32>, defender_at: Vector3<f32>| {
        let (field, context, tick_context) = election(
            |p| {
                if p.id == receiver_id {
                    p.position = receiver_at;
                } else if p.id == defender_id {
                    p.position = defender_at;
                }
            },
            |b| {
                b.position = ball_pos;
                b.velocity = ball_vel;
                b.pass_target_player_id = Some(receiver_id);
                b.flags.in_flight_state = 5;
            },
        );
        let end = tick_context
            .chase
            .path_end()
            .expect("a pass in flight has an end");
        assert_eq!(end.receiver, receiver_id);
        assert!(
            end.point.x > ball_pos.x && end.point.x <= receiver_at.x + 1e-3,
            "the pass ends between the ball and the man it was played to: {:?}",
            end.point
        );
        assert!(
            (end.point.y - ball_pos.y).abs() < 1e-3,
            "on its line: {:?}",
            end.point
        );
        let defender = field.players.iter().find(|p| p.id == defender_id).unwrap();
        let forced = PlayerFieldPositionGroup::should_force_takeball(
            PlayerFieldPositionGroup::Defender,
            defender,
            &context,
            &tick_context,
        );
        let receiver = field.players.iter().find(|p| p.id == receiver_id).unwrap();
        let receiver_sent = PlayerFieldPositionGroup::should_force_takeball(
            PlayerFieldPositionGroup::Forward,
            receiver,
            &context,
            &tick_context,
        );
        assert!(receiver_sent, "the receiver always goes to his own pass");
        assert!(forced, "and the defending side always sends its first man");
        (
            end,
            tick_context.chase.cost_of(defender_id).unwrap(),
            defender.max_speed_with_condition_cached(),
        )
    };

    // Receiver 9 m down the line, defender 7 m off it: the pass arrives
    // before he can get across, so his race is to the collection point
    // and it is a race he finishes second — never to the roll beyond.
    let (end, cost, speed) = run(
        Vector3::new(470.0, 272.0, 0.0),
        Vector3::new(430.0, 330.0, 0.0),
    );
    assert!(
        cost >= end.tick,
        "he is priced to the collection point, after the receiver: \
         {cost:.0} vs {:.0} ticks",
        end.tick
    );
    let there =
        ((end.point - Vector3::new(430.0, 330.0, 0.0)).norm() - LOOSE_CLAIM_DISTANCE) / speed;
    assert!(
        cost <= there.max(end.tick) + 1.0,
        "and no further: the straight run to it, or the wait for it; got {cost:.0}"
    );

    // Receiver 25 m away, defender a stride off the line the ball is
    // about to cross: he cuts it out.
    let (end, cost, _) = run(
        Vector3::new(600.0, 272.0, 0.0),
        Vector3::new(460.0, 280.0, 0.0),
    );
    assert!(
        cost < end.tick,
        "he gets there first: {cost:.0} vs {:.0} ticks",
        end.tick
    );
}

/// The projection's early end, on its own: past the tick the ball is
/// taken it is a fixed point, priced as the straight run to it or the
/// wait for it, whichever is longer.
#[test]
fn a_path_that_ends_early_is_a_fixed_point_from_there() {
    let ball_pos = Vector3::new(400.0, 272.0, 0.0);
    let ball_vel = Vector3::new(0.8, 0.0, 0.0);
    let mut path = ChasePath::project(&rolling(ball_pos, ball_vel), 840.0, 545.0);
    let taken_at = 80.0;
    let end = path.end_at(taken_at);
    let expected_x = ball_pos.x + BallRoll::distance(0.8, taken_at);
    assert!(
        (end.x - expected_x).abs() < 1e-2,
        "{end:?} vs x {expected_x:.2}"
    );

    // Far downstream: he would have met the whole roll, but it stops
    // short, so he runs to where it stopped.
    let downstream = Vector3::new(600.0, 272.0, 0.0);
    let t = path.time_to_reach(downstream, 0.5, LOOSE_CLAIM_DISTANCE);
    let straight = ((end - downstream).norm() - LOOSE_CLAIM_DISTANCE) / 0.5;
    assert!(
        (t - straight.max(taken_at)).abs() < 1e-2,
        "priced as the run to the stop: {t:.1} vs {straight:.1}"
    );

    // Upstream and quick to it: the early crossing is untouched.
    let near = Vector3::new(410.0, 280.0, 0.0);
    let untouched = ChasePath::project(&rolling(ball_pos, ball_vel), 840.0, 545.0).time_to_reach(
        near,
        0.5,
        LOOSE_CLAIM_DISTANCE,
    );
    let t = path.time_to_reach(near, 0.5, LOOSE_CLAIM_DISTANCE);
    assert!((t - untouched).abs() < 1e-3, "{t} vs {untouched}");
    assert!(t < taken_at);
}

/// The dispatcher's election on a genuinely loose ball reads the same
/// time: the man behind a fast ball is not his side's chaser, the man
/// downstream is — and the moment the ball is still, proximity decides
/// again.
#[test]
fn the_dispatcher_elects_by_time_to_the_ball() {
    let ball_pos = Vector3::new(400.0, 272.0, 0.0);
    let behind_id = 103;
    let downstream_id = 104;
    let elect = |ball_vel: Vector3<f32>| {
        let (field, _, tick_context) = election(
            |p| {
                if p.id == behind_id {
                    p.position = Vector3::new(360.0, 272.0, 0.0);
                } else if p.id == downstream_id {
                    p.position = Vector3::new(640.0, 320.0, 0.0);
                }
            },
            |b| {
                b.position = ball_pos;
                b.velocity = ball_vel;
            },
        );
        let side = field
            .players
            .iter()
            .find(|p| p.id == behind_id)
            .and_then(|p| p.side)
            .unwrap();
        (
            tick_context.chase.best(side).map(|row| row.id),
            tick_context.chase.is_designated(side, behind_id),
            tick_context.chase.is_designated(side, downstream_id),
        )
    };

    let (best, behind, downstream) = elect(Vector3::new(0.9, 0.0, 0.0));
    assert_eq!(
        best,
        Some(downstream_id),
        "a fast ball is the downstream man's"
    );
    assert!(downstream && !behind);

    let (best, behind, downstream) = elect(Vector3::zeros());
    assert_eq!(best, Some(behind_id), "a still ball is the nearer man's");
    assert!(behind && !downstream);
}

/// Both sides of the election agree on one man: the `Left` and `Right`
/// tables never name the same id, and a side's best never loses to its
/// own second.
#[test]
fn the_tables_are_consistent() {
    let (_, _, tick_context) = election(
        |p| {
            p.position = Vector3::new(
                300.0 + (p.id % 7) as f32 * 40.0,
                200.0 + (p.id % 5) as f32 * 30.0,
                0.0,
            );
        },
        |b| {
            b.position = Vector3::new(420.0, 272.0, 0.0);
            b.velocity = Vector3::new(0.6, 0.3, 0.0);
        },
    );
    let left = tick_context.chase.best(PlayerSide::Left).unwrap();
    let right = tick_context.chase.best(PlayerSide::Right).unwrap();
    assert_ne!(left.id, right.id);
    for row in tick_context.chase.rows() {
        let best = tick_context.chase.best(row.side).unwrap();
        if row.eligible {
            assert!(best.cost <= row.cost, "{best:?} vs {row:?}");
        }
        assert_eq!(tick_context.chase.cost_of(row.id), Some(row.cost));
    }
}

/// A field with one home outfielder on `at` and everybody else parked in a
/// far corner, so nothing but him is near the ball. Returns `(field,
/// context, his id)`.
fn field_with_a_man_at(at: Vector3<f32>) -> (MatchField, MatchContext, u32) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 10 * 60 * 1000;
    let man = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("the home side has outfielders");
    for player in field.players.iter_mut() {
        player.position = if player.id == man {
            at
        } else {
            Vector3::new(30.0, 30.0, 0.0)
        };
    }
    (field, context, man)
}

fn move_player(field: &mut MatchField, id: u32, to: Vector3<f32>) {
    field
        .players
        .iter_mut()
        .find(|p| p.id == id)
        .expect("the player is on the field")
        .position = to;
}

/// **A ball granted ahead of him rolls on; he runs onto it.**
///
/// Every reception is granted 0.25-1.9 m from the man, and the ball used
/// to be drawn back onto him at 1.5 u/tick from there — 85 reversals of a
/// median 33 cm inside one 30 ms frame per 90 min, off a recorded match.
/// The ball keeps its own motion until it is at his feet, and only then
/// is it on him.
#[test]
fn a_ball_granted_ahead_of_him_rolls_on_and_he_runs_onto_it() {
    let (mut field, _context, man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    field.ball.position = Vector3::new(605.0, 272.0, 0.0);
    field.ball.velocity = Vector3::new(0.4, 0.0, 0.0);
    field.ball.current_owner = Some(man);
    let mut players = field.players.clone();

    let mut last_x = field.ball.position.x;
    for _ in 0..20 {
        field.ball.update_velocity();
        field.ball.move_to_with_players(&players);
        assert!(
            field.ball.position.x > last_x,
            "the ball was pulled back toward him: {last_x:.2} -> {:.2}",
            field.ball.position.x
        );
        assert!(
            field.ball.velocity.x > 0.0 && field.ball.velocity.x <= 0.4 + 1e-4,
            "the roll was interfered with: {:?}",
            field.ball.velocity
        );
        last_x = field.ball.position.x;
    }
    assert_eq!(
        field.ball.current_owner,
        Some(man),
        "still his while it is inside the tracking cap"
    );

    // He gets there: inside a foot of it, it is on him.
    let ball = field.ball.position;
    let him = Vector3::new(ball.x - 1.0, ball.y, 0.0);
    players
        .iter_mut()
        .find(|p| p.id == man)
        .expect("the man is on the field")
        .position = him;
    field.ball.update_velocity();
    field.ball.move_to_with_players(&players);
    assert!(
        (field.ball.position.x - him.x).abs() < 1e-3
            && (field.ball.position.y - him.y).abs() < 1e-3,
        "at his feet the ball should be on him, it is at {:?}",
        field.ball.position
    );
    assert_eq!(field.ball.velocity, Vector3::zeros());
}

/// A ball in a keeper's gloves is still brought into his body: that is his
/// arms, not a force on the ball.
#[test]
fn a_ball_in_his_gloves_is_still_brought_into_his_body() {
    let (mut field, _context, _man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    let (keeper, at) = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| (p.id, p.position))
        .expect("the home side has a goalkeeper");
    field.ball.position = Vector3::new(at.x + 6.0, at.y, 1.5);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = true;
    let players = field.players.clone();
    for _ in 0..6 {
        field.ball.move_to_with_players(&players);
    }
    assert!(
        Ball::at_feet(field.ball.position, at),
        "a caught ball six units from him was left out there: {:?}",
        field.ball.position
    );
}

/// **A man whose ball is beyond his feet is sent to collect it**, and only
/// then. Nothing else closes that gap any more.
#[test]
fn a_man_whose_ball_is_beyond_his_feet_is_sent_to_collect_it() {
    let (mut field, context, man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(man);
    let sent = |field: &MatchField| {
        let tick_context = GameTickContext::new(field, &context.players);
        let player = field
            .players
            .iter()
            .find(|p| p.id == man)
            .expect("the man is on the field");
        PlayerFieldPositionGroup::should_force_takeball(
            player.tactical_position.current_position.position_group(),
            player,
            &context,
            &tick_context,
        )
    };

    field.ball.position = Vector3::new(606.0, 272.0, 0.0);
    assert!(
        sent(&field),
        "his ball is six units away and nobody sends him to it"
    );

    field.ball.position = Vector3::new(601.0, 272.0, 0.0);
    assert!(
        !sent(&field),
        "the ball is at his feet and he is still sent to fetch it"
    );

    field.ball.position = Vector3::new(606.0, 272.0, 1.2);
    field.ball.held_in_hands = true;
    assert!(!sent(&field), "a ball in his gloves is on him");
    field.ball.held_in_hands = false;

    field.ball.current_owner = Some(110);
    assert!(!sent(&field), "somebody else's ball is not his to collect");
}

/// TakeBall stays with the chase until the ball is at his feet, not until
/// it is merely his — otherwise the override re-enters it every tick.
#[test]
fn take_ball_keeps_chasing_his_own_ball_until_it_is_at_his_feet() {
    let (mut field, context, man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(man);
    let decide = |field: &MatchField| {
        let players = field.players.clone();
        let tick_context = GameTickContext::new(field, &context.players);
        let player = players
            .iter()
            .find(|p| p.id == man)
            .expect("the man is on the field");
        let ctx = StateProcessingContext {
            in_state_time: 3,
            player,
            context: &context,
            tick_context: &tick_context,
        };
        MidfielderTakeBallState::default()
            .process(&ctx)
            .and_then(|result| result.state)
    };

    field.ball.position = Vector3::new(606.0, 272.0, 0.0);
    assert_eq!(
        decide(&field),
        None,
        "he gave up the chase with his ball six units away"
    );

    field.ball.position = Vector3::new(601.0, 272.0, 0.0);
    assert_eq!(
        decide(&field),
        Some(PlayerState::Midfielder(MidfielderState::Running)),
        "the ball is at his feet and he is still chasing it"
    );
}

/// **A team-mate standing over his ball does not take it off him; an
/// opponent does.** With nothing dragging the ball to its owner he spends
/// a few ticks closing the last stride, and the ownership scan used to
/// hand the ball to whoever was within a stride of it in the meantime.
#[test]
fn a_team_mate_standing_over_his_ball_does_not_take_it_off_him() {
    let (mut field, context, man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    let other = |field: &MatchField, side: PlayerSide| {
        field
            .players
            .iter()
            .find(|p| {
                p.side == Some(side)
                    && p.id != man
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .map(|p| p.id)
            .expect("both sides have outfielders")
    };
    let mate = other(&field, PlayerSide::Left);
    let foe = other(&field, PlayerSide::Right);
    let over_the_ball = Vector3::new(609.0, 273.0, 0.0);
    let far_away = Vector3::new(30.0, 30.0, 0.0);
    field.ball.position = Vector3::new(608.0, 272.0, 0.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(man);
    field.ball.flags.in_flight_state = 0;
    let scan = |field: &mut MatchField| {
        field.ball.claim_cooldown = 0;
        let players = field.players.clone();
        let mut events = EventCollection::with_capacity(8);
        field
            .ball
            .process_ownership(&context, &players, &mut events);
        field.ball.current_owner
    };

    move_player(&mut field, mate, over_the_ball);
    assert_eq!(
        scan(&mut field),
        Some(man),
        "a team-mate standing over it took his ball"
    );

    move_player(&mut field, mate, far_away);
    move_player(&mut field, foe, over_the_ball);
    assert_eq!(
        scan(&mut field),
        Some(foe),
        "an opponent standing over a ball its owner has not reached must win it"
    );
}

/// **A pass arriving within a stride is touched onto his feet; one going
/// away from him is not pulled back.**
///
/// The touch is the only thing that may turn a ball toward its owner, and
/// it turns it at a controlled pace: never faster than the ball arrived
/// and no faster than a firm touch. A ball already past him is his to
/// chase, and rolls on.
#[test]
fn a_pass_arriving_within_a_stride_is_touched_onto_his_feet() {
    let (mut field, _context, man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    let players = field.players.clone();

    // Coming to him, passing three units beside him.
    field.ball.position = Vector3::new(596.0, 275.0, 0.0);
    field.ball.velocity = Vector3::new(1.5, 0.0, 0.0);
    field.ball.current_owner = Some(man);
    let mut arrived = None;
    for tick in 0..40 {
        field.ball.update_velocity();
        field.ball.move_to_with_players(&players);
        let speed = (field.ball.velocity.x.powi(2) + field.ball.velocity.y.powi(2)).sqrt();
        assert!(
            speed <= 1.5 + 1e-4,
            "the touch put pace on the ball: {speed:.2}"
        );
        if Ball::at_feet(field.ball.position, Vector3::new(600.0, 272.0, 0.0)) {
            arrived = Some(tick);
            break;
        }
    }
    let arrived = arrived.expect("a pass within a stride of him never reached his feet");
    assert!(
        arrived <= 20,
        "the touch took {arrived} ticks to bring a ball five units in"
    );
    // The next tick it is on him.
    field.ball.update_velocity();
    field.ball.move_to_with_players(&players);
    assert_eq!(field.ball.velocity, Vector3::zeros());
    assert_eq!(field.ball.position, Vector3::new(600.0, 272.0, 0.0));

    // Going away from him, already beyond his reach: no touch, no pull.
    field.ball.position = Vector3::new(606.0, 272.0, 0.0);
    field.ball.velocity = Vector3::new(1.0, 0.0, 0.0);
    field.ball.current_owner = Some(man);
    for _ in 0..10 {
        let before = field.ball.position.x;
        field.ball.update_velocity();
        field.ball.move_to_with_players(&players);
        assert!(
            field.ball.position.x > before,
            "a ball past him was pulled back: {before:.2} -> {:.2}",
            field.ball.position.x
        );
    }
}

/// **A ball coming to him inside his reach is his to play where he
/// stands.** The first touch brings it in, so he is not sent to fetch it
/// and TakeBall hands him back to his deciding state — a receiver who was
/// sent to fetch every arriving pass lost the first-time shot.
#[test]
fn a_ball_coming_to_him_is_his_to_play_where_he_stands() {
    let (mut field, context, man) = field_with_a_man_at(Vector3::new(600.0, 272.0, 0.0));
    // Eight units out and closing on him at pass pace.
    field.ball.position = Vector3::new(592.0, 275.0, 0.0);
    field.ball.velocity = Vector3::new(1.2, -0.3, 0.0);
    field.ball.current_owner = Some(man);

    let tick_context = GameTickContext::new(&field, &context.players);
    let player = field
        .players
        .iter()
        .find(|p| p.id == man)
        .expect("the man is on the field");
    assert!(
        !PlayerFieldPositionGroup::should_force_takeball(
            player.tactical_position.current_position.position_group(),
            player,
            &context,
            &tick_context,
        ),
        "he was sent to fetch a ball that is coming to him"
    );
    let ctx = StateProcessingContext {
        in_state_time: 3,
        player,
        context: &context,
        tick_context: &tick_context,
    };
    assert_eq!(
        MidfielderTakeBallState::default()
            .process(&ctx)
            .and_then(|result| result.state),
        Some(PlayerState::Midfielder(MidfielderState::Running)),
        "TakeBall kept him chasing a ball his touch is bringing in"
    );

    // The same ball rolling the other way is not coming to him.
    field.ball.velocity = Vector3::new(-1.2, 0.3, 0.0);
    let tick_context = GameTickContext::new(&field, &context.players);
    let player = field
        .players
        .iter()
        .find(|p| p.id == man)
        .expect("the man is on the field");
    assert!(
        PlayerFieldPositionGroup::should_force_takeball(
            player.tactical_position.current_position.position_group(),
            player,
            &context,
            &tick_context,
        ),
        "a ball rolling away from him is his to go and get"
    );
}
