//! The goal kick, from the shot that misses to the ball in the keeper's
//! gloves.
//!
//! Reported as: *a player shoots at goal and misses, but the ball
//! instantly ends up in the hands of a prone or falling goalkeeper.*
//! Traced (`dev_match gather`, `OF_FRAME_TRACE=gather,miss`) on a shot
//! that crossed 84 cm past the post — three separate teleports in four
//! ticks:
//!
//! ```text
//!  366977   0.59  236.48  1.29   -2.42  -0.93   d=2.59  -      gk 33.1u  h=0.39  DIVE
//! *366978  50.00  244.78  0.00    0.00   0.00   d=50.1  G200   gk  0.5u  h=0.39  pickup
//!          | check_wide_of_goal: GOAL KICK, ball (-1.8, 235.5) -> (50.0, 244.8) GK 200
//!  366980  50.96  244.80  0.00    0.00   0.00   d=0.48  G200   gk  0.1u  h=0.37  hold  HELD
//! ```
//!
//! The ball jumped 50 units — 6.3 m — from beside the post into the
//! six-yard box; the keeper was teleported with it while still airborne
//! from his dive; and he was made the ball's owner on the same tick,
//! without ever taking a step for it. Two ticks later it was in his
//! gloves, where it then rode at **10 cm** for the whole hold.
//!
//! Every assertion below is therefore about POSITION OVER TIME. A test on
//! the end state alone passes against all three teleports, because the end
//! state — keeper holding the ball, goal kick to come — was always right.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{
    AwaitedRestart, HandlingVerdict, PassOriginRestart, RestartQuiet, RunOff, RunUpPhase,
};
use crate::r#match::engine::coach::CoachInstruction;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::officiating::restart_shape::RestartShape;
use crate::r#match::engine::result::{DeadTime, Score};
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::goalkeepers::states::{
    GoalkeeperCatchingState, GoalkeeperStandingState, GoalkeeperTakeBallState,
};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, MatchRng, PlayerSide,
    ResultMatchPositionData, StateProcessingContext, StateProcessingHandler,
    events::EventCollection,
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

/// The Left side's keeper, moved to `at`.
fn keeper_at(field: &mut MatchField, at: Vector3<f32>) -> u32 {
    field
        .players
        .iter_mut()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| {
            p.position = at;
            p.id
        })
        .expect("the home side has a goalkeeper")
}

/// An attacker of the Right side, to be the last man on the ball — which
/// is what makes the restart a goal kick rather than a corner.
fn attacker(field: &MatchField) -> (u32, u32) {
    field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| (p.id, p.team_id))
        .expect("the away side has outfielders")
}

/// Put a shot wide of the LEFT goal: the ball crosses the byline at
/// `exit_y`, having been struck by an away attacker.
fn shoot_wide(field: &mut MatchField, context: &MatchContext, exit_y: f32) {
    let (shooter, team) = attacker(field);
    field.ball.position = Vector3::new(-1.8, exit_y, 0.30);
    field.ball.velocity = Vector3::new(-2.4, -0.9, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(shooter);
    field
        .ball
        .record_touch(shooter, team, context.current_tick(), true);

    let mut events = EventCollection::with_capacity(4);
    let players = field.players.clone();
    field
        .ball
        .check_wide_of_goal(context, &players, &mut events);
}

/// **The ball is not moved at all when it goes out — it carries on.**
///
/// It used to be written onto the goal-kick spot: first a 6.3 m jump into
/// the six-yard box, then, when that went, a 25 cm snap back over the
/// byline onto a spot a stride inside the pitch. The second one is the
/// reported *"the ball stops on the line behind the goal, but must go
/// beyond the goal"* — small, and the one you watch every time.
///
/// Two separate things now, and the test is that they are separate: the
/// BALL is untouched by the award and goes on rolling behind the goal, and
/// the SPOT the kick is taken from is on the pitch, where the ball crossed.
#[test]
fn a_shot_that_goes_wide_leaves_the_ball_travelling() {
    if !RunOff::armed() {
        // `OF_RUN_OUT=off` puts the ball back on the spot, which is the
        // behaviour under test. Its own arm is
        // `run_out_tests::the_off_arm_still_places_the_ball_on_the_spot`.
        return;
    }
    let (mut field, context) = kickoff();
    keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    let exit_y = 200.0;
    let crossed_at = Vector3::new(-1.8, exit_y, 0.30);
    let struck_at = Vector3::new(-2.4, -0.9, 0.0);
    shoot_wide(&mut field, &context, exit_y);

    let awaited = field
        .ball
        .awaiting_restart
        .expect("a goal kick is a restart somebody has to come and take");
    assert_eq!(
        field.ball.position, crossed_at,
        "the award moved the ball — it has to be left exactly where the \
         tick that carried it over the line left it"
    );
    assert_eq!(
        field.ball.velocity, struck_at,
        "and it has to keep the pace it crossed with, or it stops dead on \
         the line by another route"
    );
    assert!(
        !awaited.settled,
        "the ball is still travelling, so the restart is not yet waiting on a spot"
    );

    let take_from = awaited
        .take_from
        .expect("a run-out restart is taken from where the ball CROSSED, not where it stops");
    assert!(
        take_from.x > 0.0,
        "the goal kick itself is taken on the pitch, got x={:.1}",
        take_from.x
    );
    let off_the_crossing = (Vector3::new(take_from.x, take_from.y, 0.0)
        - Vector3::new(crossed_at.x, crossed_at.y, 0.0))
    .magnitude();
    assert!(
        off_the_crossing < 8.0,
        "the spot is {off_the_crossing:.1}u from where the ball crossed — \
         that is the placement teleport wearing a different name"
    );
}

/// **He is not handed it, and he is not put on it.**
///
/// Both halves mattered. Ownership is what `Ball::move_to` tracks the ball
/// by, so granting it dragged the ball across the grass into whoever held
/// it; and `pending_set_piece_teleport` moved the KEEPER, which is how he
/// arrived at the ball mid-dive with both feet off the ground.
#[test]
fn the_keeper_is_neither_handed_the_ball_nor_placed_on_it() {
    let (mut field, context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 300.0, 0.0));
    shoot_wide(&mut field, &context, 180.0);

    let awaited = field.ball.awaiting_restart.expect("armed");
    assert_eq!(awaited.taker_id, gk, "the goal kick is the keeper's");
    assert!(
        field.ball.current_owner.is_none(),
        "an out-of-play ball belongs to nobody until it is picked up"
    );
    assert!(
        !field.ball.held_in_hands,
        "and it is certainly not in his gloves yet"
    );
    assert_eq!(
        field.ball.pending_set_piece_teleport, None,
        "the keeper was placed on the ball — he has to go and get it"
    );

    let keeper = field
        .players
        .iter()
        .find(|p| p.id == gk)
        .expect("the keeper is on the pitch");
    assert!(
        (keeper.position - awaited.spot).magnitude() > AwaitedRestart::REACH,
        "the test is vacuous unless he actually starts away from the ball"
    );
}

