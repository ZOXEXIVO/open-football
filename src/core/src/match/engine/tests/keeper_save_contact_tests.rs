//! WHERE a save puts the ball.
//!
//! The bug these exist to pin down is a visual one, and it was reported
//! twice in the same words: *the goalkeeper dives for the ball, it flies
//! through his hands and into the goal, and then instantly flips back into
//! the goalkeeper's hands.* Both halves of it were teleports —
//! `Ball::try_save_shot` adjudicating a shot that had already gone past the
//! keeper, and its catch branch then writing HIS coordinate into the ball —
//! so the assertions here are about the ball's POSITION over time rather
//! than about whether a save was booked. A test that only counted saves
//! would have passed against the broken engine.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::result::Score;
use crate::r#match::goalkeepers::states::GoalkeeperPunchingState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide, ShotTarget,
    StateChangeResult, StateProcessingContext, StateProcessingHandler,
    events::{Event, EventCollection},
};
use nalgebra::Vector3;

/// Ten minutes in, for the same reason `goal_celebration_tests` needs it:
/// the shot-provenance tests are comparisons against a tick counter.
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

/// Put the Right side's keeper on a given spot and hand him a shot to face.
/// Returns his id.
fn set_up_shot_at_the_right_goal(
    field: &mut MatchField,
    keeper_at: Vector3<f32>,
    ball_at: Vector3<f32>,
    ball_velocity: Vector3<f32>,
    struck_from: Vector3<f32>,
) -> u32 {
    let keeper_id = field
        .players
        .iter_mut()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| {
            p.position = keeper_at;
            p.id
        })
        .expect("the away side has a goalkeeper");

    // Everyone else out of the way: this is about the keeper and the ball,
    // and a defender wandering into the lane would resolve the shot with a
    // block instead.
    for player in field.players.iter_mut() {
        if player.id != keeper_id {
            player.position = Vector3::new(60.0, 40.0, 0.0);
        }
    }

    field.ball.position = ball_at;
    field.ball.velocity = ball_velocity;
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(110); // a HOME outfielder — he took the shot
    field.ball.flags.in_flight_state = 60;
    field.ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: 545.0 / 2.0 - 8.0,
        goal_line_z: 0.4,
        defending_side: PlayerSide::Right,
        save_rolled: false,
        block_rolled: true,
        blocked_by: None,
        deflected: false,
        shooter_threat: 0.5,
        struck_from,
    });
    keeper_id
}

/// **A shot that has already gone past the keeper cannot be saved by him,
/// and above all cannot drag the ball back up the pitch.**
///
/// The measured case, off a recorded match: keeper on 795, ball on 832 and
/// eight units short of crossing, dead in line with the mouth. One frame
/// later the ball was in his gloves on 795 — four and a half metres
/// BACKWARDS — because the arrival window had re-opened at the goal line
/// with him metres behind it.
#[test]
fn a_shot_that_has_beaten_the_keeper_is_never_pulled_back_into_his_gloves() {
    let (mut field, mut context) = kickoff();
    let keeper_id = set_up_shot_at_the_right_goal(
        &mut field,
        Vector3::new(795.0, 263.0, 0.0),
        Vector3::new(820.0, 264.0, 0.30),
        Vector3::new(2.2, 0.05, 0.0),
        Vector3::new(700.0, 250.0, 0.0),
    );

    let goal_line = field.size.width as f32;
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    let mut furthest = field.ball.position.x;
    let mut crossed = false;
    for _ in 0..40 {
        field.ball.update_light(&mut context, &players, &mut events);
        // Past the line the netting owns the ball and pushes it back out of
        // the mesh, which is movement this test has no business policing.
        if field.ball.position.x >= goal_line {
            crossed = true;
            break;
        }
        furthest = furthest.max(field.ball.position.x);
        assert!(
            field.ball.position.x >= furthest - 0.5,
            "the ball ran back up the pitch: it reached {furthest:.1} and is now at {:.1}",
            field.ball.position.x
        );
        assert!(
            !field.ball.held_in_hands,
            "a keeper the ball had already passed took it into his gloves at x = {:.1}",
            field.ball.position.x
        );
        assert_ne!(
            field.ball.current_owner,
            Some(keeper_id),
            "a keeper the ball had already passed was given possession of it"
        );
    }
    assert!(
        crossed,
        "the shot that beat him has to finish in the goal — it stopped at x = {:.1}",
        field.ball.position.x
    );
}

