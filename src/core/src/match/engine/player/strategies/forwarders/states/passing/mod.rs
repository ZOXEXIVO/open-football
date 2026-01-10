use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_PASS_DURATION: u64 = 30; // Ticks before trying alternative action (reduced for faster decision-making)
const MIN_POSITION_ADJUSTMENT_TIME: u64 = 5; // Minimum ticks before adjusting position (prevents immediate twitching)
const MAX_POSITION_ADJUSTMENT_TIME: u64 = 20; // Maximum ticks to spend adjusting position

#[derive(Default)]
pub struct ForwardPassingState {}

impl StateProcessingHandler for ForwardPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Running state
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // PRIORITY 1: Very close to goal - SHOOT instead of pass (inside 6-yard box)
        if distance_to_goal < 60.0 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
        }

        // PRIORITY 2: Inside penalty area - prefer shooting over passing
        if distance_to_goal < 165.0 {
            let finishing = ctx.player.skills.technical.finishing;
            let has_clear_shot = ctx.player().has_clear_shot();
            let close_blockers = ctx.players().opponents().nearby(5.0).count();

            // Good finishing skill or clear shot - shoot!
            if has_clear_shot || (finishing > 11.0 && close_blockers <= 1) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
        }

        // PRIORITY 3: Edge of box with clear shot - shoot
        if distance_to_goal < 250.0 && ctx.player().has_clear_shot() {
            let finishing = ctx.player.skills.technical.finishing;
            if finishing > 10.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
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
                        .with_reason("FWD_PASSING_STATE")
                        .build(ctx),
                )),
            ));
        }

        // If no good passing option is found - try shooting anyway if close
        if distance_to_goal < 300.0 && self.should_shoot_instead_of_pass(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
        }

        // If under excessive pressure, consider going back to dribbling
        if self.is_under_heavy_pressure(ctx) {
            // But in dangerous area - shoot!
            if distance_to_goal < 200.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
            if self.can_dribble_effectively(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            } else {
                return Some(StateChangeResult::with_forward_state(ForwardState::Running));
            }
        }

        if ctx.in_state_time > MAX_PASS_DURATION {
            // Timeout - if close to goal, shoot
            if distance_to_goal < 300.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
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
            let steering_velocity = SteeringBehavior::Arrive {
                target: self.calculate_better_passing_position(ctx),
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity;

            // Apply reduced separation to avoid interference with deliberate movement
            let separation = ctx.player().separation_velocity() * 0.3;

            return Some(steering_velocity + separation);
        }

        None
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing is low intensity - minimal fatigue
        ForwardCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl ForwardPassingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let teammates = ctx.players().teammates();

        // Use player's vision skill to determine range
        let vision_range = ctx.player.skills.mental.vision * 30.0;
        let vision_range_min = 100.0;

        // PRIORITY: First look for nearby forwards for quick combinations (15-60m range)
        let nearby_forwards: Vec<MatchPlayerLite> = teammates
            .nearby_range(15.0, 60.0)
            .filter(|t| t.tactical_positions.is_forward() && self.is_viable_pass_target(ctx, t))
            .collect();

        // If we have forwards nearby in good positions, prioritize them
        if !nearby_forwards.is_empty() {
            let best_forward = nearby_forwards
                .into_iter()
                .map(|teammate| {
                    let score = self.evaluate_forward_pass(ctx, &teammate);
                    (teammate, score)
                })
                .max_by(|(_, score_a), (_, score_b)| {
                    score_a.partial_cmp(score_b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(teammate, _)| teammate);

            if best_forward.is_some() {
                return best_forward;
            }
        }

        // Fallback: Get all viable passing options within range (reduced min from 50 to 20)
        let pass_options: Vec<MatchPlayerLite> = teammates
            .nearby_range(20.0, vision_range.max(vision_range_min))
            .filter(|t| self.is_viable_pass_target(ctx, t))
            .collect();

        if pass_options.is_empty() {
            return None;
        }

        // Evaluate each option - forwards prioritize different passes than other positions
        pass_options
            .into_iter()
            .map(|teammate| {
                let score = self.evaluate_forward_pass(ctx, &teammate);
                (teammate, score)
            })
            .max_by(|(_, score_a), (_, score_b)| {
                score_a
                    .partial_cmp(score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(teammate, _)| teammate)
    }

    /// Forward-specific pass evaluation - prioritizing attacks and goal scoring opportunities
    fn evaluate_forward_pass(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        // Start with the basic pass evaluator score
        let base_score = PassEvaluator::evaluate_pass(ctx, ctx.player, teammate);

        // Forward-specific factors - much more goal-oriented than midfielders
        let mut score = base_score.expected_value;

        // Goal distance factors - forwards prioritize passes that get closer to goal
        let forward_to_goal_dist = ctx.ball().distance_to_opponent_goal();
        let teammate_to_goal_dist =
            (teammate.position - ctx.player().opponent_goal_position()).magnitude();

        // Significantly boost passes that advance toward goal - key forward priority
        if teammate_to_goal_dist < forward_to_goal_dist {
            score += 40.0 * (1.0 - (teammate_to_goal_dist / forward_to_goal_dist));
        }

        // MAJOR boost for passes to other forwards (likely in better scoring positions)
        if teammate.tactical_positions.is_forward() {
            score += 40.0; // Increased from 20.0

            // Extra bonus for forward-to-forward in dangerous zone
            if teammate_to_goal_dist < 300.0 {
                score += 25.0;
            }

            // Bonus for forward who is making a run (has high velocity toward goal)
            let teammate_velocity = teammate.velocity(ctx);
            let to_goal = (ctx.player().opponent_goal_position() - teammate.position).normalize();
            if teammate_velocity.dot(&to_goal) > 3.0 {
                score += 20.0; // Forward is actively running toward goal
            }
        }

        // Boost for passes that break defensive lines
        if self.pass_breaks_defensive_line(ctx, teammate) {
            score += 30.0; // Increased from 25.0
        }

        // Heavy bonus for teammates who have a clear shot on goal - key forward priority
        if self.teammate_has_clear_shot(ctx, teammate) {
            score += 50.0; // Increased from 35.0
        }

        // Strong penalty for backwards passes unless under heavy pressure
        if teammate.position.x < ctx.player.position.x && !self.is_under_heavy_pressure(ctx) {
            score -= 30.0; // Increased from 15.0

            // Extra penalty if passing back when in attacking third
            if forward_to_goal_dist < 350.0 {
                score -= 20.0;
            }
        }

        score
    }

    /// Check if a pass to this teammate would break through a defensive line
    fn pass_breaks_defensive_line(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let player_pos = ctx.player.position;
        let teammate_pos = teammate.position;

        // Create a line between player and teammate
        let pass_direction = (teammate_pos - player_pos).normalize();
        let pass_distance = (teammate_pos - player_pos).magnitude();

        // Look for opponents between the player and teammate
        let opponents_in_line = ctx
            .players()
            .opponents()
            .all()
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
    fn is_viable_pass_target(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        // Basic viability criteria
        let has_clear_lane = ctx.player().has_clear_pass(teammate.id);
        let not_heavily_marked = !self.is_heavily_marked(ctx, teammate);

        // Forwards are more aggressive with passing - they care less about position
        // and more about goal scoring opportunities
        let creates_opportunity = self.pass_creates_opportunity(ctx, teammate);

        has_clear_lane && not_heavily_marked && creates_opportunity
    }

    /// Check if a pass would create a good attacking opportunity
    fn pass_creates_opportunity(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let distance_to_goal =
            (teammate.position - ctx.player().opponent_goal_position()).magnitude();

        // Always allow passes to teammates close to goal
        if distance_to_goal < 250.0 {
            return true;
        }

        // Allow passes to other forwards who have space
        if teammate.tactical_positions.is_forward() {
            let space_around_teammate = self.calculate_space_around_player(ctx, teammate);
            if space_around_teammate > 5.0 {
                return true;
            }
            // Always allow passes between forwards in attacking half
            if distance_to_goal < 500.0 {
                return true;
            }
        }

        // Passing backwards is generally not a good option for forwards
        // unless under heavy pressure
        if teammate.position.x < ctx.player.position.x {
            // Only allow backwards passes if under heavy pressure or teammate has lots of space
            if self.is_under_heavy_pressure(ctx) {
                return true;
            }
            let space_around_teammate = self.calculate_space_around_player(ctx, teammate);
            if space_around_teammate > 8.0 {
                return true; // Safe backpass to unmarked player
            }
            return false;
        }

        // Check if the teammate has space to advance
        let space_around_teammate = self.calculate_space_around_player(ctx, teammate);
        if space_around_teammate > 6.0 {
            return true;
        }

        // Check if pass advances play significantly
        let current_distance = ctx.ball().distance_to_opponent_goal();
        if distance_to_goal < current_distance - 50.0 {
            return true; // Pass advances play by at least 5m
        }

        // Don't pass to heavily marked midfielders far from goal
        false
    }

    /// Check if a player is heavily marked by opponents
    fn is_heavily_marked(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        const MARKING_DISTANCE: f32 = 10.0;
        const MAX_MARKERS: usize = 2;

        let markers = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                (opponent.position - teammate.position).magnitude() <= MARKING_DISTANCE
            })
            .count();

        markers >= MAX_MARKERS
    }

    /// Determine if teammate has a clear shot on goal
    fn teammate_has_clear_shot(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let teammate_pos = teammate.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let shot_direction = (goal_pos - teammate_pos).normalize();
        let shot_distance = (goal_pos - teammate_pos).magnitude();

        let ray_cast_result =
            ctx.tick_context
                .space
                .cast_ray(teammate_pos, shot_direction, shot_distance, false);

        ray_cast_result.is_none() && shot_distance < 300.0
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

    /// Check if player should shoot instead of pass - AGGRESSIVE for forwards
    fn should_shoot_instead_of_pass(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let finishing = ctx.player.skills.technical.finishing;
        let technique = ctx.player.skills.technical.technique;
        let long_shots = ctx.player.skills.technical.long_shots;
        let composure = ctx.player.skills.mental.composure;

        let has_clear_shot = ctx.player().has_clear_shot();
        let close_blockers = ctx.players().opponents().nearby(5.0).count();

        // Very close to goal - always shoot
        if distance_to_goal < 60.0 {
            return true;
        }

        // Inside penalty area - shoot with decent skill or clear shot
        if distance_to_goal < 165.0 {
            if has_clear_shot {
                return true;
            }
            // Shoot with good skill even with blockers
            if finishing > 10.0 && close_blockers <= 1 {
                return true;
            }
            if composure > 13.0 {
                return true;
            }
        }

        // Edge of box - shoot with good skills
        if distance_to_goal < 250.0 {
            if has_clear_shot && finishing > 9.0 {
                return true;
            }
            // Good long shot taker
            if long_shots > 13.0 && close_blockers == 0 {
                return true;
            }
        }

        // Further out - only with clear shot and excellent skills
        if distance_to_goal < 350.0 && has_clear_shot {
            let shooting_skill = (finishing + technique) / 2.0;
            if shooting_skill > 14.0 || long_shots > 15.0 {
                return true;
            }
        }

        false
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
        // Only adjust position within a specific time window to prevent endless twitching
        let in_adjustment_window = ctx.in_state_time >= MIN_POSITION_ADJUSTMENT_TIME
            && ctx.in_state_time <= MAX_POSITION_ADJUSTMENT_TIME;

        // If no good passing option and not under immediate pressure and within time window
        in_adjustment_window
            && self.find_best_pass_option(ctx).is_none()
            && !self.is_under_heavy_pressure(ctx)
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
            let adjustment = perpendicular * 5.0; // Reduced from 8.0 to prevent excessive twitching

            return player_pos + adjustment;
        }

        // Default to moving toward goal if no better option
        let to_goal = goal_pos - player_pos;
        let goal_direction = to_goal.normalize();
        player_pos + goal_direction * 5.0 // Reduced from 10.0 to prevent excessive movement
    }

    /// Look for space between defenders toward the goal
    fn find_space_between_opponents_toward_goal(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal_direction = (goal_pos - player_pos).normalize();

        // Get opponents between player and goal
        let opponents_between = ctx
            .players()
            .opponents()
            .all()
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
            for j in i + 1..opponents_between.len() {
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