/// **And so does a ball over the BAR — but not into the goalmouth.**
///
/// This one was left placed in the goal area — 50 units up the pitch —
/// on the argument that a ball three metres up and behind the goal has no
/// honest resting place. Traced off a shot that came back off the
/// crossbar and looped over (`dev_match woodwork`):
///
/// ```text
///  515328    0.35 283.41  3.09   v(-0.40, 0.73, 0.27)
/// *515329   50.00 281.24  3.36   v( 0.00, 0.00, 0.00)   d = 49.70u = 6.2 m
/// ```
///
/// — the ball vanished as it crossed the line and reappeared hanging over
/// the six-yard box, which is the reported *"after a miss the ball appears
/// in the goalkeeper's hands, it looks like magic"*. The answer was the
/// point on the goal line it went over, and that fixed the sideways jump
/// and introduced a worse one: **the goal line between the posts is the
/// goalmouth**, so the ball was teed up inside the goal and the keeper
/// walked in there after it. Reported 2026-08-21, with a screenshot.
///
/// So the rule is split by axis, and both halves are pinned here. ACROSS
/// the pitch nothing moves: the kick is level with where the ball went
/// over, as it was. ALONG it the ball comes out of the goal and onto the
/// six-yard line, which is where a goal kick is taken from and what the
/// exit point cannot supply when the exit point is between the posts.
#[test]
fn a_ball_over_the_bar_also_dies_where_it_crossed() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    let (shooter, team) = attacker(&field);
    field.ball.position = Vector3::new(-1.0, 272.0, 3.4);
    // ⚠ **A realistic vertical, and it has to be.** This used to read 0.4,
    // which is 0.4 METRES PER TICK — 40 m/s straight up, a 40 m apex, and
    // above the engine's own launch ceiling. It cost nothing while the
    // restart nursed the height down at a flat 10 cm a tick and ignored
    // the velocity entirely; under real physics the fixture ball simply
    // left the stadium. 0.03 m/tick is 3 m/s: a shot skied over the bar
    // and still rising a little as it crosses.
    field.ball.velocity = Vector3::new(-2.0, 0.0, 0.03);
    field.ball.current_owner = None;
    field
        .ball
        .record_touch(shooter, team, context.current_tick(), true);

    let mut events = EventCollection::with_capacity(4);
    let players = field.players.clone();
    field
        .ball
        .check_over_goal(&mut context, &players, &mut events);

    let awaited = field.ball.awaiting_restart.expect("armed");
    assert_eq!(awaited.taker_id, gk);
    assert!(field.ball.current_owner.is_none());
    assert!(
        field.ball.position.z > 3.0,
        "the ball was dropped from 3.4 m to {:.2} m in one tick — the \
         award must not write the height, the flight does",
        field.ball.position.z
    );
    let crossed_at = Vector3::new(-1.0, 272.0, 0.0);
    // Arm-agnostic: with the run-out off there is one point and `spot` IS
    // the restart point, so the same assertions hold in both arms.
    let take_from = awaited.take_from.unwrap_or(awaited.spot);
    assert!(
        (take_from.y - crossed_at.y).abs() < 8.0,
        "the spot is {:.1}u ({:.2} m) SIDEWAYS of the point the ball went \
         over the bar — that is the placement teleport",
        (take_from.y - crossed_at.y).abs(),
        (take_from.y - crossed_at.y).abs() * 0.125
    );
    assert!(
        take_from.x >= 40.0,
        "the kick is teed up at x={:.1} — the goal line is x=0 and the posts \
         are 29u either side of {:.1}, so that is inside the goal. A goal \
         kick is taken from the goal AREA",
        take_from.x,
        crossed_at.y
    );
    assert!(
        take_from.x <= 44.0,
        "…and from the goal area, not the edge of the box, got x={:.1}",
        take_from.x
    );
}

