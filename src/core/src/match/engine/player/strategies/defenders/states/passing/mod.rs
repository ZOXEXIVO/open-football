use crate::r#match::events::Event;
use crate::r#match::defenders::states::DefenderState;
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
        if self.is_under_heavy_pressure(ctx) {
            if let Some(safe_option) = self.find_safe_pass_option(ctx) {
                // Execute a quick, safe pass
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::build()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(safe_option.id)
                            .with_target(safe_option.position)
                            .with_force(ctx.player().pass_teammate_power(safe_option.id) * 1.2) // Slightly more power for urgency
                            .build(),
                    )),
                ));
            } else {
                // No safe option, clear the ball
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }
        }

        // Normal passing situation - evaluate options more carefully
        if let Some(best_target) = self.find_best_pass_option(ctx) {
            // Execute the pass
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(best_target.id)
                        .with_target(best_target.position)
                        .with_force(ctx.player().pass_teammate_power(best_target.id))
                        .build(),
                )),
            ));
        }

        // If no good passing option and close to own goal, consider clearing
        if self.is_in_dangerous_position(ctx) {
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

        // Continue seeking passing options or adjust position
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            // Calculate target position based on the defensive situation
            if let Some(target_position) = self.calculate_better_passing_position(ctx) {
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

        // Default to minimal movement while preparing to pass
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl DefenderPassingState {
    /// Find the best pass option using an improved evaluation system
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let teammates = ctx.players().teammates();

        // Use player's vision skill to determine range
        let vision_skill = ctx.player.skills.mental.vision;
        let vision_range = vision_skill * 15.0; // Adjust range based on skill

        // Get viable passing options within range
        let pass_options: Vec<MatchPlayerLite> = teammates
            .nearby(vision_range)
            .filter(|t| self.is_viable_pass_target(ctx, t))
            .collect();

        if pass_options.is_empty() {
            return None;
        }

        // Evaluate each option using the pass evaluator
        pass_options.into_iter()
            .map(|teammate| {
                let score = PassEvaluator::evaluate_pass(ctx, &teammate, 100.0);
                // Defenders should prioritize safety more than midfielders
                let adjusted_score = if self.is_safer_pass(ctx, &teammate) {
                    score * 1.3 // Boost safer passes
                } else {
                    score
                };
                (teammate, adjusted_score)
            })
            .max_by(|(_, score_a), (_, score_b)| {
                score_a.partial_cmp(score_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(teammate, _)| teammate)
    }

    /// Find a safe pass option when under pressure
    fn find_safe_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let teammates = ctx.players().teammates();

        // Prioritize closest teammates with clear passing lanes
        let safe_options: Vec<MatchPlayerLite> = teammates
            .nearby(50.0) // Closer range for urgent passes
            .filter(|t| ctx.player().has_clear_pass(t.id) && !self.is_under_pressure(ctx, t))
            .collect();

        // Find the safest option by direction and pressure
        safe_options.into_iter()
            .min_by(|a, b| {
                // Compare how "away from danger" the pass would be
                let a_safety = self.calculate_pass_safety(ctx, a);
                let b_safety = self.calculate_pass_safety(ctx, b);
                b_safety.partial_cmp(&a_safety).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Calculate how safe a pass would be based on direction and receiver situation
    fn calculate_pass_safety(&self, ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        // Get vectors for calculations
        let pass_vector = target.position - ctx.player.position;
        let to_own_goal = ctx.ball().direction_to_own_goal() - ctx.player.position;

        // Calculate how much this pass moves away from own goal (higher is better)
        let pass_away_from_goal = -(pass_vector.normalize().dot(&to_own_goal.normalize()));

        // Calculate space around target player
        let space_factor = 1.0 - (ctx.players().opponents()
            .nearby(15.0)
            .filter(|o| (o.position - target.position).magnitude() < 10.0)
            .count() as f32 * 0.2).min(0.8);

        // Return combined safety score
        pass_away_from_goal + space_factor
    }

    /// Check if a teammate is viable for receiving a pass
    fn is_viable_pass_target(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Basic viability criteria
        let has_clear_lane = ctx.player().has_clear_pass(teammate.id);
        let not_dangerous_position = !self.is_in_dangerous_area(ctx, teammate);

        has_clear_lane && not_dangerous_position
    }


    /// Check if a target is in a dangerous position near our goal
    fn is_in_dangerous_area(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let goal_position = ctx.ball().direction_to_own_goal();
        let distance_to_goal = (teammate.position - goal_position).magnitude();

        // Consider danger zone as 20% of field width from own goal
        let danger_threshold = ctx.context.field_size.width as f32 * 0.2;

        distance_to_goal < danger_threshold
    }

    /// Check if a pass is safer (away from pressure and toward team's attacking side)
    fn is_safer_pass(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let player_pos = ctx.player.position;
        let teammate_pos = teammate.position;
        let own_goal = ctx.ball().direction_to_own_goal();

        // Direction vectors
        let to_teammate = (teammate_pos - player_pos).normalize();
        let to_own_goal = (own_goal - player_pos).normalize();

        // Passes that move away from own goal are safer
        let moving_away_from_goal = to_teammate.dot(&to_own_goal) < -0.3;

        // Passes to less pressured areas are safer
        let teammates_pressure = ctx.players().opponents().all()
            .filter(|o| (o.position - teammate_pos).magnitude() < 15.0)
            .count();

        let current_pressure = ctx.players().opponents().all()
            .filter(|o| (o.position - player_pos).magnitude() < 10.0)
            .count();

        moving_away_from_goal && (teammates_pressure < current_pressure)
    }

    /// Check if player is under heavy pressure from opponents
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        const PRESSURE_DISTANCE: f32 = 8.0;
        const PRESSURE_THRESHOLD: usize = 2;

        let pressing_opponents = ctx.players().opponents().nearby(PRESSURE_DISTANCE).count();
        pressing_opponents >= PRESSURE_THRESHOLD
    }

    /// Check if teammate is under pressure
    fn is_under_pressure(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        ctx.players().opponents().all()
            .filter(|o| (o.position - teammate.position).magnitude() < 10.0)
            .count() >= 1
    }

    /// Determine if player is in a dangerous position (near own goal)
    fn is_in_dangerous_position(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = (ctx.player.position - ctx.ball().direction_to_own_goal()).magnitude();
        let danger_threshold = ctx.context.field_size.width as f32 * 0.15; // 15% of field width

        distance_to_goal < danger_threshold
    }

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
        let under_immediate_pressure = ctx.players().opponents().exists(5.0);
        let has_clear_option = self.find_best_pass_option(ctx).is_some();

        // Adjust position if not under immediate pressure and no clear options
        !under_immediate_pressure && !has_clear_option
    }

    /// Calculate a better position for finding passing angles
    fn calculate_better_passing_position(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Get current position and key references
        let player_pos = ctx.player.position;
        let goal_pos = ctx.ball().direction_to_own_goal();

        // Find positions of nearby opponents creating pressure
        let nearby_opponents: Vec<MatchPlayerLite> = ctx.players()
            .opponents()
            .nearby(15.0)
            .collect();

        if nearby_opponents.is_empty() {
            return None;
        }

        // Calculate average position of pressing opponents
        let avg_opponent_pos = nearby_opponents.iter()
            .fold(Vector3::zeros(), |acc, p| acc + p.position)
            / nearby_opponents.len() as f32;

        // Calculate direction away from pressure and perpendicular to goal line
        let away_from_pressure = (player_pos - avg_opponent_pos).normalize();
        let to_goal = (goal_pos - player_pos).normalize();

        // Create a movement perpendicular to goal line
        let perpendicular = Vector3::new(-to_goal.y, to_goal.x, 0.0).normalize();

        // Blend the two directions (more weight to away from pressure)
        let direction = (away_from_pressure * 0.7 + perpendicular * 0.3).normalize();

        // Move slightly in the calculated direction
        Some(player_pos + direction * 5.0)
    }
}