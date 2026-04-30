use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderPassingState {}

impl StateProcessingHandler for DefenderPassingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the defender still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to appropriate state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // Under heavy pressure — prefer a safe pass, any safe pass. The
        // old rule "clear if safe pass < 20u" was too eager; a short
        // safe pass is still a ball-retention win. Only escalate to
        // Clearing when truly no safe pass exists. Bulk of the 80+
        // clearances per match came from this branch firing whenever a
        // short passing option was available but too close.
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
                // No safe option at all — hoof it.
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ))
            };
        }

        // If teammates are tired, prefer a safe short pass
        if self.are_teammates_tired(ctx) {
            if let Some(safe_target) = ctx
                .player()
                .passing()
                .find_safe_pass_option_with_distance(100.0)
            {
                let dist = (safe_target.position - ctx.player.position).magnitude();
                if dist >= 20.0 {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("DEF_PASSING_TIRED_SHORT")
                                .build(ctx),
                        )),
                    ));
                }
            }
        }

        // Normal passing situation - evaluate options more carefully
        // Defenders use shorter max distance (200 units) to avoid wild long passes
        if let Some((best_target, _reason)) = ctx
            .player()
            .passing()
            .find_best_pass_option_with_distance(200.0)
        {
            // ANTI-LOOP: Ensure pass target is far enough away for the ball to actually reach them.
            // Very short passes (< 30 units) with low pass force create claim-pass-reclaim loops.
            let pass_distance = (best_target.position - ctx.player.position).magnitude();

            // Also verify the pass isn't going backward toward own goal
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - ctx.player.position).normalize();
            let to_target = (best_target.position - ctx.player.position).normalize();
            let forward_component = to_target.dot(&to_goal);

            if pass_distance >= 30.0 && forward_component > -0.3 {
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
        }

        // If no good passing option and close to own goal, consider clearing
        if ctx.player().defensive().in_dangerous_position() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Clearing,
            ));
        }

        // If viable to dribble out of pressure (wait before bailing to prevent Running↔Passing oscillation)
        if ctx.in_state_time > 20 && self.can_dribble_effectively(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Time-based fallback - don't get stuck in this state too long
        if ctx.in_state_time > 50 {
            // If we've been in this state for a while, make a decision

            // Try to find a safe pass option (directionally aware) rather than any random teammate
            if let Some(safe_target) = ctx
                .player()
                .passing()
                .find_safe_pass_option_with_distance(200.0)
            {
                let dist = (safe_target.position - ctx.player.position).magnitude();
                if dist >= 20.0 {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("DEF_PASSING_TIMEOUT")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // If no safe option, clear the ball rather than making a wild pass
            if ctx.in_state_time > 65 {
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

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // While holding the ball and looking for pass options, move slowly or stand still

        // If player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            // Calculate target position based on the defensive situation
            if let Some(target_position) =
                ctx.player().movement().calculate_better_passing_position()
            {
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

        let under_immediate_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let has_clear_option = ctx.player().passing().find_best_pass_option().is_some();

        // Adjust position if not under immediate pressure and no clear options
        !under_immediate_pressure && !has_clear_option
    }

    /// Check if nearby teammates are tired (average condition below threshold)
    fn are_teammates_tired(&self, ctx: &StateProcessingContext) -> bool {
        let mut total_condition = 0u32;
        let mut count = 0u32;

        for teammate in ctx.players().teammates().nearby(150.0) {
            if let Some(player) = ctx.context.players.by_id(teammate.id) {
                total_condition += player.player_attributes.condition_percentage();
                count += 1;
            }
        }

        if count == 0 {
            return false;
        }

        let avg_condition = total_condition / count;
        avg_condition < 40
    }
}
