use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

/// Ticks of wind-up before the ball is struck. Matches the midfielder /
/// forward crossing states so the delivery cadence is role-independent.
const CROSS_EXECUTION_TIME: u64 = 5;
/// A target must be this close to the opponent goal to be worth aiming
/// at — i.e. in or arriving into the box.
const TARGET_BOX_RANGE: f32 = 150.0;
/// Opponents within this perpendicular distance of the delivery line
/// count as being "in the path".
const PATH_BLOCK_RADIUS: f32 = 6.0;

/// The overlapping fullback's delivery.
///
/// Defenders had no crossing state at all: a fullback who had carried the
/// ball into the attacking third could only pass, shoot or keep running,
/// so the single most common source of chances in modern football — the
/// overlap and cross — was unreachable from the defender state machine.
/// `PushingUp` and `Running` now hand off here when a wide defender wins
/// an advanced position with the ball.
#[derive(Default, Clone)]
pub struct DefenderCrossingState {}

impl StateProcessingHandler for DefenderCrossingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Lost it in the wind-up — a fullback caught upfield gets back.
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TrackingBack,
            ));
        }

        if ctx.in_state_time > CROSS_EXECUTION_TIME {
            if let Some(target) = self.find_cross_target(ctx) {
                // Delivered — the fullback immediately recovers their
                // shape rather than loitering on the byline.
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::TrackingBack,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason("DEF_OVERLAP_CROSS")
                            .build(ctx),
                    )),
                ));
            }

            // Nobody in the box — recycle rather than hit a hopeful ball.
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Passing,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Planted over the ball for the delivery.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Whipping in a cross after an overlapping run is explosive work.
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderCrossingState {
    /// Best teammate in or arriving into the box.
    ///
    /// Scored on heading ability, closeness to the goal, how tightly the
    /// target is marked, and how congested the delivery line is — the
    /// same shape as the midfielder's cross evaluation, minus the corner
    /// routine bias (a fullback overlap is open play by construction).
    fn find_cross_target<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let goal_pos = ctx.player().opponent_goal_position();
        let mut best: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().all() {
            if teammate.id == ctx.player.id {
                continue;
            }

            let dist_to_goal = (teammate.position - goal_pos).magnitude();
            if dist_to_goal > TARGET_BOX_RANGE {
                continue;
            }

            let pass_vector = teammate.position - ctx.player.position;
            let pass_distance = pass_vector.magnitude();
            if pass_distance < 1.0 {
                continue;
            }
            let pass_direction = pass_vector / pass_distance;

            let opponents_in_path = ctx
                .players()
                .opponents()
                .all()
                .filter(|opponent| {
                    let to_opponent = opponent.position - ctx.player.position;
                    let projection = to_opponent.dot(&pass_direction);
                    if projection <= 0.0 || projection >= pass_distance {
                        return false;
                    }
                    let projected_point = ctx.player.position + pass_direction * projection;
                    (opponent.position - projected_point).magnitude() < PATH_BLOCK_RADIUS
                })
                .count();

            if opponents_in_path >= 2 {
                continue;
            }

            let heading_skill = ctx
                .context
                .players
                .by_id(teammate.id)
                .map(|p| p.skills.technical.heading)
                .unwrap_or(10.0);

            let close_opponents = ctx.tick_context.grid.opponents(teammate.id, 8.0).count();
            let marking_penalty = match close_opponents {
                0 => 1.0,
                1 => 0.6,
                _ => 0.25,
            };
            let path_penalty = if opponents_in_path == 1 { 0.6 } else { 1.0 };

            let score = (heading_skill + (TARGET_BOX_RANGE - dist_to_goal) / 10.0)
                * marking_penalty
                * path_penalty;

            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((teammate, score));
            }
        }

        best.map(|(t, _)| t)
    }
}