/// The same shot, but reaching him while he is still in FRONT of it, is
/// exactly the case the save model is calibrated on and must still be
/// adjudicated. Without this the test above would pass on an engine that
/// had stopped saving anything at all.
#[test]
fn a_shot_still_in_front_of_the_keeper_is_adjudicated() {
    let (mut field, mut context) = kickoff();
    set_up_shot_at_the_right_goal(
        &mut field,
        Vector3::new(832.0, 264.0, 0.0),
        Vector3::new(810.0, 264.0, 0.30),
        Vector3::new(2.2, 0.0, 0.0),
        Vector3::new(700.0, 264.0, 0.0),
    );

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    for _ in 0..40 {
        field.ball.update_light(&mut context, &players, &mut events);
        if field
            .ball
            .cached_shot_target
            .as_ref()
            .is_none_or(|t| t.save_rolled)
        {
            return; // the roll happened, which is all this is asking
        }
    }
    panic!("a shot arriving at a keeper standing in its path was never adjudicated");
}

/// **A gathered ball travels into his hands; it does not appear in them.**
///
/// The catch resolves anywhere inside the reach `SaveModel::wedge` prices —
/// up to four metres — and used to be written onto the keeper's own
/// coordinate at `carry_height` in the same tick, on both axes at once.
/// `Ball::move_to_with_players` now draws it in at `BALL_TRACK_SPEED` and
/// lifts it at `CARRY_RATE`, and must not disown it for the distance on the
/// way.
#[test]
fn a_ball_taken_at_full_stretch_is_drawn_in_rather_than_teleported() {
    let (mut field, _context) = kickoff();
    let (keeper_id, keeper_team, keeper_pos) = field
        .players
        .iter_mut()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| {
            p.position = Vector3::new(820.0, 272.0, 0.0);
            (p.id, p.team_id, p.position)
        })
        .expect("the away side has a goalkeeper");

    // Caught at full stretch: 24 units (3 m) to his left, at head height.
    field.ball.position = Vector3::new(820.0, 248.0, 1.90);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper_id);
    field.ball.gather_in_hands(keeper_id, keeper_team, 1);

    let players = field.players.clone();
    let mut previous = field.ball.position;
    let mut ticks_to_hand = None;
    for tick in 0..60 {
        field.ball.move_to_with_players(&players);
        let step = (field.ball.position - previous).magnitude();
        assert!(
            step < 2.0,
            "the ball jumped {step:.2} units in one tick on the way into his hands"
        );
        previous = field.ball.position;
        assert_eq!(
            field.ball.current_owner,
            Some(keeper_id),
            "a ball in his gloves was disowned for the distance on tick {tick}"
        );
        if ticks_to_hand.is_none()
            && (field.ball.position - Vector3::new(keeper_pos.x, keeper_pos.y, 1.15)).magnitude()
                < 0.2
        {
            ticks_to_hand = Some(tick);
        }
    }
    let arrived = ticks_to_hand.expect("the ball must end up in his hands");
    assert!(
        arrived >= 8,
        "a three-metre gather took {arrived} ticks — that is a teleport with extra steps"
    );
}

/// The Right side's keeper on `at`, everyone else out of the way. Returns
/// `(id, team)`.
fn right_keeper_alone_at(field: &mut MatchField, at: Vector3<f32>) -> (u32, u32) {
    let keeper = field
        .players
        .iter_mut()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| {
            p.position = at;
            (p.id, p.team_id)
        })
        .expect("the away side has a goalkeeper");
    for player in field.players.iter_mut() {
        if player.id != keeper.0 {
            player.position = Vector3::new(60.0, 40.0, 0.0);
        }
    }
    keeper
}

/// The tick the keeper ARRIVES in `Punching` — the one on which the state
/// decides whether there is a contact left to make.
fn punch_entry_tick(
    field: &MatchField,
    context: &MatchContext,
    keeper_id: u32,
) -> Option<StateChangeResult> {
    let players = field.players.clone();
    let tick_context = GameTickContext::new(field, &context.players);
    let player = players
        .iter()
        .find(|p| p.id == keeper_id)
        .expect("the keeper is on the field");
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player,
        context,
        tick_context: &tick_context,
    };
    GoalkeeperPunchingState::default().process(&ctx)
}

