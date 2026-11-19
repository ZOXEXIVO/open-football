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
use crate::r#match::goalkeepers::states::common::{AerialClaim, KeeperAerialClaim};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::engine::player::events::players::PlayerEventDispatcher;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    Ball, ClaimIntent, GameTickContext, MatchContext, MatchField, MatchPlayer,
    MatchPlayerCollection, PlayerSide, ResultMatchPositionData, ShotTarget, StateProcessingContext,
    StateProcessingHandler,
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
        seen_from: struck_from,
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
/// The carry now brings it in at a hand's pace (`Ball::HANDS_BOUND`) and
/// lifts it at 10 cm a tick, and must not disown it for the distance on the
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
    for tick in 0..120 {
        field.ball.move_to_with_players(&players);
        let step = field.ball.position - previous;
        assert!(
            step.x.abs() <= Ball::HANDS_BOUND + 1e-4
                && step.y.abs() <= Ball::HANDS_BOUND + 1e-4
                && step.z.abs() <= 0.1 + 1e-4,
            "the ball jumped {step:?} in one tick on the way into his hands"
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
        (8..=100).contains(&arrived),
        "a three-metre gather took {arrived} ticks — a teleport, or longer than a second"
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
/// `with` run against the right-hand keeper, as a state handler sees him.
fn as_keeper<T>(
    field: &MatchField,
    context: &MatchContext,
    keeper_id: u32,
    with: impl FnOnce(&StateProcessingContext) -> T,
) -> T {
    let tick_context = GameTickContext::new(field, &context.players);
    let player = field
        .players
        .iter()
        .find(|p| p.id == keeper_id)
        .expect("the keeper is on the field");
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player,
        context,
        tick_context: &tick_context,
    };
    with(&ctx)
}

/// **The punch state throws no punch of its own.** It is the follow-through
/// of a punch thrown on the aerial contest's word, so a ball within a fist's
/// reach, his own spill or an opponent's delivery, is not struck by it.
#[test]
fn the_punch_state_strikes_nothing_the_contest_did_not_give_it() {
    let (mut field, context) = kickoff();
    let (keeper_id, keeper_team) =
        right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    let tick = context.current_tick();
    field.ball.position = Vector3::new(809.0, 243.0, 0.12);
    field.ball.velocity = Vector3::new(-1.0, -0.5, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(keeper_id);
    field.ball.flags.in_flight_state = 10;
    field.ball.record_touch(keeper_id, keeper_team, tick, false);

    for (z, from) in [(0.12, keeper_id), (2.0, 110)] {
        field.ball.position.z = z;
        field.ball.previous_owner = Some(from);
        let struck = as_keeper(&field, &context, keeper_id, |ctx| {
            GoalkeeperPunchingState::default().process(ctx)
        })
        .is_some_and(|mut change| {
            change
                .events
                .drain()
                .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::ClearBall(..))))
        });
        assert!(!struck, "the punch state struck a ball at {z} m on its own");
    }
}

/// **Nothing below his head is punched.** However crowded the box, a claim
/// he meets at the waist is one he means to catch.
#[test]
fn a_ball_below_his_head_is_not_punched() {
    let (mut field, context) = kickoff();
    let (keeper_id, _) = right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    let keeper = field
        .players
        .iter_mut()
        .find(|p| p.id == keeper_id)
        .expect("the keeper");
    keeper.skills.goalkeeping.command_of_area = 20.0;
    keeper.skills.goalkeeping.aerial_reach = 20.0;
    keeper.skills.goalkeeping.punching = 20.0;
    keeper.skills.physical.jumping = 20.0;
    field.ball.velocity = Vector3::new(-1.0, -0.5, 0.0);
    let intent = |height: f32| {
        let claim = AerialClaim {
            meeting_point: Vector3::new(809.0, 243.0, 0.0),
            height,
            ticks: 10.0,
            traffic: 1.0,
            standing: true,
        };
        as_keeper(&field, &context, keeper_id, |ctx| {
            KeeperAerialClaim::intent(ctx, &claim)
        })
    };
    assert_eq!(
        intent(1.0),
        ClaimIntent::Catch,
        "he meant to fist a ball at his waist"
    );
    assert_eq!(
        intent(GoalkeeperPunchingState::FLOOR + 0.1),
        ClaimIntent::Punch,
        "a ball at his head in a crowd was not one to punch"
    );
}

