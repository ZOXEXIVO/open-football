use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::processor::StateChangeResult;
use crate::r#match::player::strategies::processor::{StateProcessingContext, StateProcessingHandler};
use crate::r#match::{
    ConditionContext, PlayerSide, SteeringBehavior,
    VectorExtensions,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperWalkingState {}

impl StateProcessingHandler for GoalkeeperWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If goalkeeper has the ball, immediately transition to passing
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // Check ball proximity and threat level
        let ball_distance = ctx.ball().distance();
        let ball_on_own_side = ctx.ball().on_own_side();

        // Improved threat assessment using goalkeeper skills
        let threat_level = self.assess_threat_level(ctx);

        // Transition to Attentive if ball is on own side and moderately close
        if ball_on_own_side {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Attentive,
            ));
        }

        // Check if ball is coming directly at goalkeeper
        if ctx.ball().is_towards_player_with_angle(0.85) && ball_distance < 200.0 {
            // Use anticipation skill to determine response timing
            let anticipation_factor = ctx.player.skills.mental.anticipation / 20.0;
            let reaction_distance = 250.0 + (anticipation_factor * 100.0); // Better anticipation = earlier reaction

            if ball_distance < reaction_distance {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // Use decision-making skill for coming out
        if self.should_come_out_advanced(ctx) && ball_distance < 250.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Check positioning using goalkeeper-specific skills
        if self.is_significantly_out_of_position(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check for immediate threats using concentration skill
        if threat_level > 0.7 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::UnderPressure,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate optimal position using goalkeeper skills
        let optimal_position = self.calculate_intelligent_position(ctx);

        if ctx.player.position.distance_to(&optimal_position) < 10.0 {
            // Small adjustments - use wander for natural movement
            Some(
                SteeringBehavior::Wander {
                    target: optimal_position,
                    radius: 50.0,
                    jitter: 1.0,
                    distance: 50.0,
                    angle: IntegerUtils::random(0, 360) as f32,
                }
                    .calculate(ctx.player)
                    .velocity * 0.5, // Slow movement for fine positioning
            )
        } else {
            // Need to reposition - use arrive for smooth movement
            Some(
                SteeringBehavior::Arrive {
                    target: optimal_position,
                    slowing_distance: 15.0,
                }
                    .calculate(ctx.player)
                    .velocity,
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking state has low intensity but more activity than standing
        GoalkeeperCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}

impl GoalkeeperWalkingState {
    /// Assess threat level using goalkeeper mental skills
    fn assess_threat_level(&self, ctx: &StateProcessingContext) -> f32 {
        let mut threat = 0.0;

        // Use concentration to assess multiple threats
        let concentration_factor = ctx.player.skills.mental.concentration / 20.0;

        // Check for opponents with ball
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent_with_ball.position.distance_to(&ctx.player.position);

            // Better concentration means better threat assessment
            if distance_to_opponent < 50.0 {
                threat += 0.8 * concentration_factor;
            } else if distance_to_opponent < 100.0 {
                threat += 0.5 * concentration_factor;
            }
        }

        // Check ball velocity and trajectory
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Use anticipation to predict threats
        let anticipation_factor = ctx.player.skills.mental.anticipation / 20.0;
        if ball_speed > 10.0 && ctx.ball().is_towards_player_with_angle(0.6) {
            threat += 0.4 * anticipation_factor;
        }

        threat.min(1.0)
    }

    /// Advanced decision for coming out using goalkeeper skills
    fn should_come_out_advanced(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let goalkeeper_skills = &ctx.player.skills;

        // Key skills for coming out decisions
        let decision_skill = goalkeeper_skills.mental.decisions / 20.0;
        let rushing_out = goalkeeper_skills.technical.long_throws / 20.0; // Goalkeeper-specific skill
        let command_of_area = goalkeeper_skills.mental.vision / 20.0; // Goalkeeper-specific
        let anticipation = goalkeeper_skills.mental.anticipation / 20.0;

        // Combined skill factor for coming out
        let coming_out_ability = (decision_skill + rushing_out + command_of_area + anticipation) / 4.0;

        // Base threshold adjusted by skills
        let base_threshold = 100.0;
        let skill_adjusted_threshold = base_threshold * (0.6 + coming_out_ability * 0.8); // Range: 60-140

        // Check if ball is loose and in dangerous area
        let ball_loose = !ctx.ball().is_owned();
        let ball_in_danger_zone = ball_distance < skill_adjusted_threshold;

        // Check if goalkeeper can reach ball first
        if ball_loose && ball_in_danger_zone {
            // Use acceleration and agility for reach calculation
            let reach_ability = (goalkeeper_skills.physical.acceleration +
                goalkeeper_skills.physical.agility) / 40.0;

            // Check if any opponent is closer
            for opponent in ctx.players().opponents().nearby(150.0) {
                let opp_distance_to_ball = (opponent.position - ctx.tick_context.positions.ball.position).magnitude();
                let keeper_distance_to_ball = ball_distance;

                // Factor in goalkeeper's reach ability
                if opp_distance_to_ball < keeper_distance_to_ball * (1.0 - reach_ability * 0.3) {
                    return false; // Opponent will reach first
                }
            }

            return true;
        }

        false
    }

    /// Check if significantly out of position using positioning skill
    fn is_significantly_out_of_position(&self, ctx: &StateProcessingContext) -> bool {
        let optimal_position = self.calculate_intelligent_position(ctx);
        let current_distance = ctx.player.position.distance_to(&optimal_position);

        // Use positioning skill to determine tolerance
        let positioning_skill = ctx.player.skills.mental.positioning / 20.0;
        let tolerance = 120.0 - (positioning_skill * 40.0); // Better positioning = tighter tolerance (80-120)

        current_distance > tolerance
    }

    /// Calculate intelligent position using multiple goalkeeper skills
    fn calculate_intelligent_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_position = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance_to_goal = (ball_position - goal_position).magnitude();

        // Use positioning and command of area skills
        let positioning_skill = ctx.player.skills.mental.positioning / 20.0;
        let command_of_area = ctx.player.skills.mental.vision / 20.0;
        let communication = ctx.player.skills.mental.leadership / 20.0;

        // Calculate angle to ball for positioning
        let angle_to_ball = if ball_distance_to_goal > 0.1 {
            (ball_position - goal_position).normalize()
        } else {
            ctx.player.start_position
        };

        // Base distance from goal line
        let mut optimal_distance = 3.0; // Start closer to goal

        // Adjust based on ball position and goalkeeper skills
        if ctx.ball().on_own_side() {
            // Ball on own half - position based on threat
            let threat_factor = ball_distance_to_goal / (ctx.context.field_size.width as f32 * 0.5);

            // Better command of area = more aggressive positioning
            optimal_distance += (15.0 - threat_factor * 10.0) * command_of_area;

            // Better positioning = more accurate placement
            optimal_distance *= 0.8 + positioning_skill * 0.4;

            // If ball is wide, adjust position laterally
            let ball_y_offset = ball_position.y - goal_position.y;
            let lateral_adjustment = ball_y_offset * 0.1 * positioning_skill; // Better positioning = better angle coverage

            // Calculate the position
            let mut new_position = goal_position + angle_to_ball * optimal_distance;
            new_position.y += lateral_adjustment;

            // Ensure within penalty area
            self.limit_to_penalty_area(new_position, ctx)
        } else {
            // Ball on opponent's half - position for distribution or counter
            let sweeper_keeper_ability = (command_of_area + communication) / 2.0;

            // Modern sweeper-keeper positioning
            optimal_distance = 8.0 + (sweeper_keeper_ability * 12.0); // 8-20 units from goal

            // Position more centrally when ball is far
            let mut new_position = goal_position;
            new_position.x += optimal_distance * (if ctx.player.side == Some(PlayerSide::Left) { 1.0 } else { -1.0 });

            self.limit_to_penalty_area(new_position, ctx)
        }
    }

    /// Limit position to penalty area with some flexibility based on skills
    fn limit_to_penalty_area(
        &self,
        position: Vector3<f32>,
        ctx: &StateProcessingContext,
    ) -> Vector3<f32> {
        let penalty_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left));

        // Allow slight extension for sweeper-keepers with high command of area
        let command_of_area = ctx.player.skills.mental.vision / 20.0;
        let extension_factor = 1.0 + (command_of_area * 0.1); // Up to 10% extension for excellent keepers

        let extended_min_x = penalty_area.min.x - (2.0 * extension_factor);
        let extended_max_x = penalty_area.max.x + (2.0 * extension_factor);

        Vector3::new(
            position.x.clamp(extended_min_x, extended_max_x),
            position.y.clamp(penalty_area.min.y, penalty_area.max.y),
            0.0,
        )
    }
}