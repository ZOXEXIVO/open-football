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

        // Determine the best teammate to pass to using improved logic
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

    /// Check if a teammate is viable for receiving a pass
    fn is_viable_pass_target(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Basic viability criteria
        let has_clear_lane = ctx.player().has_clear_pass(teammate.id);
        let not_heavily_marked = !self.is_heavily_marked(ctx, teammate);
        let is_in_good_position = self.is_in_good_position(ctx, teammate);

        has_clear_lane && not_heavily_marked && is_in_good_position
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

    /// Check if teammate is in a good position tactically
    fn is_in_good_position(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Determine if this is a backward pass based on player's side
        let is_backward_pass = match ctx.player.side {
            Some(PlayerSide::Left) => teammate.position.x < ctx.player.position.x,
            Some(PlayerSide::Right) => teammate.position.x > ctx.player.position.x,
            None => false, // Default case, should not happen in a match
        };

        // For midfielders, we want to avoid passing backward unless necessary
        if is_backward_pass {
            // Only pass backward if under pressure
            return self.is_under_heavy_pressure(ctx);
        }

        // Otherwise, player is in a good position
        true
    }

    /// Determine if player should shoot instead of pass
    fn should_shoot_instead_of_pass(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let shooting_skill = ctx.player.skills.technical.long_shots / 20.0;

        // Consider shooting from good positions with skilled players
        distance_to_goal < 200.0 * shooting_skill && ctx.player().has_clear_shot()
    }

    /// Check if player is under heavy pressure from opponents
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        const PRESSURE_DISTANCE: f32 = 10.0;
        const PRESSURE_THRESHOLD: usize = 2;

        let pressing_opponents = ctx.players().opponents().nearby(PRESSURE_DISTANCE).count();
        pressing_opponents >= PRESSURE_THRESHOLD
    }

    /// Determine if player can effectively dribble out of pressure
    fn can_dribble_effectively(&self, ctx: &StateProcessingContext) -> bool {
        // Check player's dribbling skill
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;

        // Check if there's space to dribble into
        let has_space = !ctx.players().opponents().exists(15.0);

        dribbling_skill > 0.7 && has_space
    }

    /// Determine if player should adjust position to find better passing angles
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        // If no good passing option and not under immediate pressure
        self.find_best_pass_option(ctx).is_none() && !self.is_under_heavy_pressure(ctx)
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