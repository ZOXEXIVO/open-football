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
            .tick_awaited_restart(&context, &players, &mut events);
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
        AwaitedRestart::patience_for(f32::MAX) <= 1200,
        "…but not long enough to stall the match"
    );
}

/// **The ball reaches his gloves.**
///
/// `move_to`'s `carry_toward` climbs to `Ball::carry_height` at 10 cm a
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
            .tick_awaited_restart(&context, &players, &mut settle_events);
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
