use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct DefenderPassingState {}

impl StateProcessingHandler for DefenderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the defender still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to appropriate state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // Under heavy pressure - make a quick decision
        if ctx.player().pressure().is_under_heavy_pressure() {
            return if let Some(safe_option) = ctx.player().passing().find_safe_pass_option() {
                Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(safe_option.id)
                            .with_reason("DEF_PASSING_UNDER_PRESSURE")
                            .build(ctx),
                    )),
                ))
            } else {
                // No safe option, clear the ball
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ))
            };
        }

        // Normal passing situation - evaluate options more carefully
        if let Some((best_target, _reason)) = ctx.player().passing().find_best_pass_option() {
            // Execute the pass
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(best_target.id)
                        .with_reason("DEF_PASSING_NORMAL")
                        .build(ctx),
                )),
            ));
        }

        // If no good passing option and close to own goal, consider clearing
        if ctx.player().defensive().in_dangerous_position() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Clearing,
            ));
        }

        // If viable to dribble out of pressure
        if self.can_dribble_effectively(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Time-based fallback - don't get stuck in this state too long
        if ctx.in_state_time > 50 {
            // If we've been in this state for a while, make a decision

            // Try to find ANY teammate to pass to
            if let Some(any_teammate) = ctx.player().passing().find_any_teammate() {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(any_teammate.id)
                            .with_reason("DEF_PASSING_TIMEOUT")
                            .build(ctx),
                    )),
                ));
            }

            // If no teammates at all, clear the ball
            if ctx.in_state_time > 75 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            // Otherwise start running with the ball
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // While holding the ball and looking for pass options, move slowly or stand still

        // If player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            // Calculate target position based on the defensive situation
            if let Some(target_position) = ctx.player().movement().calculate_better_passing_position() {
                return Some(
                    SteeringBehavior::Arrive {
                        target: target_position,
                        slowing_distance: 5.0, // Short distance for subtle movement
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }

        // Default to very slow movement or stationary
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing is a quick action with minimal physical effort - very low intensity
        DefenderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl DefenderPassingState {

    /// Determine if player can effectively dribble out of the current situation
    fn can_dribble_effectively(&self, ctx: &StateProcessingContext) -> bool {
        // Check player's dribbling skill
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;

        // Check if there's space to dribble into
        let opposition_ahead = ctx.players().opponents().nearby(20.0).count();

        // Defenders typically need more space and skill to dribble effectively
        dribbling_skill > 0.8 && opposition_ahead < 1
    }

    /// Determine if player should adjust position to find better passing angles
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        // Don't adjust if we've been in state too long
        if ctx.in_state_time > 40 {
            return false;
        }

        let under_immediate_pressure = ctx.players().opponents().exists(5.0);
        let has_clear_option = ctx.player().passing().find_best_pass_option().is_some();

        // Adjust position if not under immediate pressure and no clear options
        !under_immediate_pressure && !has_clear_option
    }
}