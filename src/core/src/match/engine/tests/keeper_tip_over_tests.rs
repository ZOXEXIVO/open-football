//! The tip over the bar, from the keeper's fingertips to the corner flag.
//!
//! Before this the engine had one safe parry and it was flat: a shot the
//! keeper turned away left his hands at whatever height he took it and
//! travelled HORIZONTALLY to a point beside the post — `velocity.z = 0`
//! whether he had scooped it off the grass or reached above his head for
//! it. A shot into the top corner that he got his fingertips to was
//! therefore drawn as a ball at 2.2 m sliding sideways along the bar,
//! which is not a save anyone has seen. And had it gone UP, the resolver
//! for a ball crossing above the bar awarded a goal kick without asking
//! who touched it last, because until now nothing but a skied shot could
//! get there.
//!
//! Two claims, each pinned on its own: the tipped ball clears the bar
//! when it is flown through the REAL integrator (a solve that ignored the
//! drag or mixed the two axes' units would fail here, not in a comment),
//! and a ball over the bar off the defending side is a corner.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::contest::save::SaveModel;
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH};
use crate::r#match::engine::result::Score;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, MatchRng, PassOriginRestart, PlayerSide,
    ShotTarget, events::EventCollection,
};
use nalgebra::Vector3;

const WIDTH: usize = 840;
const HEIGHT: usize = 545;
const KICKOFF_MS: u64 = 10 * 60 * 1000;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(WIDTH, HEIGHT, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

/// The keeper of `side`, moved to `at`. Returns `(id, team)`.
fn keeper_at(field: &mut MatchField, side: PlayerSide, at: Vector3<f32>) -> (u32, u32) {
    field
        .players
        .iter_mut()
        .find(|p| p.side == Some(side) && p.tactical_position.current_position.is_goalkeeper())
        .map(|p| {
            p.position = at;
            (p.id, p.team_id)
        })
        .expect("the side has a goalkeeper")
}

/// An outfielder of `side`. Returns `(id, team)`.
fn outfielder(field: &MatchField, side: PlayerSide) -> (u32, u32) {
    field
        .players
        .iter()
        .find(|p| p.side == Some(side) && !p.tactical_position.current_position.is_goalkeeper())
        .map(|p| (p.id, p.team_id))
        .expect("the side has outfielders")
}

/// Fly a ball from `from` at `velocity` through the real physics until it
/// crosses `goal_line_x` or `bound` ticks run out. Returns the height at
/// which it crossed, if it did.
fn crossing_height(
    field: &mut MatchField,
    from: Vector3<f32>,
    velocity: Vector3<f32>,
    goal_line_x: f32,
    bound: usize,
) -> Option<f32> {
    field.ball.position = from;
    field.ball.velocity = velocity;
    field.ball.current_owner = None;
    field.ball.flags.in_flight_state = 60;
    let outward = (goal_line_x - from.x).signum();
    let mut previous = field.ball.position;
    for _ in 0..bound {
        field.ball.update_velocity();
        field.ball.apply_movement();
        let now = field.ball.position;
        if (now.x - goal_line_x) * outward >= 0.0 {
            // Interpolate the height at the line itself: the ball crosses
            // it somewhere inside this tick.
            let share = ((goal_line_x - previous.x) / (now.x - previous.x)).clamp(0.0, 1.0);
            return Some(previous.z + (now.z - previous.z) * share);
        }
        previous = now;
    }
    None
}

/// **The tipped ball clears the bar — from the line, and from three
/// metres out.**
///
/// The solve is gravity-only and splits its two axes by unit (game units
/// across the ground, metres up), so the only honest check is to fly a
/// real ball with the real drag and read where it crosses.
#[test]
fn a_tipped_ball_clears_the_bar_wherever_he_took_it() {
    let (mut field, _) = kickoff();
    let goal_line = WIDTH as f32;
    let goal_y = HEIGHT as f32 / 2.0;
    // Contact points: on his own line, a stride off it, and out at the
    // edge of the six-yard box; at the tip threshold and well above it;
    // with and without spray.
    for (x, z, spray) in [
        (goal_line - 1.0, SaveModel::TIP_OVER_HEIGHT, 0.0),
        (goal_line - 1.0, 2.30, SaveModel::TIP_SPRAY),
        (goal_line - 8.0, 1.90, -SaveModel::TIP_SPRAY),
        (goal_line - 24.0, SaveModel::TIP_OVER_HEIGHT, 0.0),
        (goal_line - 24.0, 2.20, SaveModel::TIP_SPRAY),
        (goal_line - 44.0, 1.80, 0.0),
    ] {
        let contact = Vector3::new(x, goal_y + 6.0, z);
        let velocity = SaveModel::tip_over_velocity(contact, goal_line, spray);
        assert!(
            velocity.z > 0.0,
            "a tip from {z:.2} m at x={x:.0} has no climb on it: {velocity:?}"
        );
        let crossed = crossing_height(&mut field, contact, velocity, goal_line, 120)
            .unwrap_or_else(|| panic!("the ball tipped from x={x:.0} never reached the line"));
        assert!(
            crossed > GOAL_HEIGHT + 0.2,
            "tipped from ({x:.0}, {z:.2}) with spray {spray:+.2}, the ball crossed the line \
             at {crossed:.2} m — under or onto a {GOAL_HEIGHT} m crossbar",
        );
        assert!(
            crossed < GOAL_HEIGHT + 1.5,
            "tipped from ({x:.0}, {z:.2}), the ball crossed at {crossed:.2} m — that is not a \
             fingertip over the bar, it is a punt",
        );
        // …and between the posts, so `check_over_goal` is the resolver
        // that sees it: the spray is a fingertip's worth, not a clearance.
        assert!(
            (field.ball.position.y - goal_y).abs() < GOAL_WIDTH,
            "the tip sprayed to y={:.1} against posts at {:.1}±{GOAL_WIDTH}",
            field.ball.position.y,
            goal_y
        );
    }
}

/// **A ball over the bar off the DEFENDING side is a corner.**
///
/// `check_over_goal` awarded a goal kick unconditionally. The sibling
/// resolver for a ball wide of the post has always asked who touched it
/// last; this one never had to, because nothing could deflect a ball up
/// there but the shooter. Same ball, same height, same place — the only
/// difference between the two halves of this test is whose touch is on
/// it, and that is the whole of the Laws' answer.
#[test]
fn a_shot_tipped_over_the_bar_is_a_corner() {
    let over_the_left_bar = |field: &mut MatchField| {
        field.ball.position = Vector3::new(-1.0, HEIGHT as f32 / 2.0, 3.2);
        field.ball.velocity = Vector3::new(-1.0, 0.1, 0.05);
        field.ball.current_owner = None;
        field.ball.awaiting_restart = None;
    };

    // Off the keeper's fingertips.
    let (mut field, mut context) = kickoff();
    let (keeper, keeper_team) = keeper_at(&mut field, PlayerSide::Left, Vector3::new(6.0, 272.0, 0.0));
    let (shooter, _) = outfielder(&field, PlayerSide::Right);
    over_the_left_bar(&mut field);
    field.ball.previous_owner = Some(shooter);
    field
        .ball
        .record_touch(keeper, keeper_team, context.current_tick(), false);
    let mut events = EventCollection::with_capacity(8);
    let players = field.players.clone();
    field
        .ball
        .check_over_goal(&mut context, &players, &mut events);

    let awaited = field
        .ball
        .awaiting_restart
        .expect("a ball over the bar is out of play, one way or the other");
    assert_eq!(
        awaited.origin,
        PassOriginRestart::Corner,
        "the keeper tipped it over and was given a goal kick"
    );
    assert_eq!(field.ball.pass_origin_restart, PassOriginRestart::Corner);
    let taker = field
        .players
        .iter()
        .find(|p| p.id == awaited.taker_id)
        .expect("the taker is on the pitch");
    assert_eq!(
        taker.side,
        Some(PlayerSide::Right),
        "the corner belongs to the side that was attacking"
    );
    assert!(
        !taker.tactical_position.current_position.is_goalkeeper(),
        "a goalkeeper does not take corners"
    );
    assert!(
        field.ball.current_owner.is_none(),
        "the award handed somebody the ball on the tick it went over"
    );

    // …and off the shooter's boot, the same ball is still a goal kick.
    let (mut field, mut context) = kickoff();
    let (keeper, _) = keeper_at(&mut field, PlayerSide::Left, Vector3::new(6.0, 272.0, 0.0));
    let (shooter, shooter_team) = outfielder(&field, PlayerSide::Right);
    over_the_left_bar(&mut field);
    field
        .ball
        .record_touch(shooter, shooter_team, context.current_tick(), true);
    let mut events = EventCollection::with_capacity(8);
    let players = field.players.clone();
    field
        .ball
        .check_over_goal(&mut context, &players, &mut events);
    let awaited = field.ball.awaiting_restart.expect("armed");
    assert_eq!(awaited.origin, PassOriginRestart::GoalKick);
    assert_eq!(awaited.taker_id, keeper);
}

/// **A high save that is not held goes UP, through the real save model.**
///
/// The outcome split is a roll, so the fixture is run under a sweep of
/// seeds: every seed on which the keeper parries a shot he took above his
/// head must leave the ball climbing, and flown on from there it must
/// cross the line above the bar. A sweep that produces no parry at all
/// fails too — a test that never reaches the branch it is about proves
/// nothing.
#[test]
fn a_high_shot_the_keeper_cannot_hold_is_tipped_over_the_bar() {
    let goal_line = WIDTH as f32;
    let goal_y = HEIGHT as f32 / 2.0;
    let mut parried = 0;
    let mut tipped = 0;
    let mut round_the_post = 0;
    for seed in 0..160u64 {
        let (mut field, mut context) = kickoff();
        context.rng = MatchRng::from_seed(seed);
        // Right-hand keeper a stride off his line, dead in line with the
        // shot; the ball at head height and above, arriving at pace.
        let (keeper, _) = keeper_at(&mut field, PlayerSide::Right, Vector3::new(832.0, goal_y, 0.0));
        let (shooter, _) = outfielder(&field, PlayerSide::Left);
        for player in field.players.iter_mut() {
            if player.id != keeper {
                player.position = Vector3::new(60.0, 40.0, 0.0);
            }
        }
        field.ball.position = Vector3::new(812.0, goal_y, 2.05);
        field.ball.velocity = Vector3::new(2.4, 0.0, 0.0);
        field.ball.current_owner = None;
        field.ball.previous_owner = Some(shooter);
        field.ball.flags.in_flight_state = 60;
        field.ball.cached_shot_target = Some(ShotTarget {
            goal_line_y: goal_y,
            goal_line_z: 2.0,
            defending_side: PlayerSide::Right,
            save_rolled: false,
            block_rolled: true,
            blocked_by: None,
            deflected: false,
            shooter_threat: 0.5,
            struck_from: Vector3::new(700.0, goal_y, 0.0),
        });

        let players = field.players.clone();
        let mut events = EventCollection::with_capacity(8);
        let mut saved = false;
        for _ in 0..30 {
            field.ball.update_light(&mut context, &players, &mut events);
            if field.ball.pending_save_credit.is_some() {
                saved = true;
                break;
            }
            if field.ball.cached_shot_target.is_none() || field.ball.position.x >= goal_line {
                break;
            }
        }
        if !saved || field.ball.current_owner.is_some() {
            continue; // beaten, or held — neither is a parry
        }
        assert_eq!(field.ball.last_touch_player_id, Some(keeper));
        // The spilled parry drops the ball in front of him; the safe one
        // sends it to the line. Only the second is under test here.
        let toward_the_line = field.ball.velocity.x > 0.0
            && (field.ball.position.x + field.ball.velocity.x * 40.0) >= goal_line;
        if !toward_the_line {
            continue;
        }
        parried += 1;
        if field.ball.velocity.z > 0.0 {
            tipped += 1;
            let from = field.ball.position;
            let velocity = field.ball.velocity;
            let crossed = crossing_height(&mut field, from, velocity, goal_line, 120)
                .expect("a tipped ball reaches the line");
            assert!(
                crossed > GOAL_HEIGHT,
                "seed {seed}: parried from {:.2} m, the ball crossed the line at {crossed:.2} m \
                 — under the bar, into his own net",
                from.z
            );
            assert!(
                (from.y - goal_y).abs() < GOAL_WIDTH,
                "seed {seed}: the tip left from y={:.1}, outside the posts",
                from.y
            );
        } else {
            round_the_post += 1;
        }
    }
    assert!(
        parried >= 3,
        "only {parried} safe parries in 160 seeds — the fixture never reaches the branch"
    );
    assert_eq!(
        round_the_post, 0,
        "{round_the_post} of {parried} parries of a ball taken above 2 m went flat round \
         the post instead of over the bar"
    );
    assert!(tipped >= 3, "{tipped} tips over the bar");
}