/// The ball comes down to the grass under gravity rather than being
/// dropped there, and stays put once it arrives.
///
/// The bug this pins is a WRITE — the placement putting `spot.z = 0` into
/// the ball and teleporting it three metres downward on one tick. The
/// route to the grass has changed twice since (it is the flight now, not
/// the restart's 10 cm/tick settle) but the assertion has not: whatever
/// brings the ball down, no single tick of it may be a drop.
///
/// ⚠ **With one exception, and telling it apart is the point.** A ball
/// over the bar flies out of the ground, and the ball that comes back is
/// a different one — `Ball::replace_dead_ball` puts it on the spot, at
/// rest, on the deck. That tick drops the height *and* moves the ball
/// across the pitch, because it is a swap. The artefact this test exists
/// for does neither: it leaves the ball where it is and sinks it. So a
/// drop is allowed exactly when the ball has been relocated with it, and
/// only once.
#[test]
fn the_ball_falls_to_the_grass_instead_of_being_dropped_on_it() {
    let (mut field, mut context) = kickoff();
    keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    let (shooter, team) = attacker(&field);
    field.ball.position = Vector3::new(-1.0, 272.0, 3.4);
    // See `a_ball_over_the_bar_also_dies_where_it_crossed` on why this is
    // 0.03 and not 0.4.
    field.ball.velocity = Vector3::new(-2.0, 0.0, 0.03);
    field.ball.current_owner = None;
    field
        .ball
        .record_touch(shooter, team, context.current_tick(), true);
    let mut events = EventCollection::with_capacity(4);
    let players = field.players.clone();
    field
        .ball
        .check_over_goal(&mut context, &players, &mut events);

    let mut previous = field.ball.position;
    let mut swaps = 0;
    for _ in 0..400 {
        context.increment_time();
        let players = field.players.clone();
        field
            .ball
            .tick_awaited_restart(&mut context, &players, &mut events);
        let now = field.ball.position;
        let fell = previous.z - now.z;
        if fell > 0.11 {
            let moved = ((now.x - previous.x).powi(2) + (now.y - previous.y).powi(2)).sqrt();
            assert!(
                moved > AwaitedRestart::REACH,
                "the ball fell {fell:.2} m in one tick without going anywhere \
                 — that is a drop, not a fall and not a new ball"
            );
            swaps += 1;
            assert!(
                swaps == 1,
                "the ball has been swapped {swaps} times for one restart"
            );
        }
        previous = now;
    }
    assert!(
        field.ball.position.z < 1.0e-3,
        "and it has to get there, got {:.2} m",
        field.ball.position.z
    );
}

/// **The wait has to cover the job.**
///
/// A throw-in's taker is chosen for being near the ball, so one flat bound
/// covers him. The keeper is not chosen at all — he is wherever the shot
/// left him — so the bound is a function of the distance. With the flat
/// 5 s, 11.2% of goal kicks timed out with him a mean 15.7 m short and the
/// backstop teleport put the ball under him.
#[test]
fn the_ball_waits_longer_the_further_the_keeper_has_to_come() {
    let near = AwaitedRestart::patience_for(0.0);
    let far = AwaitedRestart::patience_for(200.0);
    assert_eq!(
        near,
        AwaitedRestart::PATIENCE_TICKS,
        "a taker standing on the ball waits exactly as long as anyone else"
    );
    assert!(
        far > near,
        "a keeper twenty-five metres away must be given longer, got {far} against {near}"
    );
    assert!(
        AwaitedRestart::patience_for(f32::MAX) <= AwaitedRestart::CEILING,
        "…but not long enough to stall the match"
    );
}