/// **A punch clears the heads and is still not a lob.** It goes up at least
/// 4 m and carries 15-25 m, a launch in the mid-forties, where the 6-10 m
/// apex over the same ground left the fist at 58° and on screen was a ball
/// popping straight up and dropping.
#[test]
fn a_punch_clears_the_heads_without_becoming_a_lob() {
    let (mut field, context) = kickoff();
    let (keeper_id, _) = right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    field.ball.position = Vector3::new(809.0, 243.0, 2.0);
    field.ball.velocity = Vector3::new(-1.0, -0.5, -0.05);
    field.ball.current_owner = None;

    let mut strike = as_keeper(&field, &context, keeper_id, GoalkeeperPunchingState::strike);
    let launch = strike
        .events
        .drain()
        .find_map(|event| match event {
            Event::PlayerEvent(PlayerEvent::ClearBall(_, velocity)) => Some(velocity),
            _ => None,
        })
        .expect("the punch throws the ball");

    // x/y in game units per tick, z in metres per tick: 1u = 0.125 m.
    let across_metres = (launch.x * launch.x + launch.y * launch.y).sqrt() * 0.125;
    let angle = launch.z.atan2(across_metres).to_degrees();
    assert!(
        (40.0..50.0).contains(&angle),
        "the punch left the fist at {angle:.0} degrees: {launch:?}"
    );
    assert!(
        Ball::apex_for_launch(launch.z) >= 4.0 - 1e-3,
        "the punch went up {:.1} m",
        Ball::apex_for_launch(launch.z)
    );
}

#[test]
fn a_howler_is_the_keepers_error() {
    let (mut field, mut context) = kickoff();
    let (keeper_id, _) = right_keeper_alone_at(&mut field, Vector3::new(811.0, 248.0, 0.0));
    let before = context.psychology.get(keeper_id).map_or(0.0, |s| s.confidence);

    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(
        PlayerEvent::Howler(keeper_id),
        &mut field,
        &mut context,
        &mut data,
    );

    let keeper = field.get_player(keeper_id).unwrap();
    assert_eq!(keeper.statistics.errors_leading_to_shot, 1);
    assert_eq!(
        field.ball.pending_error_to_shot_player_id,
        Some(keeper_id),
        "the goal it becomes is charged to him"
    );
    assert!(context.psychology.get(keeper_id).unwrap().confidence < before);
}

// ── His posture at contact ──────────────────────────────────────────────

/// Whether the save model takes up a shot passing `beside` units to the side
/// of the away keeper at `height` metres, with him put in his posture by
/// `posture`: rolled (held, tipped, spilled or beaten) rather than out of
/// his reach.
fn engages(posture: impl Fn(&mut MatchPlayer), beside: f32, height: f32) -> bool {
    let (mut field, context) = kickoff();
    let keeper_id = set_up_shot_at_the_right_goal(
        &mut field,
        Vector3::new(820.0, 272.0, 0.0),
        Vector3::new(817.0, 272.0 + beside, height),
        Vector3::new(2.2, 0.0, 0.0),
        Vector3::new(700.0, 272.0 + beside, 0.0),
    );
    posture(
        field
            .players
            .iter_mut()
            .find(|p| p.id == keeper_id)
            .expect("the keeper"),
    );
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    field.ball.try_save_shot(&context, &players, &mut events);
    field.ball.cached_shot_target.is_none_or(|t| t.save_rolled)
}

fn in_state(state: GoalkeeperState) -> impl Fn(&mut MatchPlayer) {
    move |keeper| keeper.transition_to(PlayerState::Goalkeeper(state), TransitionSource::Reset)
}

#[test]
fn a_set_keeper_reaches_a_shot_two_metres_wide() {
    assert!(engages(
        in_state(GoalkeeperState::PreparingForSave),
        16.0,
        0.5
    ));
}

