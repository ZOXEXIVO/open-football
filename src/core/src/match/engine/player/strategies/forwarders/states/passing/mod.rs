use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_PASS_DURATION: u64 = 100; // Ticks before considering fatigue

#[derive(Default)]
pub struct ForwardPassingState {}

impl StateProcessingHandler for ForwardPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Running state
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Determine the best teammate to pass to
        if let Some(target_teammate) = self.find_best_pass_option(ctx) {
            // Execute the pass
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target_teammate.id)
                        .build(ctx),
                )),
            ));
        }

        // If no good passing option is found and we're close to goal, consid-er shooting
        if ctx.ball().distance_to_opponent_goal() < 250.0 && self.should_shoot_instead_of_pass(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
        }

        // If under excessive pressure, consider going back to dribbling
        if self.is_under_heavy_pressure(ctx)  {
            if self.can_dribble_effectively(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            } else {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Running,
                ));
            }
        }

        if ctx.in_state_time > MAX_PASS_DURATION {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If the player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            // Look for space to move into
            return Some(
                SteeringBehavior::Arrive {
                    target: self.calculate_better_passing_position(ctx),
                    slowing_distance: 30.0,
                }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
            );
        }

        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardPassingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let teammates = ctx.players().teammates();

        // Use player's vision skill to determine range
        let vision_range = ctx.player.skills.mental.vision * 15.0;

        // Get viable passing options within range
        let pass_options: Vec<MatchPlayerLite> = teammates
            .nearby(vision_range)
            .filter(|t| self.is_viable_pass_target(ctx, t))
            .collect();

        if pass_options.is_empty() {
            return None;
        }

        // Evaluate each option - forwards prioritize different passes than other positions
        pass_options.into_iter()
            .map(|teammate| {
                let score = self.evaluate_forward_pass(ctx, &teammate);
                (teammate, score)
            })
            .max_by(|(_, score_a), (_, score_b)| {
                score_a.partial_cmp(score_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(teammate, _)| teammate)
    }

    /// Forward-specific pass evaluation - prioritizing attacks and goal scoring opportunities
    fn evaluate_forward_pass(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> f32 {
        // Start with the basic pass evaluator score
        let base_score = PassEvaluator::evaluate_pass(ctx, teammate, 100.0);

        // Forward-specific factors - much more goal-oriented than midfielders
        let mut score = base_score;

        // Goal distance factors - forwards prioritize passes that get closer to goal
        let forward_to_goal_dist = ctx.ball().distance_to_opponent_goal();
        let teammate_to_goal_dist = (teammate.position - ctx.player().opponent_goal_position()).magnitude();

        // Significantly boost passes that advance toward goal - key forward priority
        if teammate_to_goal_dist < forward_to_goal_dist {
            score += 30.0 * (1.0 - (teammate_to_goal_dist / forward_to_goal_dist));
        }

        // Boost for passes to other forwards (likely in better scoring positions)
        if teammate.tactical_positions.is_forward() {
            score += 20.0;
        }

        // Boost for passes that break defensive lines
        if self.pass_breaks_defensive_line(ctx, teammate) {
            score += 25.0;
        }

        // Heavy bonus for teammates who have a clear shot on goal - key forward priority
        if self.teammate_has_clear_shot(ctx, teammate) {
            score += 35.0;
        }

        // Small penalty for backwards passes unless under heavy pressure
        if teammate.position.x < ctx.player.position.x && !self.is_under_heavy_pressure(ctx) {
            score -= 15.0;
        }

        score
    }

    /// Check if a pass to this teammate would break through a defensive line
    fn pass_breaks_defensive_line(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite
    ) -> bool {
        let player_pos = ctx.player.position;
        let teammate_pos = teammate.position;

        // Create a line between player and teammate
        let pass_direction = (teammate_pos - player_pos).normalize();
        let pass_distance = (teammate_pos - player_pos).magnitude();

        // Look for opponents between the player and teammate
        let opponents_in_line = ctx.players().opponents().all()
            .filter(|opponent| {
                // Project opponent onto pass line
                let to_opponent = opponent.position - player_pos;
                let projection_distance = to_opponent.dot(&pass_direction);

                // Check if opponent is between player and teammate
                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                // Calculate perpendicular distance to pass line
                let projected_point = player_pos + pass_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).magnitude();

                // Consider opponents close to passing lane
                perp_distance < 3.0
            })
            .count();

        // If there are opponents in the passing lane, this pass breaks a line
        opponents_in_line > 0
    }

    /// Check if a teammate is viable for receiving a pass
    fn is_viable_pass_target(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Basic viability criteria
        let has_clear_lane = ctx.player().has_clear_pass(teammate.id);
        let not_heavily_marked = !self.is_heavily_marked(ctx, teammate);

        // Forwards are more aggressive with passing - they care less about position
        // and more about goal scoring opportunities
        let creates_opportunity = self.pass_creates_opportunity(ctx, teammate);

        has_clear_lane && not_heavily_marked && creates_opportunity
    }

    /// Check if a pass would create a good attacking opportunity
    fn pass_creates_opportunity(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Passing backwards is generally not a good option for forwards
        // unless under heavy pressure
        if teammate.position.x < ctx.player.position.x && !self.is_under_heavy_pressure(ctx) {
            return false;
        }

        // Check if the teammate is in a good shooting position
        let distance_to_goal = (teammate.position - ctx.player().opponent_goal_position()).magnitude();

        if distance_to_goal < 200.0 {
            return true;
        }

        // Check if the teammate has space to advance
        let space_around_teammate = self.calculate_space_around_player(ctx, teammate);
        if space_around_teammate > 7.0 {
            return true;
        }

        // Default - other passes may still be viable but lower priority
        true
    }
    
    /// Check if a player is heavily marked by opponents
    fn is_heavily_marked(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        const MARKING_DISTANCE: f32 = 5.0;
        const MAX_MARKERS: usize = 2;

        let markers = ctx.players().opponents().all()
            .filter(|opponent| {
                (opponent.position - teammate.position).magnitude() <= MARKING_DISTANCE
            })
            .count();

        markers >= MAX_MARKERS
    }

    /// Determine if teammate has a clear shot on goal
    fn teammate_has_clear_shot(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let teammate_pos = teammate.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let shot_direction = (goal_pos - teammate_pos).normalize();
        let shot_distance = (goal_pos - teammate_pos).magnitude();

        let ray_cast_result = ctx.tick_context.space.cast_ray(
            teammate_pos,
            shot_direction,
            shot_distance,
            false,
        );

        ray_cast_result.is_none() && shot_distance < 250.0
    }

    /// Calculate the amount of space around a player
    fn calculate_space_around_player(
        &self,
        ctx: &StateProcessingContext,
        player: &MatchPlayerLite,
    ) -> f32 {
        let space_radius = 10.0;
        let num_opponents_nearby = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let distance = (opponent.position - player.position).magnitude();
                distance <= space_radius
            })
            .count();

        space_radius - num_opponents_nearby as f32
    }

    /// Check if player should shoot instead of pass
    fn should_shoot_instead_of_pass(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let shooting_skill = (ctx.player.skills.technical.finishing +
            ctx.player.skills.technical.technique) / 40.0;

        // Forwards are more likely to shoot than midfielders
        // They'll shoot from further out and with less clear sight of goal
        let shooting_range = 250.0 * shooting_skill;

        distance_to_goal < shooting_range && ctx.player().has_clear_shot()
    }

    /// Check if player is under heavy pressure from opponents
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        const PRESSURE_DISTANCE: f32 = 20.0; // Forwards consider closer pressure
        const PRESSURE_THRESHOLD: usize = 1; // Even one opponent is significant for forwards

        let pressing_opponents = ctx.players().opponents().nearby(PRESSURE_DISTANCE).count();
        pressing_opponents > PRESSURE_THRESHOLD
    }

    /// Determine if player can effectively dribble out of pressure
    fn can_dribble_effectively(&self, ctx: &StateProcessingContext) -> bool {
        // Forward players are generally better at dribbling under pressure
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;

        // Calculate combined dribbling effectiveness
        let dribbling_effectiveness = (dribbling_skill * 0.7) + (agility * 0.3);

        // Check if there's space to dribble into
        let has_space = !ctx.players().opponents().exists(15.0);

        dribbling_effectiveness > 0.5 && has_space
    }

    /// Determine if player should adjust position to find better passing angles
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        // If no good passing option and not under immediate pressure
        self.find_best_pass_option(ctx).is_none() && !self.is_under_heavy_pressure(ctx)
    }

    /// Calculate a better position for finding passing angles - forwards look for
    /// spaces that open up shooting opportunities first, passing lanes second
    fn calculate_better_passing_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Get positions
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // First priority: move to a better shooting position if possible
        if ctx.ball().distance_to_opponent_goal() < 250.0 {
            // Look for space between defenders toward goal
            if let Some(space) = self.find_space_between_opponents_toward_goal(ctx) {
                return space;
            }
        }

        // Second priority: find space for a better passing angle
        let closest_teammate = ctx.players().teammates().nearby(150.0).next();

        if let Some(teammate) = closest_teammate {
            // Find a position that improves angle to this teammate
            let to_teammate = teammate.position - player_pos;
            let teammate_direction = to_teammate.normalize();

            // Move slightly perpendicular to create a better angle
            let perpendicular = Vector3::new(-teammate_direction.y, teammate_direction.x, 0.0);
            let adjustment = perpendicular * 8.0; // More aggressive movement for forwards

            return player_pos + adjustment;
        }

        // Default to moving toward goal if no better option
        let to_goal = goal_pos - player_pos;
        let goal_direction = to_goal.normalize();
        player_pos + goal_direction * 10.0
    }

    /// Look for space between defenders toward the goal
    fn find_space_between_opponents_toward_goal(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal_direction = (goal_pos - player_pos).normalize();

        // Get opponents between player and goal
        let opponents_between = ctx.players().opponents().all()
            .filter(|opp| {
                let to_opp = opp.position - player_pos;
                let projection = to_opp.dot(&to_goal_direction);
                // Only consider opponents between player and goal
                projection > 0.0 && projection < (goal_pos - player_pos).magnitude()
            })
            .collect::<Vec<_>>();

        if opponents_between.len() < 2 {
            return None; // Not enough opponents to find a gap
        }

        // Find the pair of opponents with the largest gap between them
        let mut best_gap = None;
        let mut max_gap_width = 0.0;

        for i in 0..opponents_between.len() {
            for j in i+1..opponents_between.len() {
                let opp1 = &opponents_between[i];
                let opp2 = &opponents_between[j];

                let midpoint = (opp1.position + opp2.position) * 0.5;
                let gap_width = (opp1.position - opp2.position).magnitude();

                // Check if midpoint is roughly toward goal
                let to_midpoint = midpoint - player_pos;
                let dot_product = to_midpoint.dot(&to_goal_direction);

                if dot_product > 0.0 && gap_width > max_gap_width {
                    max_gap_width = gap_width;
                    best_gap = Some(midpoint);
                }
            }
        }

        // Return the midpoint of the largest gap
        best_gap
    }
}