/// **The ball reaches his gloves.**
///
/// The carry climbs to `Ball::carry_height` at 10 cm a
/// tick, and the settle branch in `update_velocity` read "airborne" as
/// `z > 0.1` — exactly one carry step. A carried ball has no velocity of
/// its own, so it took that branch every tick, and a gather off the deck
/// locked into a two-cycle: 0 → 0.10 → 0 → 0.10, for the whole hold. The
/// replay draws that as a goalkeeper standing over the ball rather than
/// holding one, which is the artefact `carry_height` exists to prevent —
/// reintroduced from the other side.
///
/// Driven through the pair that actually fight, in the order the tick runs
/// them: either alone is self-consistent and only the ORDER produced the
/// cycle.
#[test]
fn a_ball_in_the_gloves_climbs_to_chest_height() {
    let (mut field, _context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(50.0, 272.0, 0.0));
    let team = field
        .players
        .iter()
        .find(|p| p.id == gk)
        .map(|p| p.team_id)
        .unwrap();

    field.ball.position = Vector3::new(50.0, 272.0, 0.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(gk);
    field.ball.gather_in_hands(gk, team, 1);

    let players = field.players.clone();
    for _ in 0..40 {
        field.ball.update_velocity();
        field.ball.move_to_with_players(&players);
    }
    let carry = field.ball.carry_height();
    assert!(
        (field.ball.position.z - carry).abs() < 1.0e-3,
        "a held ball settled at {:.2} m instead of the {:.2} m it is carried \
         at — the settle branch is flattening the carry every tick",
        field.ball.position.z,
        carry
    );
}

/// **A dead ball is not a loose ball.**
///
/// The loose-ball election ran on it regardless, and both halves got it
/// wrong. `should_yield_takeball` asks whether a teammate is nearer the
/// ball and throws the chaser out if one is — which for a goal kick is
/// always true, so the keeper was returned to `Standing` on the tick after
/// every nudge and never covered a metre. Measured over 400 matches with
/// the election running: 11.2% of goal kicks timed out, **100% of them
/// with the keeper standing still**, a mean 15.7 m short. With it standing
/// down: 0.0%.
///
/// The mirror half matters as much — `should_force_takeball` sent the
/// other twenty-one at a ball nobody but the taker may touch.
#[test]
fn the_election_stands_down_while_the_ball_is_out_of_play() {
    use crate::PlayerFieldPositionGroup;
    use crate::r#match::GameTickContext;
    use crate::r#match::goalkeepers::states::state::GoalkeeperState;
    use crate::r#match::player::state::PlayerState;
    use crate::r#match::player::transition::TransitionSource;

    let (mut field, context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 300.0, 0.0));
    shoot_wide(&mut field, &context, 180.0);
    let spot = field.ball.awaiting_restart.expect("armed").spot;

    // He is chasing it, and a team-mate is standing right on top of it —
    // the exact shape that used to yield him out.
    let mut nearer = None;
    for player in field.players.iter_mut() {
        if player.id == gk {
            player.transition_to(
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall),
                TransitionSource::EventHandler,
            );
        } else if nearer.is_none() && player.side == Some(PlayerSide::Left) {
            player.position = spot;
            nearer = Some(player.id);
        }
    }
    let nearer = nearer.expect("the defending side has an outfielder");

    let tick_context = GameTickContext::new(&field, &context.players);
    let keeper = field.players.iter().find(|p| p.id == gk).unwrap();
    let mate = field.players.iter().find(|p| p.id == nearer).unwrap();

    assert!(
        !PlayerFieldPositionGroup::should_yield_takeball(
            PlayerFieldPositionGroup::Goalkeeper,
            keeper,
            &tick_context
        ),
        "the taker was yielded out of his own restart because a team-mate \
         stood nearer the ball — he is not racing anybody for it"
    );
    assert!(
        !PlayerFieldPositionGroup::should_force_takeball(
            mate.tactical_position.current_position.position_group(),
            mate,
            &context,
            &tick_context
        ),
        "a player who is not the taker was sent at a ball he may not touch"
    );
}

/// **…and nothing may touch it either.**
///
/// The election standing down keeps players from RUNNING at a dead ball.
/// It does not stop the ones already standing beside it, and every player
/// event that reaches the ball writes it directly — `GainBall` and
/// `BallOwnerChange` route to `secure_ball_for`, which snaps the ball to
/// the actor's feet. `tick_awaited_restart` then pins it back on the spot
/// on the next tick, so the pair ran as a two-cycle for as long as the
/// restart waited.
///
/// Traced on a throw-in awarded by the corner flag (`dev_match gather`),
/// with the defending keeper 1.2 m away:
///
/// ```text
///  270389  801.51 543.00  own -      catch    WAIT
/// *270390  802.74 534.01  own G200   clear    WAIT   <- GainBall
/// *270391  801.51 543.00  own -      clear    WAIT   <- pinned again
/// ```
///
/// Measured over 60 matches with the guard off: **76.5 ball-touching
/// events applied to a dead ball per match, 98% of them a goalkeeper's.**
/// See [`DeadBall`](crate::r#match::engine::ball::ball::DeadBall).
#[test]
fn nobody_may_touch_a_dead_ball_including_its_taker() {
    use crate::r#match::engine::player::events::players::{PlayerEvent, PlayerEventDispatcher};
    use crate::r#match::result::ResultMatchPositionData;

    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 300.0, 0.0));
    shoot_wide(&mut field, &context, 180.0);
    // Let the run-out finish first. The rule under test is "nothing may
    // touch it", and a ball that is still rolling out has a velocity of
    // its own — which is not a touch, and would make an absolute
    // "velocity must be zero" assertion measure the wrong thing.
    let mut settle_events = EventCollection::with_capacity(4);
    for _ in 0..400 {
        if field
            .ball
            .awaiting_restart
            .is_some_and(|restart| restart.settled)
        {
            break;
        }
        context.increment_time();
        let players = field.players.clone();
        field
            .ball
            .tick_awaited_restart(&mut context, &players, &mut settle_events);
    }
    let restart = field.ball.awaiting_restart.expect("armed");
    assert!(restart.settled, "the run-out has to finish inside 4 s");
    let spot = restart.spot;

    // Somebody is standing right on the ball — the shape that used to take
    // it off the line. The taker is included on purpose: he acquires it
    // through `tick_awaited_restart` when he arrives and no other way, or
    // the ball is dragged to wherever he had got to.
    let (nearby, _) = attacker(&field);
    for player in field.players.iter_mut() {
        if player.id == nearby || player.id == gk {
            player.position = spot;
        }
    }

    let mut data = ResultMatchPositionData::new();
    for actor in [nearby, gk] {
        for event in [
            PlayerEvent::GainBall(actor),
            PlayerEvent::ClaimBall(actor),
            PlayerEvent::CaughtBall(actor),
            PlayerEvent::BallOwnerChange(actor),
            PlayerEvent::TacklingBall(actor),
            PlayerEvent::MoveBall(actor, Vector3::new(2.0, 0.0, 0.0)),
        ] {
            let before = field.ball.position;
            let label = format!("{event:?}");
            PlayerEventDispatcher::dispatch(event, &mut field, &mut context, &mut data);
            assert!(
                field.ball.current_owner.is_none(),
                "{label} took possession of a ball that is out of play"
            );
            assert!(
                (field.ball.position - before).magnitude() < 1.0e-4,
                "{label} moved the dead ball {:.2}u off its spot",
                (field.ball.position - before).magnitude()
            );
            assert!(
                field.ball.velocity.magnitude() < 1.0e-4,
                "{label} put {:.2}u/tick on a dead ball",
                field.ball.velocity.magnitude()
            );
        }
    }
    assert!(
        field.ball.awaiting_restart.is_some(),
        "and the restart is still waiting for its taker"
    );
}