#[test]
fn a_committed_dive_reaches_only_his_spread() {
    let committed = |keeper: &mut MatchPlayer| {
        keeper.transition_to(
            PlayerState::Goalkeeper(GoalkeeperState::Diving),
            TransitionSource::Reset,
        );
        keeper.dive_aim = Some(272.0 - 30.0);
    };
    assert!(
        !engages(committed, 16.0, 0.5),
        "he dived the other way and still reached it"
    );
    assert!(
        engages(committed, 3.0, 0.5),
        "a ball at his body was out of his reach"
    );
}

#[test]
fn a_keeper_on_the_floor_reaches_only_a_low_ball_at_his_body() {
    let grounded = in_state(GoalkeeperState::GettingUp);
    assert!(
        engages(&grounded, 3.0, 0.3),
        "a low ball at a man on the floor was out of reach"
    );
    assert!(
        !engages(&grounded, 3.0, 1.2),
        "a man lying down reached a ball at his chest"
    );
    assert!(
        !engages(&grounded, 16.0, 0.3),
        "a man lying down dived for a ball two metres wide"
    );
}

/// Saves out of `n` shots passing `beside` units wide of a set keeper at
/// `height` metres.
fn saves_at(height: f32, beside: f32, n: usize) -> usize {
    let (mut field, context) = kickoff();
    (0..n)
        .filter(|_| {
            let keeper_id = set_up_shot_at_the_right_goal(
                &mut field,
                Vector3::new(820.0, 272.0, 0.0),
                Vector3::new(817.0, 272.0 + beside, height),
                Vector3::new(2.2, 0.0, 0.0),
                Vector3::new(700.0, 272.0 + beside, 0.0),
            );
            field.ball.held_in_hands = false;
            field.ball.pending_save_credit = None;
            in_state(GoalkeeperState::PreparingForSave)(
                field
                    .players
                    .iter_mut()
                    .find(|p| p.id == keeper_id)
                    .expect("the keeper"),
            );
            let players = field.players.clone();
            let mut events = EventCollection::with_capacity(8);
            field.ball.try_save_shot(&context, &players, &mut events);
            field.ball.pending_save_credit.is_some()
                || field.ball.held_in_hands
                || field.ball.current_owner == Some(keeper_id)
        })
        .count()
}

/// **Height costs him as width does.** The same metre to his side is a
/// harder save just under the bar than at his chest, priced by the exchange
/// rate the dive decision already uses (`SaveModel::climb_gap`).
#[test]
fn a_shot_under_the_bar_is_saved_less_often_than_one_at_his_chest() {
    let chest = saves_at(1.2, 8.0, 400);
    let bar = saves_at(2.3, 8.0, 400);
    assert!(
        chest > 0,
        "no chest-height save at all, so nothing was tested"
    );
    assert!(
        bar < chest,
        "under the bar {bar} of 400 saved, at his chest {chest}"
    );
}

// ── Where a ball he does not hold comes to rest ─────────────────────────

/// Everybody off to a corner, then the ball run on its own physics until it
/// stops. `None` when it leaves the pitch instead.
pub(super) fn rest_point(
    field: &mut MatchField,
    context: &mut MatchContext,
) -> Option<Vector3<f32>> {
    for player in field.players.iter_mut() {
        player.position = Vector3::new(30.0, 30.0, 0.0);
        player.velocity = Vector3::zeros();
    }
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    for _ in 0..4000 {
        field.ball.update_light(context, &players, &mut events);
        if field.ball.awaiting_restart.is_some() || field.ball.in_net.is_some() {
            return None;
        }
        let ball = &field.ball;
        if ball.position.z < 0.2 && ball.velocity.x.hypot(ball.velocity.y) < 0.02 {
            return Some(ball.position);
        }
    }
    Some(field.ball.position)
}

/// How a save he did not hold left him, read off the ball the tick after.
#[derive(Clone, Copy, PartialEq, Debug)]
enum LooseSave {
    /// Pushed wide on a fixed line: twice as much across him as out.
    Parried,
    Spilled,
}

