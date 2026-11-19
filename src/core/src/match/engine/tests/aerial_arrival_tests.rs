//! **A won delivery arrives at the head's reach.** The contest has given a
//! man the header; the ball reaches him when it is within his head's reach
//! above the boot, not within a boot's reach, and a ball that comes down to
//! boot height before that is nobody's award.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{AerialDelivery, AerialOutcome};
use crate::r#match::engine::result::Score;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, events::EventCollection,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
const DRIFT: Vector3<f32> = Vector3::new(0.12, 0.0, -0.02);

/// A delivery the contest has given a left-side forward, dropping at
/// `height` m with the forward `gap` units from it. Everyone else is far
/// away.
fn delivery(gap: f32, height: f32) -> (MatchField, MatchContext, u32) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    let winner = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    let at = Vector3::new(740.0, 272.0, height);
    for p in field.players.iter_mut() {
        p.velocity = Vector3::zeros();
        p.position = if p.id == winner {
            Vector3::new(at.x + gap, at.y, 0.0)
        } else {
            Vector3::new(60.0, 40.0, 0.0)
        };
    }
    let tick = context.current_tick();
    let ball = &mut field.ball;
    ball.current_tick_cached = tick;
    ball.position = at;
    ball.velocity = Vector3::new(0.0, 0.0, -0.03);
    ball.current_owner = None;
    ball.aerial_contest_winner = Some(winner);
    ball.flags.in_flight_state = 50;
    ball.aerial_delivery = Some(AerialDelivery {
        winner_id: winner,
        target: Vector3::new(at.x, at.y, 2.4),
        outcome: AerialOutcome::Header { drift: DRIFT },
        arrival_height: 2.4,
        deadline_tick: tick + 500,
        force_heading: true,
    });
    (field, context, winner)
}

fn tick(field: &mut MatchField, context: &mut MatchContext) {
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field.ball.update_light(context, &players, &mut events);
}

#[test]
fn a_won_cross_within_a_heads_reach_reaches_him() {
    let (mut field, mut context, winner) = delivery(4.0, 2.0);
    tick(&mut field, &mut context);
    assert_eq!(
        field.ball.pending_aerial_strike,
        Some(winner),
        "handed to his head"
    );
}

#[test]
fn a_won_cross_a_stride_and_a_half_away_is_not_headed_or_re_aimed() {
    let (mut field, mut context, _) = delivery(12.0, 2.0);
    tick(&mut field, &mut context);
    assert!(
        field.ball.pending_aerial_strike.is_none(),
        "1.5 m is beyond a head"
    );
    assert!(
        field.ball.aerial_delivery.is_some(),
        "still on its way to him"
    );
    let across = field.ball.velocity.xy().norm();
    assert!(
        across < 1.0e-3,
        "the ball is not turned toward him: {across}"
    );
}

#[test]
fn a_ball_that_drops_to_boot_height_is_released() {
    let (mut field, mut context, _) = delivery(12.0, 1.40);
    tick(&mut field, &mut context);
    assert!(field.ball.aerial_delivery.is_none(), "no longer a delivery");
    assert!(field.ball.aerial_contest_winner.is_none(), "nobody's award");
}