/// **The keeper does not go for one either.**
///
/// The dispatcher refuses the touch, so the ball no longer moves — but he
/// still walked at it, and `Catching` → `Clearing` → `Standing` →
/// `Catching` beside a ball he can never have is the state churn the
/// sweep-limit and catch-release bands were widened to remove. All three
/// doors into a keeper claim ask [`KeeperBallClaim::is_favourite`], so one
/// answer closes them together.
#[test]
fn a_dead_ball_is_never_the_keepers_to_claim() {
    use crate::r#match::GameTickContext;
    use crate::r#match::StateProcessingContext;
    use crate::r#match::goalkeepers::states::common::KeeperBallClaim;

    let (mut field, context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 300.0, 0.0));

    // Live ball at his feet, nobody near it: he is favourite, and the test
    // is vacuous unless he is.
    field.ball.position = Vector3::new(20.5, 300.0, 0.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = None;
    let tick_context = GameTickContext::new(&field, &context.players);
    let keeper = field.players.iter().find(|p| p.id == gk).unwrap();
    let live = StateProcessingContext {
        in_state_time: 0,
        player: keeper,
        context: &context,
        tick_context: &tick_context,
    };
    assert!(
        KeeperBallClaim::is_favourite(&live),
        "a loose ball at his feet with nobody near it is his"
    );

    // Same ball, same everybody — but it is out of play now.
    shoot_wide(&mut field, &context, 180.0);
    field.ball.position = Vector3::new(20.5, 300.0, 0.0);
    let tick_context = GameTickContext::new(&field, &context.players);
    let keeper = field.players.iter().find(|p| p.id == gk).unwrap();
    let dead = StateProcessingContext {
        in_state_time: 0,
        player: keeper,
        context: &context,
        tick_context: &tick_context,
    };
    assert!(
        !KeeperBallClaim::is_favourite(&dead),
        "he claimed a dead ball — nobody is favourite for one, it belongs \
         to whoever was awarded the restart"
    );
}

// ── A goal kick is kicked ─────────────────────────────────────────────
//
// Measured before this (`dev_match stats 300 14 14`): the keeper sat in
// `Catching` beside his own placed goal kick for 64% of the wait, and on
// the tick the restart handed it to him the catch went through — 60% of
// goal kicks became a hand possession, held another 3.4 s and thrown.

/// One whole-field tick, the way a match runs it.
fn tick(field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::empty();
    let mut tick_context = GameTickContext::new(field, &context.players);
    FootballEngine::<840, 545>::game_tick(field, context, &mut data, &mut tick_context);
    context.increment_time();
}

/// A keeper who plays his goal kicks short: no leg, all touch.
fn short_kicker(field: &mut MatchField, gk: u32) {
    let keeper = field
        .get_player_mut(gk)
        .expect("the keeper is on the pitch");
    keeper.skills.goalkeeping.kicking = 1.0;
    keeper.skills.goalkeeping.passing = 20.0;
    keeper.skills.goalkeeping.first_touch = 20.0;
    keeper.skills.mental.composure = 20.0;
}

/// What happened to one goal kick ticked on the ball alone.
struct Driven {
    keeper: u32,
    /// The tick he reached the ball, and the tick it was handed to him.
    reached: u64,
    handed: u64,
    /// The tick his run-up started, for a long one.
    ran: Option<u64>,
}

/// The home keeper's goal kick, ticked on the ball alone: the shot goes
/// wide and he is stood on the ball — or on his mark — for each leg, and
/// `meddle` moves anybody else, ticks since he reached the ball in hand,
/// until the restart hands it to him.
fn drive_goal_kick(
    field: &mut MatchField,
    context: &mut MatchContext,
    gk: u32,
    mut meddle: impl FnMut(&mut MatchField, u64),
) -> Driven {
    shoot_wide(field, context, 240.0);
    let mut events = EventCollection::with_capacity(8);
    let mut ran = None;
    let mut reached = None;
    for _ in 0..4000 {
        let Some(restart) = field.ball.awaiting_restart else {
            return Driven {
                keeper: gk,
                reached: reached.expect("he never reached the ball"),
                handed: context.current_tick(),
                ran,
            };
        };
        reached = reached.or(restart.settled_tick);
        let target = match field.ball.goal_kick_run_up {
            Some(run_up) if run_up.phase != RunUpPhase::Running => run_up.mark,
            Some(_) => {
                ran.get_or_insert(context.current_tick());
                restart.spot
            }
            None => restart.spot,
        };
        field.get_player_mut(gk).unwrap().position = Vector3::new(target.x, target.y, 0.0);
        if let Some(at) = reached {
            meddle(field, context.current_tick().saturating_sub(at));
        }
        context.increment_time();
        let players = field.players.clone();
        field
            .ball
            .tick_awaited_restart(context, &players, &mut events);
    }
    panic!("the goal kick was never handed over");
}