/// A low shot at the middle of the away goal, saved but not held: how each
/// one left him and where it came to rest from where he met it, in metres.
fn rests_of_saves_not_held(n: usize) -> Vec<(LooseSave, Vector3<f32>)> {
    (0..n)
        .filter_map(|_| {
            let (mut field, mut context) = kickoff();
            let keeper_id = set_up_shot_at_the_right_goal(
                &mut field,
                Vector3::new(820.0, 272.0, 0.0),
                Vector3::new(817.0, 272.0, 0.5),
                Vector3::new(2.2, 0.0, 0.0),
                Vector3::new(700.0, 272.0, 0.0),
            );
            in_state(GoalkeeperState::PreparingForSave)(
                field
                    .players
                    .iter_mut()
                    .find(|p| p.id == keeper_id)
                    .expect("the keeper"),
            );
            let players = field.players.clone();
            let mut events = EventCollection::with_capacity(8);
            field.ball.try_save_shot(&context, &players, &mut events);
            if field.ball.pending_save_credit.is_none() || field.ball.current_owner.is_some() {
                return None;
            }
            let v = field.ball.velocity;
            let kind = if (v.y.abs() - 2.0 * v.x.abs()).abs() <= 1e-5 * v.y.abs() {
                LooseSave::Parried
            } else {
                LooseSave::Spilled
            };
            let contact = field.ball.position;
            rest_point(&mut field, &mut context).map(|rest| (kind, (rest - contact) * 0.125))
        })
        .collect()
}

/// **A spill drops in the box in front of him, and a parry kept in play
/// goes wide of him.** The spec's two bands, checked where each ball
/// actually came to rest.
#[test]
fn a_save_he_does_not_hold_comes_to_rest_where_its_outcome_says() {
    let rests = rests_of_saves_not_held(400);
    let spills: Vec<_> = rests
        .iter()
        .filter(|(k, _)| *k == LooseSave::Spilled)
        .collect();
    let parries: Vec<_> = rests
        .iter()
        .filter(|(k, _)| *k == LooseSave::Parried)
        .collect();
    assert!(
        !spills.is_empty(),
        "no spill in {} saves not held",
        rests.len()
    );
    assert!(
        !parries.is_empty(),
        "no parry kept in play in {} saves not held",
        rests.len()
    );
    for (_, d) in &spills {
        let metres = d.x.hypot(d.y);
        assert!(
            (1.5..=6.0).contains(&metres),
            "a spill came to rest {metres:.1} m away: {d:?}"
        );
        assert!(d.x < 0.0, "a spill went back toward his own goal: {d:?}");
    }
    for (_, d) in &parries {
        let metres = d.x.hypot(d.y);
        assert!(
            (5.0..=15.0).contains(&metres),
            "a parry came to rest {metres:.1} m away: {d:?}"
        );
        assert!(
            d.y.abs() > 2.0 * d.x.abs() - 0.5,
            "a parry kept in play was not wide: {d:?}"
        );
    }
}

/// **A save he does not hold is his touch.** The own-goal rule reads the
/// last touch, so a parry or a spill that ricochets in off nobody else has
/// to be on him.
#[test]
fn a_save_he_does_not_hold_books_his_touch() {
    let mut loose = 0;
    for _ in 0..200 {
        let (mut field, context) = kickoff();
        let keeper_id = set_up_shot_at_the_right_goal(
            &mut field,
            Vector3::new(820.0, 272.0, 0.0),
            Vector3::new(817.0, 272.0, 0.5),
            Vector3::new(2.2, 0.0, 0.0),
            Vector3::new(700.0, 272.0, 0.0),
        );
        in_state(GoalkeeperState::PreparingForSave)(
            field
                .players
                .iter_mut()
                .find(|p| p.id == keeper_id)
                .expect("the keeper"),
        );
        let players = field.players.clone();
        let mut events = EventCollection::with_capacity(8);
        field.ball.try_save_shot(&context, &players, &mut events);
        if field.ball.pending_save_credit.is_none() || field.ball.current_owner.is_some() {
            continue;
        }
        loose += 1;
        assert_eq!(field.ball.last_touch_player_id, Some(keeper_id));
    }
    assert!(loose > 0, "no save in 200 was left loose");
}
