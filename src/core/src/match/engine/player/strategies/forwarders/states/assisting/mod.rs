use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

#[derive(Default, Clone)]
pub struct ForwardAssistingState {}

impl StateProcessingHandler for ForwardAssistingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.9) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Check if the player is on the opponent's side of the field
        if ctx.team().is_control_ball()
            && !ctx.player().on_own_side()
            && ctx.players().opponents().exists(100.0)
        {
            // If not on the opponent's side, focus on creating space and moving forward
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
            ));
        }

        // Check if there's an immediate threat from an opponent
        if self.is_under_pressure(ctx) {
            // If under high pressure, decide between quick pass or dribbling
            if self.should_make_quick_pass(ctx) {
                if let Some(_teammate_id) = self.find_best_teammate_to_assist(ctx) {
                    //result.events.add_player_event(PlayerEvent::Pass(ctx.player.player_id, teammate_id));
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Dribbling,
                    ));
                }
            }
            // If no good passing option, try to dribble
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // If not under immediate pressure, look for assist opportunities
        if let Some(_) = self.find_best_teammate_to_assist(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        if self.is_in_shooting_range(ctx) && ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
        } else if self.should_create_space(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Assisting is moderate intensity - supporting movement
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl ForwardAssistingState {
    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_immediate_pressure()
    }

    fn should_make_quick_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Quick-pass willingness blends passing and decision-making
        // smoothly across 1-20. Sigmoid pivots (13/12) match the
        // "competent passer" / "decent decision-maker" intent of the
        // original `> 70/65` gate (which was bugged for the 1-20 scale
        // and never fired). Product of two curves so both skills matter.
        let pass_p = SkillCurve::new(ctx.player.skills.technical.passing, 13.0, 0.6).probability();
        let dec_p = SkillCurve::new(ctx.player.skills.mental.decisions, 12.0, 0.6).probability();
        rand::random::<f32>() < pass_p * dec_p
    }

    fn find_best_teammate_to_assist(&self, ctx: &StateProcessingContext) -> Option<u32> {
        ctx.players()
            .teammates()
            .nearby_ids(200.0)
            .filter(|(id, _)| self.is_in_good_scoring_position(ctx, *id))
            .min_by(|(_, dist_a), (_, dist_b)| {
                dist_a.partial_cmp(dist_b).unwrap_or(Ordering::Equal)
            })
            .map(|(id, _)| id)
    }

    fn is_in_good_scoring_position(&self, ctx: &StateProcessingContext, player_id: u32) -> bool {
        // Find the teammate's actual position
        if let Some(teammate) = ctx.players().teammates().all().find(|p| p.id == player_id) {
            let goal_pos = ctx.player().opponent_goal_position();
            let distance_to_goal = (teammate.position - goal_pos).magnitude();

            // Good scoring position: within 35m of goal
            if distance_to_goal > 350.0 {
                return false;
            }

            // Check if teammate has space (not heavily marked)
            let close_defenders = ctx.tick_context.grid.opponents(teammate.id, 10.0).count();

            // Good if close to goal with some space or is another forward
            distance_to_goal < 350.0
                && (close_defenders < 2 || teammate.tactical_positions.is_forward())
        } else {
            false
        }
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal < 25.0
    }

    fn should_create_space(&self, ctx: &StateProcessingContext) -> bool {
        if !ctx.players().teammates().exists(100.0) {
            return false;
        }
        // Off-the-ball skill scales the urge to create space smoothly —
        // elite movers (15+) almost always do it; ordinary 10s do it
        // sometimes; very poor (sub-5) almost never.
        let p = SkillCurve::new(ctx.player.skills.mental.off_the_ball, 15.0, 0.6).probability();
        rand::random::<f32>() < p
    }
}
