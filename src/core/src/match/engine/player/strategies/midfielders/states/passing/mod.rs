use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // First, look for high-value breakthrough passes (for skilled players)
        if let Some(breakthrough_target) = self.find_breakthrough_pass_option(ctx) {
            // Execute the high-quality breakthrough pass
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(breakthrough_target.id)
                        .with_target(breakthrough_target.position)
                        .with_force(ctx.player().pass_teammate_power(breakthrough_target.id) * 1.1) // Slightly more power for breakthrough passes
                        .build(),
                )),
            ));
        }

        // If no breakthrough pass, determine the best regular pass option
        if let Some(target_teammate) = self.find_best_pass_option(ctx) {
            // Execute the pass
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target_teammate.id)
                        .with_target(target_teammate.position)
                        .with_force(ctx.player().pass_teammate_power(target_teammate.id))
                        .build(),
                )),
            ));
        }

        // If no good passing option is found and we're close to goal, consider shooting
        if ctx.ball().distance_to_opponent_goal() < 200.0
            && self.should_shoot_instead_of_pass(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        // If under excessive pressure, consider dribbling to create space
        if self.is_under_heavy_pressure(ctx) && self.can_dribble_effectively(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Dribbling,
            ));
        }

        // Add a timeout mechanism - if we've been in this state for too long, make a decision
        if ctx.in_state_time > 60 {  // 60 ticks is a reasonable timeout
            // If we're under pressure, clear the ball or make a risky pass
            if self.is_under_heavy_pressure(ctx) {
                // Just make the safest available pass even if not ideal
                if let Some(any_teammate) = ctx.players().teammates().nearby(150.0).next() {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::build()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(any_teammate.id)
                                .with_target(any_teammate.position)
                                .with_force(ctx.player().pass_teammate_power(any_teammate.id) * 1.2) // Slightly more power for urgency
                                .build(),
                        )),
                    ));
                } else {
                    // No teammate in range - transition to dribbling as a last resort
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Dribbling,
                    ));
                }
            } else {
                // Not under immediate pressure, can take a more measured decision
                // Try to advance with the ball
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }
        }

        // Default - continue in current state looking for options
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If the player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
                return Some(
                    SteeringBehavior::Arrive {
                        target: self.calculate_better_passing_position(ctx, &nearest_teammate),
                        slowing_distance: 30.0,
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

impl MidfielderPassingState {
    /// Find breakthrough pass opportunities for players with high vision
    fn find_breakthrough_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        // Only exceptional vision players can make breakthrough passes
        let vision_skill = ctx.player.skills.mental.vision;
        let passing_skill = ctx.player.skills.technical.passing;

        // This is a special ability only for players with very good vision and passing
        if vision_skill < 15.0 || passing_skill < 14.0 {
            return None;
        }

        // Extended vision range for skilled players
        let vision_range = vision_skill * 20.0;

        let teammates = ctx.players().teammates();

        // Find teammates making attacking runs
        let breakthrough_targets = teammates.all()
            .filter(|teammate| {
                // Calculate if teammate is making a forward run
                let velocity = ctx.tick_context.positions.players.velocity(teammate.id);
                let is_moving_forward = velocity.magnitude() > 1.0;

                // Only consider attackers and attacking midfielders
                let is_attacking_player = teammate.tactical_positions.is_forward() ||
                    teammate.tactical_positions.is_midfielder(); // attacking

                // Check if teammate is in good position to receive
                let distance = (teammate.position - ctx.player.position).magnitude();

                // Check if this pass would break defensive lines
                let would_break_lines = self.would_pass_break_defensive_lines(ctx, teammate);

                // Player can see further with better vision
                distance < vision_range && is_moving_forward && is_attacking_player && would_break_lines
            })
            .collect::<Vec<_>>();

        // Find the best option based on potential threat
        breakthrough_targets.into_iter()
            .max_by(|a, b| {
                let a_value = self.calculate_breakthrough_value(ctx, a);
                let b_value = self.calculate_breakthrough_value(ctx, b);
                a_value.partial_cmp(&b_value).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Determine if a pass would break through defensive lines
    fn would_pass_break_defensive_lines(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite
    ) -> bool {
        let player_pos = ctx.player.position;
        let teammate_pos = teammate.position;
        let pass_direction = (teammate_pos - player_pos).normalize();
        let pass_distance = (teammate_pos - player_pos).magnitude();

        // Find opponents positioned between passer and target
        let opponents_between = ctx.players().opponents().all()
            .filter(|opponent| {
                // Vector from player to opponent
                let to_opponent = opponent.position - player_pos;

                // Project opponent onto pass direction
                let projection_distance = to_opponent.dot(&pass_direction);

                // Only consider opponents between player and target
                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                // Calculate perpendicular distance from passing lane
                let projected_point = player_pos + pass_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).magnitude();

                // Consider opponent in path if within reasonable distance of pass lane
                perp_distance < 8.0
            })
            .collect::<Vec<_>>();

        // If we have multiple opponents in the path, this would be a line-breaking pass
        if opponents_between.len() >= 2 {
            // Adjust success chance based on player skill
            let vision_skill = ctx.player.skills.mental.vision / 20.0;
            let passing_skill = ctx.player.skills.technical.passing / 20.0;
            let skill_factor = (vision_skill + passing_skill) / 2.0;

            // More skilled players can fit passes through tighter spaces
            let max_opponents = 2.0 + (skill_factor * 2.0); // Skilled players can pass through more defenders

            return opponents_between.len() as f32 <= max_opponents;
        }

        false
    }

    /// Calculate the strategic value of a breakthrough pass
    fn calculate_breakthrough_value(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite
    ) -> f32 {
        // Calculate distance to goal
        let goal_distance = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
        let field_width = ctx.context.field_size.width as f32;

        // Normalize goal distance value (closer to goal = higher value)
        let goal_distance_value = 1.0 - (goal_distance / field_width).clamp(0.0, 1.0);

        // Calculate space around the target player
        let space_value = self.calculate_space_around_player(ctx, teammate) / 10.0;

        // Calculate value based on target player's finishing ability
        let player = ctx.player();

        let target_skills = player.skills(teammate.id);
        let finishing_skill = target_skills.technical.finishing / 20.0;

        // Calculate final value - prioritize dangerous positions
        (goal_distance_value * 0.5) + (space_value * 0.3) + (finishing_skill * 0.2)
    }

    /// Find the best pass option using the improved evaluation system
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        // Use player's vision skill to determine range
        let vision_range = ctx.player.skills.mental.vision * 20.0;

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
                (teammate, score)
            })
            .max_by(|(_, score_a), (_, score_b)| {
                score_a.partial_cmp(score_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(teammate, _)| teammate)
    }

    /// Enhanced logic to check if a teammate is viable for receiving a pass
    fn is_viable_pass_target(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Basic viability criteria
        let has_clear_lane = self.has_clear_passing_lane(ctx, teammate);
        let not_heavily_marked = !self.is_heavily_marked(ctx, teammate);
        let is_in_good_position = self.is_in_good_position(ctx, teammate);

        has_clear_lane && not_heavily_marked && is_in_good_position
    }

    /// Enhanced logic to check for clear passing lanes
    fn has_clear_passing_lane(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let player_position = ctx.player.position;
        let teammate_position = teammate.position;
        let passing_direction = (teammate_position - player_position).normalize();
        let pass_distance = (teammate_position - player_position).magnitude();

        // Base lane width on player skill
        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;

        // Players with better passing/vision can thread passes through tighter spaces
        let base_lane_width = 3.0;
        let skill_factor = 0.6 + (pass_skill * 0.2) + (vision_skill * 0.2);
        let lane_width = base_lane_width * skill_factor;

        // Check if any opponent is close enough to intercept
        let intercepting_opponents = ctx.players().opponents().all()
            .filter(|opponent| {
                // Vector from player to opponent
                let to_opponent = opponent.position - player_position;

                // Project opponent position onto pass direction
                let projection_distance = to_opponent.dot(&passing_direction);

                // Only consider opponents between passer and target
                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                // Calculate perpendicular distance from passing lane
                let projected_point = player_position + passing_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).magnitude();

                // Consider opponent interception skills
                let interception_skill = ctx.player().skills(opponent.id).technical.tackling / 20.0;
                let effective_width = lane_width * (1.0 - interception_skill * 0.3);

                perp_distance < effective_width
            })
            .count();

        intercepting_opponents == 0
    }

    /// Check if a player is heavily marked by opponents with enhanced logic
    fn is_heavily_marked(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        const MARKING_DISTANCE: f32 = 5.0;
        const MAX_MARKERS: usize = 2;

        // Get opponents who are marking the teammate
        let markers = ctx.players().opponents().all()
            .filter(|opponent| {
                let distance = (opponent.position - teammate.position).magnitude();
                distance <= MARKING_DISTANCE
            })
            .collect::<Vec<_>>();

        // Consider both number of markers and their skill levels
        if markers.len() >= MAX_MARKERS {
            return true;
        }

        // If even one very skilled defender is marking closely, consider heavily marked
        if markers.len() == 1 {
            let marker = &markers[0];
            let marking_skill = ctx.player().skills(marker.id).mental.positioning;
            if marking_skill > 16.0 && (marker.position - teammate.position).magnitude() < 2.5 {
                return true;
            }
        }

        false
    }

    /// Check if teammate is in a good position tactically with enhanced logic
    fn is_in_good_position(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Determine if this is a backward pass based on player's side
        let is_backward_pass = match ctx.player.side {
            Some(PlayerSide::Left) => teammate.position.x < ctx.player.position.x,
            Some(PlayerSide::Right) => teammate.position.x > ctx.player.position.x,
            None => false, // Default case, should not happen in a match
        };

        // Calculate if this pass advances toward goal
        let player_goal_distance =
            (ctx.player.position - ctx.player().opponent_goal_position()).magnitude();
        let teammate_goal_distance =
            (teammate.position - ctx.player().opponent_goal_position()).magnitude();
        let advances_toward_goal = teammate_goal_distance < player_goal_distance;

        // For creative midfielders, allow some backward passes for recycling possession
        if is_backward_pass {
            // Only allow backward passes if under pressure or player has good vision
            let under_pressure = self.is_under_heavy_pressure(ctx);
            let has_good_vision = ctx.player.skills.mental.vision > 15.0;

            return under_pressure || has_good_vision;
        }

        // Check if the teammate will immediately be under pressure
        let teammate_will_be_pressured = ctx.players().opponents().all()
            .any(|opponent| {
                let current_distance = (opponent.position - teammate.position).magnitude();
                let opponent_velocity = ctx.tick_context.positions.players.velocity(opponent.id);

                // Calculate future position
                let future_opponent_pos = opponent.position + opponent_velocity * 10.0;
                let future_distance = (future_opponent_pos - teammate.position).magnitude();

                // Opponent is closing in quickly
                current_distance < 15.0 && future_distance < 5.0
            });

        // If the pass advances toward goal and doesn't immediately put teammate under pressure
        advances_toward_goal && !teammate_will_be_pressured
    }

    /// Determine if player should shoot instead of pass with enhanced logic
    fn should_shoot_instead_of_pass(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let shooting_skill = ctx.player.skills.technical.long_shots / 20.0;
        let finishing_skill = ctx.player.skills.technical.finishing / 20.0;

        // Combined shooting ability affects shooting distance
        let shooting_ability = (shooting_skill * 0.7) + (finishing_skill * 0.3);
        let effective_shooting_range = 150.0 + (shooting_ability * 100.0);

        // Consider shooting from good positions with skilled players
        distance_to_goal < effective_shooting_range && ctx.player().has_clear_shot()
    }

    /// Check if player is under heavy pressure from opponents
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        const PRESSURE_DISTANCE: f32 = 10.0;
        const PRESSURE_THRESHOLD: usize = 2;

        let pressing_opponents = ctx.players().opponents().nearby(PRESSURE_DISTANCE).count();
        pressing_opponents >= PRESSURE_THRESHOLD
    }

    /// Calculate the amount of space around a player
    fn calculate_space_around_player(
        &self,
        ctx: &StateProcessingContext,
        player: &MatchPlayerLite,
    ) -> f32 {
        // Dynamic space radius based on player's mental vision
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let base_space_radius = 10.0;
        let space_radius = base_space_radius * (0.8 + vision_skill * 0.4);

        // Weighted opponent threat calculation
        let opponents_nearby = ctx.players().opponents().all()
            .filter(|opponent| {
                let distance = (opponent.position - player.position).magnitude();
                distance <= space_radius
            })
            .collect::<Vec<_>>();

        // Calculate threat level based on distance and opponent marking skills
        let threat_level = opponents_nearby.iter().map(|opponent| {
            let distance = (opponent.position - player.position).magnitude();
            let marking_skill = ctx.player().skills(opponent.id).mental.positioning / 20.0;

            // Closer opponents and those with better marking are bigger threats
            let distance_factor = 1.0 - (distance / space_radius);
            distance_factor * (0.5 + marking_skill * 0.5)
        }).sum::<f32>();

        // Higher number means more space (less threat)
        space_radius - threat_level * 5.0
    }

    /// Determine if player can effectively dribble out of pressure
    fn can_dribble_effectively(&self, ctx: &StateProcessingContext) -> bool {
        // Check player's dribbling skill
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;
        let agility_skill = ctx.player.skills.physical.agility / 20.0;

        // Calculate combined dribbling effectiveness
        let dribbling_effectiveness = (dribbling_skill * 0.7) + (agility_skill * 0.3);

        // Check if there's space to dribble into
        let has_space = !ctx.players().opponents().exists(15.0);

        dribbling_effectiveness > 0.7 && has_space
    }

    /// Determine if player should adjust position to find better passing angles
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        // If no good passing option and not under immediate pressure
        self.find_best_pass_option(ctx).is_none() &&
            self.find_breakthrough_pass_option(ctx).is_none() &&
            !self.is_under_heavy_pressure(ctx)
    }

    /// Calculate a better position for finding passing angles
    fn calculate_better_passing_position(
        &self,
        ctx: &StateProcessingContext,
        target: &MatchPlayerLite
    ) -> Vector3<f32> {
        // Get positions
        let player_pos = ctx.player.position;
        let target_pos = target.position;

        // Calculate a position that improves the passing angle
        let to_target = target_pos - player_pos;
        let direction = to_target.normalize();

        // Move slightly to the side to create a better angle
        let perpendicular = Vector3::new(-direction.y, direction.x, 0.0);
        let adjustment = perpendicular * 5.0;

        player_pos + adjustment
    }
}