//! **The ball does not go through the goalkeeper.**
//!
//! Reported from the stands, verbatim: *"when a goalkeeper jumps to block
//! a ball, it is not taken into account that he can block it with his
//! body — the ball passes through him no matter how he jumps."* Measured
//! before the fix with `dev_match stats` and the `KEEPER BODY` census
//! reading the sweep with the volume switched off: **3.1 balls a match
//! travelled through a goalkeeper**, 0.46 of them live shots on frame —
//! 18% of every goal the engine scored.
//!
//! These are position tests rather than save-count tests, for the same
//! reason `keeper_save_contact_tests` is: the complaint is about what the
//! man watching sees, and a test that counted saves would have passed
//! against the broken engine — the save model was working exactly as
//! designed, and being beaten on the roll is supposed to let the ball
//! through. What must not happen is the ball travelling through the space
//! his body is standing in.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::contest::body::KeeperBody;
use crate::r#match::engine::result::Score;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayer, MatchPlayerCollection, PlayerSide, ShotTarget,
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

/// The Right side's keeper, everyone else parked out of the way so the only
/// thing between the ball and the goal is the man this is about.
fn lone_keeper(field: &mut MatchField, at: Vector3<f32>) -> u32 {
    let keeper_id = field
        .players
        .iter_mut()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| {
            p.position = at;
            p.id
        })
        .expect("the away side has a goalkeeper");
    for player in field.players.iter_mut() {
        if player.id != keeper_id {
            player.position = Vector3::new(60.0, 40.0, 0.0);
        }
    }
    keeper_id
}

/// A shot in flight at the right-hand goal that the keeper has ALREADY been
/// beaten by on the roll.
///
/// `save_rolled` is the whole point of the setup: `try_save_shot` latches it
/// and returns on every subsequent tick ("one shot, one roll"), so nothing
/// in the save model can touch this ball. Whatever stops it is the body.
fn a_shot_he_has_already_been_beaten_by(
    field: &mut MatchField,
    at: Vector3<f32>,
    velocity: Vector3<f32>,
) {
    field.ball.position = at;
    field.ball.velocity = velocity;
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(110); // a HOME outfielder took it
    field.ball.flags.in_flight_state = 60;
    field.ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: 545.0 / 2.0,
        goal_line_z: 0.4,
        defending_side: PlayerSide::Right,
        save_rolled: true,
        block_rolled: true,
        blocked_by: None,
        deflected: false,
        shooter_threat: 0.5,
        struck_from: Vector3::new(700.0, 264.0, 0.0),
    });
}

fn keeper_mut(field: &mut MatchField, id: u32) -> &mut MatchPlayer {
    field
        .players
        .iter_mut()
        .find(|p| p.id == id)
        .expect("the keeper is on the pitch")
}

/// **The headline case.** A shot struck at a standing keeper's chest that
/// his hands have already failed to stop does not come out the other side
/// of him.
#[test]
fn a_shot_at_his_chest_does_not_pass_through_him() {
    let (mut field, mut context) = kickoff();
    let keeper_at = Vector3::new(820.0, 264.0, 0.0);
    let keeper_id = lone_keeper(&mut field, keeper_at);
    // Two metres out, dead in line with him, chest height, 2.4 u/tick — an
    // ordinary strike.
    a_shot_he_has_already_been_beaten_by(
        &mut field,
        Vector3::new(804.0, 264.0, 1.10),
        Vector3::new(2.4, 0.0, 0.0),
    );

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    for _ in 0..20 {
        field.ball.update_light(&mut context, &players, &mut events);
    }
    assert!(
        field.ball.position.x < keeper_at.x,
        "the ball finished at x = {:.1}, behind a keeper standing on {:.1}",
        field.ball.position.x,
        keeper_at.x
    );
    assert_eq!(
        field.ball.last_touch_player_id,
        Some(keeper_id),
        "it came off him, so he is the last man to have touched it"
    );
}

/// …and it is a BODY, not a wall. A ball a metre and a half past his
/// shoulder is his hands' problem, and his hands have already been beaten:
/// it goes in.
#[test]
fn a_shot_past_his_shoulder_still_goes_in() {
    let (mut field, mut context) = kickoff();
    let keeper_at = Vector3::new(820.0, 264.0, 0.0);
    lone_keeper(&mut field, keeper_at);
    a_shot_he_has_already_been_beaten_by(
        &mut field,
        Vector3::new(804.0, 276.0, 1.10),
        Vector3::new(2.4, 0.0, 0.0),
    );

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    for _ in 0..20 {
        field.ball.update_light(&mut context, &players, &mut events);
    }
    assert!(
        field.ball.position.x > keeper_at.x,
        "a ball a metre and a half wide of him was stopped by his body at x = {:.1}",
        field.ball.position.x
    );
}