/// [`drive_goal_kick`] for a keeper who plays it short, with nobody
/// interfering. Returns his id.
fn hand_over_goal_kick(field: &mut MatchField, context: &mut MatchContext) -> u32 {
    let gk = keeper_at(field, Vector3::new(20.0, 272.0, 0.0));
    short_kicker(field, gk);
    drive_goal_kick(field, context, gk, |_, _| {}).keeper
}

fn team_of(field: &MatchField, id: u32) -> u32 {
    field.get_player(id).expect("on the pitch").team_id
}

#[test]
fn a_goal_kick_is_his_until_he_plays_it() {
    let (mut field, mut context) = kickoff();
    let gk = hand_over_goal_kick(&mut field, &mut context);
    let tick = context.current_tick();
    assert_eq!(
        field.ball.current_owner,
        Some(gk),
        "the restart hands the ball to its taker"
    );
    assert_eq!(
        field.ball.goal_kick_taker,
        Some(gk),
        "…and it is a goal kick he has not kicked yet"
    );
    field.ball.record_touch(gk, team_of(&field, gk), tick, true);
    assert_eq!(
        field.ball.goal_kick_taker,
        Some(gk),
        "his own touch on it is not the kick"
    );
    let at = field.ball.position;
    field.ball.note_release(gk, at, tick);
    assert_eq!(
        field.ball.goal_kick_taker, None,
        "he has kicked it, so it is in play"
    );
}

#[test]
fn anybody_elses_touch_ends_the_goal_kick() {
    let (mut field, mut context) = kickoff();
    hand_over_goal_kick(&mut field, &mut context);
    let (other, team) = attacker(&field);
    field
        .ball
        .record_touch(other, team, context.current_tick(), true);
    assert_eq!(
        field.ball.goal_kick_taker, None,
        "the ball has been played by somebody else — it is in play whoever kicked it"
    );
}

#[test]
fn an_unplayed_goal_kick_may_not_be_handled() {
    let (mut field, mut context) = kickoff();
    let gk = hand_over_goal_kick(&mut field, &mut context);
    {
        let tick_context = GameTickContext::new(&field, &context.players);
        let keeper = field.get_player(gk).unwrap();
        let ctx = StateProcessingContext {
            in_state_time: 0,
            player: keeper,
            context: &context,
            tick_context: &tick_context,
        };
        assert_eq!(
            ctx.ball().handling_verdict(),
            HandlingVerdict::Restart,
            "a goal kick in his own six-yard box read as a ball he may pick up"
        );
    }

    // He plays it, and an opponent's shot brings it back to him.
    let tick = context.current_tick();
    let at = field.ball.position;
    field.ball.note_release(gk, at, tick);
    field.ball.current_owner = None;
    let (shooter, team) = attacker(&field);
    field.ball.record_touch(shooter, team, tick + 1, true);
    let tick_context = GameTickContext::new(&field, &context.players);
    let keeper = field.get_player(gk).unwrap();
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player: keeper,
        context: &context,
        tick_context: &tick_context,
    };
    assert_eq!(
        ctx.ball().handling_verdict(),
        HandlingVerdict::Legal,
        "once the goal kick has been played the ball is a ball like any other"
    );
}

/// **Whatever he is doing when it is handed to him, he kicks it.**
///
/// `Catching` is the state the measurement caught him in; `Standing` and
/// `TakeBall` are the other two he waits in. Each used to answer on its
/// own, and only two of them knew it was a goal kick.
#[test]
fn whatever_state_he_is_in_the_hand_over_is_kicked() {
    for long in [false, true] {
        for waiting in [
            GoalkeeperState::Catching,
            GoalkeeperState::Standing,
            GoalkeeperState::TakeBall,
        ] {
            let (mut field, mut context) = kickoff();
            let gk = hand_over_goal_kick(&mut field, &mut context);
            field.ball.goal_kick_long = long;
            field.get_player_mut(gk).unwrap().transition_to(
                PlayerState::Goalkeeper(waiting),
                TransitionSource::EventHandler,
            );
            let tick_context = GameTickContext::new(&field, &context.players);
            let keeper = field.get_player(gk).unwrap();
            let ctx = StateProcessingContext {
                in_state_time: 0,
                player: keeper,
                context: &context,
                tick_context: &tick_context,
            };
            // `Catching` rolls for the ball, so give it the draws.
            let change = (0..200)
                .find_map(|_| {
                    match waiting {
                        GoalkeeperState::Catching => {
                            GoalkeeperCatchingState::default().process(&ctx)
                        }
                        GoalkeeperState::Standing => {
                            GoalkeeperStandingState::default().process(&ctx)
                        }
                        _ => GoalkeeperTakeBallState::default().process(&ctx),
                    }
                    .filter(|change| change.state.is_some())
                })
                .unwrap_or_else(|| panic!("{waiting} never answered the hand-over"));
            let kick = if long {
                GoalkeeperState::Kicking
            } else {
                GoalkeeperState::Distributing
            };
            assert_eq!(
                change.state,
                Some(PlayerState::Goalkeeper(kick)),
                "{waiting}, handed a {} goal kick",
                if long { "long" } else { "short" }
            );
            let mut events = change.events;
            assert!(
                !events
                    .drain()
                    .any(|event| matches!(event, Event::PlayerEvent(PlayerEvent::CaughtBall(_)))),
                "{waiting} caught his own goal kick"
            );
            assert!(!field.ball.held_in_hands);
        }
    }
}

