//! Where a goal is booked in the distance census.
//!
//! A goal that goes in straight off a parry or a deflection is the
//! original strike's goal and is booked at that strike's distance. A
//! rebound that somebody hits again is a new strike, and the census must
//! book the goal at the second strike's distance rather than the first.

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::GoalOrigin;
use crate::r#match::engine::result::Score;
use crate::r#match::events::{EventCollection, EventDispatcher};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ops::xg::ShotType;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, ResultMatchPositionData,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
/// 6 m in game units — inside the six-yard box.
const SIX_YARDS: f32 = 48.0;
/// 16.5 m — the edge of the penalty area.
const BOX_EDGE: f32 = 132.0;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

fn home_outfielders(field: &MatchField) -> Vec<u32> {
    field
        .players
        .iter()
        .filter(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .collect()
}

/// Put the ball at `shooter`'s feet at `spot` and strike it at the
/// right-hand goal, which the Left side attacks.
fn strike_from(
    field: &mut MatchField,
    context: &mut MatchContext,
    shooter: u32,
    spot: Vector3<f32>,
) {
    if let Some(player) = field.get_player_mut(shooter) {
        player.position = spot;
    }
    field.ball.position = Vector3::new(spot.x, spot.y, 0.1);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(shooter);
    field.ball.previous_owner = None;

    let mut events = EventCollection::with_capacity(4);
    events.add_player_event(PlayerEvent::Shoot(ShootingEventContext {
        from_player_id: shooter,
        target: Vector3::new(840.0, 272.0, 1.0),
        force: 1.0,
        reason: "goal_distance_tests",
        tick: context.current_tick(),
        shot_type: ShotType::FootOpenPlay,
    }));
    let mut match_data = ResultMatchPositionData::empty();
    EventDispatcher::dispatch(&mut events, field, context, &mut match_data, true);
}

#[test]
fn a_rebound_hit_again_is_booked_at_the_rebound_distance() {
    let (mut field, mut context) = kickoff();
    let shooters = home_outfielders(&field);
    let (long_range, follow_up) = (shooters[0], shooters[1]);

    strike_from(
        &mut field,
        &mut context,
        long_range,
        Vector3::new(600.0, 272.0, 0.0),
    );
    assert!(
        field.ball.last_shot_struck_dist > BOX_EDGE,
        "the first strike is from outside the box, got {:.1}u",
        field.ball.last_shot_struck_dist
    );

    // The keeper has parried it back out to the six-yard line, and the
    // follow-up man hits it again.
    context.total_match_time += 600;
    strike_from(
        &mut field,
        &mut context,
        follow_up,
        Vector3::new(800.0, 280.0, 0.0),
    );
    assert!(
        field.ball.last_shot_struck_dist < SIX_YARDS,
        "the rebound is booked where it was hit from, got {:.1}u",
        field.ball.last_shot_struck_dist
    );
}

#[test]
fn an_own_goal_is_filed_as_an_own_goal_whatever_the_phase() {
    let (field, _) = kickoff();
    assert_eq!(field.ball.goal_origin(true), GoalOrigin::OwnGoal);
    assert_eq!(field.ball.goal_origin(false), GoalOrigin::OpenPlay);
}