/// **A dive is a low, wide obstacle**, and this is the picture in the
/// report: he leaves his feet, the low drive is going across him half a
/// metre to his side, and it hits him. Standing in the same spot it goes
/// past — his hands would be its only chance, and they have already been
/// beaten.
///
/// The height is chosen at the level a body laid out flat actually
/// occupies, and that is not an arbitrary choice of test case: a keeper
/// airborne on his own dive apex has a real gap under him, and a ball
/// rolling flat along the grass genuinely does pass beneath a man in the
/// air. Which is football, and is why the block is a swept volume rather
/// than a lateral distance.
#[test]
fn a_diving_keeper_blocks_a_low_drive_where_a_standing_one_cannot() {
    let ball_y = 268.0; // half a metre to his side
    let ball_z = 0.35; // knee height — where a flat body is
    let strike = |diving: bool| {
        let (mut field, mut context) = kickoff();
        let keeper_at = Vector3::new(820.0, 264.0, 0.0);
        let keeper_id = lone_keeper(&mut field, keeper_at);
        if diving {
            let keeper = keeper_mut(&mut field, keeper_id);
            keeper.transition_to(
                PlayerState::Goalkeeper(GoalkeeperState::Diving),
                TransitionSource::EventHandler,
            );
            keeper.in_state_time = 12; // past the extension: all the way over
            keeper.height = 0.16; // the engine's own dive apex floor
            keeper.velocity = Vector3::new(0.0, 0.6, 0.0); // going that way
        }
        a_shot_he_has_already_been_beaten_by(
            &mut field,
            Vector3::new(804.0, ball_y, ball_z),
            Vector3::new(2.4, 0.0, 0.0),
        );
        let players = field.players.clone();
        let mut events = EventCollection::with_capacity(8);
        for _ in 0..20 {
            field.ball.update_light(&mut context, &players, &mut events);
        }
        field.ball.position.x
    };
    let standing_x = strike(false);
    let diving_x = strike(true);
    assert!(
        standing_x > 820.0,
        "it is half a metre past an upright man — it should not touch him (x = {standing_x:.1})"
    );
    assert!(
        diving_x < 820.0,
        "he is laid out flat across its path and it went through him (x = {diving_x:.1})"
    );
}

/// A ball ALREADY inside him is not a fresh contact — otherwise every parry
/// he makes bounces off his own chest for the four ticks it takes to clear
/// him, and a save turns into a keeper juggling the ball on the goal line.
#[test]
fn a_ball_leaving_his_hands_does_not_bounce_off_him_again() {
    let (mut field, mut context) = kickoff();
    let keeper_at = Vector3::new(820.0, 264.0, 0.0);
    let keeper_id = lone_keeper(&mut field, keeper_at);
    // Where a spilled parry leaves it: on him, moving away slowly.
    field.ball.position = Vector3::new(820.0, 264.0, 0.9);
    field.ball.velocity = Vector3::new(-0.9, 0.4, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(keeper_id);
    field.ball.flags.in_flight_state = 10;

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(8);
    let mut travelled = 0.0f32;
    for _ in 0..10 {
        let before = field.ball.position;
        field.ball.update_light(&mut context, &players, &mut events);
        travelled += (field.ball.position - before).xy().norm();
    }
    assert!(
        travelled > 3.0,
        "the ball he pushed away only got {travelled:.1}u — it is stuck against him"
    );
}

/// The posture the engine gives a diving keeper and the one the replay rig
/// draws have to be the same man.
///
/// Both pivot a dive about the hips and both settle them from
/// `Physique::HIP` to `Carriage::LYING` as he goes over. This pins the
/// engine's half of that, because the two live in different crates and
/// nothing else can fail when they drift: the sim would decide the ball
/// missed him while the viewer drew it going through his chest, which is
/// the bug this whole module exists for, one layer further down.
#[test]
fn the_engine_and_the_replay_agree_about_where_a_diving_keeper_is() {
    let (mut field, _) = kickoff();
    let keeper_id = lone_keeper(&mut field, Vector3::new(820.0, 264.0, 0.0));
    let keeper = keeper_mut(&mut field, keeper_id);
    keeper.transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Diving),
        TransitionSource::EventHandler,
    );
    keeper.in_state_time = 12; // past `TOPPLE_TICKS`: all the way over
    keeper.velocity = Vector3::new(0.0, 1.0, 0.0);

    // On the deck: hips a hand's width up, the body laid out across the
    // goal, and nothing of him above knee height.
    keeper.height = 0.0;
    let flat = KeeperBody::of(keeper);
    let (top, bottom) = flat.envelope();
    assert!(
        (0.35..0.45).contains(&top),
        "a keeper lying on the grass reaches {top:.2} m"
    );
    assert!(
        bottom < 0.05,
        "and his underside is on it, at {bottom:.2} m"
    );
    assert!(
        flat.reach_across() > 1.5,
        "flat out he is {:.2} m across the goal",
        flat.reach_across()
    );

    // …and on his feet he is the same man stood up: tall, and no wider
    // than a man.
    let keeper = keeper_mut(&mut field, keeper_id);
    keeper.transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Standing),
        TransitionSource::EventHandler,
    );
    let upright = KeeperBody::of(keeper);
    let (top, bottom) = upright.envelope();
    assert!(
        (1.75..1.90).contains(&top),
        "standing he is {top:.2} m tall"
    );
    assert!(bottom.abs() < 0.05, "with his soles on the grass");
    assert!(
        upright.reach_across() < 0.7,
        "upright he is {:.2} m across",
        upright.reach_across()
    );
}