/// **The whole goal kick, on the real engine: he stands over a ball on the
/// grass, and he kicks it.**
#[test]
fn the_keeper_waits_over_his_goal_kick_and_kicks_it_off_the_grass() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    short_kicker(&mut field, gk);
    shoot_wide(&mut field, &context, 240.0);

    let mut handed = false;
    let mut waited = 0;
    let mut last_state = None;
    for _ in 0..6000 {
        last_state = Some(field.get_player(gk).unwrap().state);
        tick(&mut field, &mut context);
        assert!(
            !field.ball.held_in_hands,
            "the goal kick went into his gloves"
        );
        match field.ball.awaiting_restart {
            Some(restart) => {
                let Some(at) = restart.settled_tick else {
                    continue;
                };
                waited += 1;
                let state = field.get_player(gk).unwrap().state;
                assert_eq!(
                    state,
                    PlayerState::Goalkeeper(GoalkeeperState::TakeBall),
                    "waiting over his goal kick in {state}"
                );
                if context.current_tick().saturating_sub(at) > 30 {
                    assert!(
                        field.ball.position.z < 0.05,
                        "the ball is {:.2} m up — a goal kick is placed on the grass",
                        field.ball.position.z
                    );
                }
            }
            None => {
                handed = true;
                if field.ball.goal_kick_taker != Some(gk) {
                    break;
                }
            }
        }
    }
    assert!(handed, "the goal kick was never handed over");
    assert!(
        waited > 0,
        "the test is vacuous unless he waited at the spot"
    );
    assert_eq!(
        field.ball.last_release_player_id,
        Some(gk),
        "somebody other than the keeper played his goal kick"
    );
    assert!(
        matches!(
            last_state,
            Some(PlayerState::Goalkeeper(
                GoalkeeperState::Distributing | GoalkeeperState::Kicking
            ))
        ),
        "the goal kick was released from {last_state:?}, not by a pass off the floor"
    );
}

/// **A wasted goal kick is wasted once.** The delay a leading side drags it
/// out by still applies; the keeper-hold delay that used to follow it —
/// because the goal kick had become a hand possession — does not.
#[test]
fn a_wasted_goal_kick_is_delayed_once() {
    let (mut field, mut context) = kickoff();
    context.score.increment_home_goals();
    context.total_match_time = 85 * 60_000;
    context.coach_home.instruction = CoachInstruction::WasteTime;
    context.rng = MatchRng::from_seed(0x60A1);
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    assert_eq!(
        team_of(&field, gk),
        field.home_team_id,
        "the test needs the side that is ahead to be taking the goal kick"
    );
    short_kicker(&mut field, gk);
    shoot_wide(&mut field, &context, 240.0);

    let mut delayed = false;
    for _ in 0..9000 {
        tick(&mut field, &mut context);
        let keeper = field.get_player(gk).unwrap();
        assert!(
            !field.ball.held_in_hands
                && keeper.state != PlayerState::Goalkeeper(GoalkeeperState::HoldingBall),
            "the goal kick became a hand possession, which is what drew the second delay"
        );
        match field.ball.awaiting_restart {
            Some(restart) => {
                delayed |= restart
                    .hold
                    .is_some_and(|hold| hold.reason == DeadTime::Delay);
            }
            None if field.ball.goal_kick_taker != Some(gk) => break,
            None => {}
        }
    }
    assert!(delayed, "the side ahead did not drag its goal kick out");
    assert_eq!(field.ball.last_release_player_id, Some(gk));
}

// ── Taken when the set-up is formed ───────────────────────────────────
//
// Measured before this (`dev_match stats 300 14 14`): every goal kick
// waited a fixed 20 s from the keeper reaching the ball, and in 78% of
// them there was a stretch of 3 s or more — 7.0 s on average — with all
// twenty-one others standing still.

/// A keeper who plays his goal kicks long, with nobody near him to give
/// it to short.
fn long_kicker(field: &mut MatchField, gk: u32) {
    let side = field.get_player(gk).unwrap().side;
    let keeper = field
        .get_player_mut(gk)
        .expect("the keeper is on the pitch");
    keeper.skills.goalkeeping.kicking = 20.0;
    keeper.skills.goalkeeping.passing = 1.0;
    keeper.skills.goalkeeping.first_touch = 1.0;
    keeper.skills.mental.composure = 1.0;
    for p in field
        .players
        .iter_mut()
        .filter(|p| p.side == side && p.id != gk)
    {
        p.position.x = 300.0;
    }
}