/// **A parry is ONE contact.**
///
/// The physics save resolves the shot and sends the spill on its way; the
/// keeper is then put into `Punching` so that the parry is SEEN. That state
/// rolls a fresh punch on the tick it is entered whenever a ball is within
/// a fist's reach — and a spill that has just left his gloves at a metre a
/// second always is. Off a recorded match: a ground shot spilled at 0.12 m
/// was struck again on the same tick and looped 7 m into the air, 14 m
/// back up the pitch, off a keeper who had already made his save.
///
/// The same ball off an opponent's boot is a delivery, and he may put a
/// fist through that.
#[test]
fn the_punch_state_does_not_strike_a_ball_that_just_came_off_him() {
    let (mut field, context) = kickoff();
    let (keeper_id, keeper_team) =
        right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    let tick = context.current_tick();

    // The spill exactly as `try_save_shot` leaves it: a metre a second
    // across the ground, off his hands, uncontrolled.
    field.ball.position = Vector3::new(809.0, 243.0, 0.12);
    field.ball.velocity = Vector3::new(-1.0, -0.5, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(keeper_id);
    field.ball.flags.in_flight_state = 10;
    field.ball.record_touch(keeper_id, keeper_team, tick, false);

    assert!(
        punch_entry_tick(&field, &context, keeper_id).is_none(),
        "the keeper struck his own spill a second time"
    );

    // Off an opponent, at head height: a ball he has yet to touch.
    field.ball.position.z = 2.0;
    field.ball.previous_owner = Some(110);
    field.ball.record_touch(110, 1, tick, false);

    assert!(
        punch_entry_tick(&field, &context, keeper_id).is_some(),
        "a delivery within a fist's reach was left alone"
    );
}

/// **Nothing below his head is punched.** A ball at the waist is caught,
/// gathered or dived on; the punch state had a ceiling and no floor, so a
/// crowded box could have him fisting a ball off the turf.
#[test]
fn a_ball_below_his_head_is_not_punched() {
    let (mut field, context) = kickoff();
    let (keeper_id, _) = right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    field.ball.position = Vector3::new(809.0, 243.0, 1.0);
    field.ball.velocity = Vector3::new(-1.0, -0.5, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(110);
    field
        .ball
        .record_touch(110, 1, context.current_tick(), false);

    assert!(
        punch_entry_tick(&field, &context, keeper_id).is_none(),
        "he fisted a ball at his waist"
    );

    field.ball.position.z = GoalkeeperPunchingState::FLOOR + 0.1;
    assert!(
        punch_entry_tick(&field, &context, keeper_id).is_some(),
        "a ball at his head was left alone"
    );
}

/// **A punch is a clearance, not a lob.** It leaves the fist nearer 40°
/// than the 58° a 6-10 m apex over 15-25 m gave, which on screen was a
/// ball popping straight up and dropping.
#[test]
fn a_punch_leaves_the_fist_flatter_than_it_climbs() {
    let (mut field, context) = kickoff();
    let (keeper_id, _) = right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    field.ball.position = Vector3::new(809.0, 243.0, 2.0);
    field.ball.velocity = Vector3::new(-1.0, -0.5, -0.05);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(110);
    field
        .ball
        .record_touch(110, 1, context.current_tick(), false);

    // The contact is a roll with at least a one-in-five chance; sixty
    // entries without one would be a broken state, not bad luck.
    let launch = (0..60)
        .find_map(|_| {
            let mut result = punch_entry_tick(&field, &context, keeper_id)?;
            result.events.drain().find_map(|event| match event {
                Event::PlayerEvent(PlayerEvent::ClearBall(_, velocity)) => Some(velocity),
                _ => None,
            })
        })
        .expect("sixty punch entries produced no contact");

    // x/y in game units per tick, z in metres per tick: 1u = 0.125 m.
    let across_metres = (launch.x * launch.x + launch.y * launch.y).sqrt() * 0.125;
    let angle = launch.z.atan2(across_metres).to_degrees();
    assert!(
        (25.0..45.0).contains(&angle),
        "the punch left the fist at {angle:.0} degrees: {launch:?}"
    );
}