/// A home outfielder who is not the keeper.
fn team_mate(field: &MatchField, gk: u32) -> u32 {
    let team = team_of(field, gk);
    field
        .players
        .iter()
        .find(|p| {
            p.team_id == team && p.id != gk && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("the home side has outfielders")
}

#[test]
fn a_goal_kick_is_formed_once_nobody_is_a_stride_inside_the_area() {
    let (mut field, _context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    let spot = Vector3::new(20.0, 272.0, 0.0);
    let formed = |field: &MatchField| {
        let taker = field.get_player(gk).unwrap();
        RestartShape::formed(
            PassOriginRestart::GoalKick,
            &field.players,
            taker,
            spot,
            0.0,
            545.0,
        )
    };
    assert!(formed(&field), "nobody of theirs is anywhere near his area");
    let (intruder, _) = attacker(&field);
    field.get_player_mut(intruder).unwrap().position = Vector3::new(100.0, 272.0, 0.0);
    assert!(
        !formed(&field),
        "a man four metres inside the area holds the goal kick"
    );
    field.get_player_mut(intruder).unwrap().position = Vector3::new(127.0, 272.0, 0.0);
    assert!(
        formed(&field),
        "a man standing on the 18-yard line is out of the area for the referee"
    );
}

#[test]
fn a_side_still_getting_into_shape_is_not_quiet() {
    let (mut field, _context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    let mate = team_mate(&field, gk);
    let look = |field: &MatchField, quiet: &mut RestartQuiet, tick: u64| {
        let taker = field.get_player(gk).unwrap().clone();
        quiet.observe(&field.players, &taker, tick);
    };

    let mut still = RestartQuiet::default();
    for tick in 0..150 {
        look(&field, &mut still, tick);
    }
    assert!(still.settled(), "a side standing in its shape is quiet");

    let mut jogging = RestartQuiet::default();
    for tick in 0..400 {
        field.get_player_mut(mate).unwrap().position.y = 50.0 + tick as f32 * 0.6;
        look(&field, &mut jogging, tick);
    }
    assert!(
        !jogging.settled(),
        "a man still running to his place is not a set side"
    );

    // A man holding his slot steers back and forth across it: his velocity
    // reads a jog and he goes nowhere.
    let mut holding = RestartQuiet::default();
    for tick in 0..150 {
        let p = field.get_player_mut(mate).unwrap();
        p.position.y = if tick % 2 == 0 { 200.0 } else { 200.5 };
        p.velocity = Vector3::new(0.0, 0.5, 0.0);
        look(&field, &mut holding, tick);
    }
    assert!(
        holding.settled(),
        "dither on the spot was read as the side still moving — the test is ground covered"
    );
}

#[test]
fn a_set_goal_kick_is_taken_after_the_placing_beat_and_not_before() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    short_kicker(&mut field, gk);
    let driven = drive_goal_kick(&mut field, &mut context, gk, |_, _| {});
    let beat = PassOriginRestart::GoalKick.set_routine_ticks();
    let waited = driven.handed - driven.reached;
    assert!(
        waited >= beat,
        "taken {waited} ticks after he reached it — before he had put it down and looked up"
    );
    assert!(
        waited < beat + 50,
        "a goal kick with its side set and its area clear waited {waited} ticks"
    );
}

#[test]
fn a_goal_kick_whose_side_never_settles_is_taken_at_the_ceiling() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    short_kicker(&mut field, gk);
    let mate = team_mate(&field, gk);
    let driven = drive_goal_kick(&mut field, &mut context, gk, |field, since| {
        let swing = ((since % 400) as f32 - 200.0).abs();
        field.get_player_mut(mate).unwrap().position.y = 50.0 + swing * 0.6;
    });
    let ceiling =
        PassOriginRestart::GoalKick.set_routine_ticks() + AwaitedRestart::GOAL_KICK_SETUP_CEILING;
    let waited = driven.handed - driven.reached;
    assert_eq!(
        ceiling, 2000,
        "the ceiling is the fixed wait every goal kick used to have"
    );
    assert!(
        (ceiling..ceiling + 10).contains(&waited),
        "a goal kick whose side never stopped moving waited {waited} ticks, not {ceiling}"
    );
}

#[test]
fn an_opponent_in_the_area_holds_the_goal_kick_until_he_leaves() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    short_kicker(&mut field, gk);
    let (intruder, _) = attacker(&field);
    const LEAVES: u64 = 600;
    let driven = drive_goal_kick(&mut field, &mut context, gk, |field, since| {
        field.get_player_mut(intruder).unwrap().position = if since < LEAVES {
            Vector3::new(100.0, 272.0, 0.0)
        } else {
            Vector3::new(300.0, 272.0, 0.0)
        };
    });
    let waited = driven.handed - driven.reached;
    assert!(
        waited >= LEAVES,
        "taken {waited} ticks in, with a man still four metres inside the area"
    );
    assert!(
        waited < LEAVES + 50,
        "he left at {LEAVES} and the kick waited until {waited}"
    );
}

#[test]
fn the_run_up_starts_when_the_picture_is_set() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    long_kicker(&mut field, gk);
    let driven = drive_goal_kick(&mut field, &mut context, gk, |_, _| {});
    let ran = driven
        .ran
        .expect("the test needs him to go long — he never took a run-up");
    let off_the_mark = ran - driven.reached;
    assert!(
        off_the_mark >= PassOriginRestart::GoalKick.set_routine_ticks(),
        "he ran in {off_the_mark} ticks after reaching the ball, before his placing beat"
    );
    assert!(
        off_the_mark < 400,
        "with the side set and the area clear he stood on his mark for {off_the_mark} ticks"
    );
}

#[test]
fn an_opponent_in_the_area_holds_the_run_up_on_its_mark() {
    let (mut field, mut context) = kickoff();
    let gk = keeper_at(&mut field, Vector3::new(20.0, 272.0, 0.0));
    long_kicker(&mut field, gk);
    let (intruder, _) = attacker(&field);
    let driven = drive_goal_kick(&mut field, &mut context, gk, |field, _| {
        field.get_player_mut(intruder).unwrap().position = Vector3::new(100.0, 272.0, 0.0);
    });
    let ran = driven
        .ran
        .expect("the test needs him to go long — he never took a run-up");
    assert!(
        ran - driven.reached >= 2000,
        "he ran in {} ticks after reaching the ball with a man still inside the area",
        ran - driven.reached
    );
}
