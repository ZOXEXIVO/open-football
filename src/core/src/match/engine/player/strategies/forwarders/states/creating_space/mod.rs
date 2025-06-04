use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const CREATING_SPACE_THRESHOLD: f32 = 150.0;
const OPPONENT_DISTANCE_THRESHOLD: f32 = 20.0;
const MAX_DISTANCE_FROM_START: f32 = 180.0; // Maximum distance from starting position
const RETURN_TO_POSITION_THRESHOLD: f32 = 250.0; // Distance to trigger return to position
const MAX_TIME_IN_STATE: u64 = 200; // Maximum time to stay in this state
const INTELLIGENT_RUN_DISTANCE: f32 = 120.0; // Distance for intelligent runs

#[derive(Default)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball - immediate transition
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Check if player has strayed too far from position
        if ctx.player().distance_from_start_position() > RETURN_TO_POSITION_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        // Check if team lost possession - switch to defensive positioning
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // If the ball is close and moving toward player, try to intercept
        if ctx.ball().distance() < 150.0 && ctx.ball().is_towards_player_with_angle(0.9) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Add a time limit for staying in this state to prevent getting stuck
        if ctx.in_state_time > MAX_TIME_IN_STATE {
            // Transition based on tactical situation
            if self.should_make_attacking_run(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::RunningInBehind));
            } else {
                return Some(StateChangeResult::with_forward_state(ForwardState::Running));
            }
        }

        // Check if the player has successfully created space and should receive the ball
        if self.has_created_space_and_is_available(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        }

        // Check if the player should make an intelligent run
        if self.should_make_intelligent_run(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::RunningInBehind,
            ));
        }

        // Check if the player is too close to an opponent and should change position
        if self.should_adjust_position_due_to_pressure(ctx) {
            // Continue in creating space state but adjust position
            return None;
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target_position = self.calculate_intelligent_space_creating_position(ctx);

        Some(
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 20.0,
            }
                .calculate(ctx.player)
                .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardCreatingSpaceState {
    /// Check if the player has created sufficient space and is in a good position to receive
    fn has_created_space_and_is_available(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there are no opponents within the space threshold
        let space_created = !ctx.players().opponents().exists(CREATING_SPACE_THRESHOLD);

        // Check if player is in a good attacking position
        let in_good_position = self.is_in_good_attacking_position(ctx);

        // Check if there's a clear passing lane from ball holder
        let has_clear_lane = self.has_clear_passing_lane_from_ball_holder(ctx);

        // Minimum time to avoid rapid state changes
        let minimum_time_in_state = 40;

        space_created && in_good_position && has_clear_lane && ctx.in_state_time > minimum_time_in_state
    }

    /// Determine if the player should make an intelligent attacking run
    fn should_make_intelligent_run(&self, ctx: &StateProcessingContext) -> bool {
        // Only make runs when team has possession and ball is in good position
        if !ctx.team().is_control_ball() {
            return false;
        }

        // Check if ball holder is in a position to make a through pass
        let ball_holder_can_pass = self.ball_holder_can_make_through_pass(ctx);

        // Check if there's space to run into
        let space_ahead = self.has_space_ahead_for_run(ctx);

        // Check if player isn't offside
        let not_offside = ctx.player().on_own_side() || self.run_would_stay_onside(ctx);

        // Make runs more likely when in attacking third
        let in_attacking_phase = self.is_in_attacking_phase(ctx);

        ball_holder_can_pass && space_ahead && not_offside && in_attacking_phase
    }

    /// Check if player should make an attacking run based on game situation
    fn should_make_attacking_run(&self, ctx: &StateProcessingContext) -> bool {
        // More aggressive when team is losing or in final third
        let team_needs_goals = ctx.team().is_loosing() || ctx.context.time.is_running_out();
        let ball_in_final_third = ctx.ball().distance_to_opponent_goal() < ctx.context.field_size.width as f32 * 0.33;

        team_needs_goals || ball_in_final_third
    }

    /// Check if player should adjust position due to opponent pressure
    fn should_adjust_position_due_to_pressure(&self, ctx: &StateProcessingContext) -> bool {
        // Check for nearby opponents that are limiting space
        let close_opponents = ctx.players().opponents().all()
            .filter(|opp| (opp.position - ctx.player.position).magnitude() < 15.0)
            .count();

        close_opponents >= 2
    }

    /// Calculate an intelligent position for creating space that focuses on goal-scoring opportunities
    fn calculate_intelligent_space_creating_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let player_side = ctx.player.side.unwrap_or(PlayerSide::Left);

        // Get ball position and ball holder information
        let ball_holder = self.get_ball_holder(ctx);

        // Calculate the attacking direction based on player's side
        let attacking_direction = match player_side {
            PlayerSide::Left => 1.0,  // Moving towards positive X
            PlayerSide::Right => -1.0, // Moving towards negative X
        };

        // If a teammate has the ball, create space relative to ball holder's position
        if let Some(holder) = ball_holder {
            return self.calculate_position_relative_to_ball_holder(ctx, &holder, attacking_direction);
        }

        // No specific ball holder - calculate general attacking position
        self.calculate_general_attacking_position(ctx, attacking_direction, field_width, field_height)
    }

    /// Calculate position relative to the ball holder to create effective space
    fn calculate_position_relative_to_ball_holder(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite,
        attacking_direction: f32
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Calculate different types of runs based on ball holder's position
        let run_type = self.determine_run_type(ctx, holder);

        let target_position = match run_type {
            SpaceCreatingRunType::DiagonalRun => {
                // Diagonal run towards goal, creating space diagonally
                let diagonal_offset_x = 40.0 * attacking_direction;
                let diagonal_offset_y = if player_position.y < field_height / 2.0 { 30.0 } else { -30.0 };

                Vector3::new(
                    holder_position.x + diagonal_offset_x,
                    holder_position.y + diagonal_offset_y,
                    0.0
                )
            },
            SpaceCreatingRunType::WideRun => {
                // Wide run to stretch the defense
                let wide_y = if player_position.y < field_height / 2.0 {
                    field_height * 0.2 // Go wide to the touchline
                } else {
                    field_height * 0.8
                };

                Vector3::new(
                    holder_position.x + (30.0 * attacking_direction),
                    wide_y,
                    0.0
                )
            },
            SpaceCreatingRunType::DeepRun => {
                // Run deeper towards goal
                Vector3::new(
                    holder_position.x + (60.0 * attacking_direction),
                    holder_position.y + ((player_position.y - holder_position.y) * 0.3),
                    0.0
                )
            },
            SpaceCreatingRunType::SupportRun => {
                // Short support run to create passing option
                Vector3::new(
                    holder_position.x + (20.0 * attacking_direction),
                    holder_position.y + if player_position.y > holder_position.y { 15.0 } else { -15.0 },
                    0.0
                )
            }
        };

        // Ensure the position is within field boundaries and not too far from start
        self.constrain_position(ctx, target_position, field_width, field_height)
    }

    /// Calculate a general attacking position when no specific ball holder
    fn calculate_general_attacking_position(
        &self,
        ctx: &StateProcessingContext,
        attacking_direction: f32,
        field_width: f32,
        field_height: f32
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;

        // Create positions that focus on goal-scoring opportunities
        let goal_oriented_x = ball_position.x + (50.0 * attacking_direction);

        // Vary Y position based on current position to create width
        let target_y = if player_position.y < field_height * 0.4 {
            field_height * 0.3  // Stay wide left
        } else if player_position.y > field_height * 0.6 {
            field_height * 0.7  // Stay wide right
        } else {
            field_height * 0.5  // Central position
        };

        let target_position = Vector3::new(goal_oriented_x, target_y, 0.0);

        self.constrain_position(ctx, target_position, field_width, field_height)
    }

    /// Determine the type of run to make based on tactical situation
    fn determine_run_type(&self, ctx: &StateProcessingContext, holder: &crate::r#match::MatchPlayerLite) -> SpaceCreatingRunType {
        let holder_position = holder.position;
        let field_width = ctx.context.field_size.width as f32;

        // Determine run type based on ball holder's position and game situation
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        if distance_to_goal < field_width * 0.3 {
            // Close to goal - make runs into the box
            if self.has_space_in_penalty_area(ctx) {
                SpaceCreatingRunType::DeepRun
            } else {
                SpaceCreatingRunType::DiagonalRun
            }
        } else if distance_to_goal < field_width * 0.6 {
            // Middle third - create width or diagonal runs
            if self.should_create_width(ctx) {
                SpaceCreatingRunType::WideRun
            } else {
                SpaceCreatingRunType::DiagonalRun
            }
        } else {
            // Defensive third - support play buildup
            SpaceCreatingRunType::SupportRun
        }
    }

    /// Constrain position to field boundaries and distance limits
    fn constrain_position(
        &self,
        ctx: &StateProcessingContext,
        target_position: Vector3<f32>,
        field_width: f32,
        field_height: f32
    ) -> Vector3<f32> {
        // Ensure we're not moving too far from starting position
        let distance_from_start = (target_position - ctx.player.start_position).magnitude();
        let final_position = if distance_from_start > MAX_DISTANCE_FROM_START {
            let direction = (target_position - ctx.player.start_position).normalize();
            ctx.player.start_position + direction * MAX_DISTANCE_FROM_START
        } else {
            target_position
        };

        // Final boundary check with some margin from touchlines
        Vector3::new(
            final_position.x.clamp(field_width * 0.05, field_width * 0.95),
            final_position.y.clamp(field_height * 0.05, field_height * 0.95),
            0.0,
        )
    }

    // Helper methods for tactical decision making

    fn get_ball_holder(&self, ctx: &StateProcessingContext) -> Option<crate::r#match::MatchPlayerLite> {
        ctx.players()
            .teammates()
            .all()
            .find(|t| ctx.ball().owner_id() == Some(t.id))
    }

    fn is_in_good_attacking_position(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = (ctx.player.position - ctx.player().opponent_goal_position()).magnitude();
        let field_width = ctx.context.field_size.width as f32;

        // Good attacking position is within 60% of field from goal
        distance_to_goal < field_width * 0.6
    }

    fn has_clear_passing_lane_from_ball_holder(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(holder) = self.get_ball_holder(ctx) {
            let pass_direction = (ctx.player.position - holder.position).normalize();
            let pass_distance = (ctx.player.position - holder.position).magnitude();

            // Check for opponents in the passing lane
            let blocking_opponents = ctx.players().opponents().all()
                .filter(|opp| {
                    let to_opponent = opp.position - holder.position;
                    let projection = to_opponent.dot(&pass_direction);

                    if projection <= 0.0 || projection >= pass_distance {
                        return false;
                    }

                    let projected_point = holder.position + pass_direction * projection;
                    let perp_distance = (opp.position - projected_point).magnitude();

                    perp_distance < 3.0
                })
                .count();

            blocking_opponents == 0
        } else {
            true // No specific holder, assume clear
        }
    }

    fn ball_holder_can_make_through_pass(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(holder) = self.get_ball_holder(ctx) {
            // Check ball holder's vision and passing skills
            let player = ctx.player();
            let holder_skills = player.skills(holder.id);
            let vision_skill = holder_skills.mental.vision;
            let passing_skill = holder_skills.technical.passing;

            // Check if holder is under pressure
            let holder_under_pressure = ctx.players().opponents().all()
                .any(|opp| (opp.position - holder.position).magnitude() < 10.0);

            (vision_skill > 12.0 || passing_skill > 13.0) && !holder_under_pressure
        } else {
            false
        }
    }

    fn has_space_ahead_for_run(&self, ctx: &StateProcessingContext) -> bool {
        let player_position = ctx.player.position;
        let attacking_direction = match ctx.player.side.unwrap_or(PlayerSide::Left) {
            PlayerSide::Left => Vector3::new(1.0, 0.0, 0.0),
            PlayerSide::Right => Vector3::new(-1.0, 0.0, 0.0),
        };

        let check_position = player_position + attacking_direction * INTELLIGENT_RUN_DISTANCE;

        // Check if there are opponents in the space we want to run into
        let opponents_in_space = ctx.players().opponents().all()
            .filter(|opp| (opp.position - check_position).magnitude() < 20.0)
            .count();

        opponents_in_space < 2
    }

    fn run_would_stay_onside(&self, _ctx: &StateProcessingContext) -> bool {
        // Simplified offside check - in a full implementation, this would check
        // defender positions and timing
        true
    }

    fn is_in_attacking_phase(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;

        ball_distance_to_goal < field_width * 0.7
    }

    fn has_space_in_penalty_area(&self, ctx: &StateProcessingContext) -> bool {
        let goal_position = ctx.player().opponent_goal_position();
        let penalty_area_center = Vector3::new(
            goal_position.x - 16.5, // Standard penalty area depth
            goal_position.y,
            0.0
        );

        let opponents_in_box = ctx.players().opponents().all()
            .filter(|opp| (opp.position - penalty_area_center).magnitude() < 20.0)
            .count();

        opponents_in_box < 3
    }

    fn should_create_width(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let teammates_wide = ctx.players().teammates().all()
            .filter(|t| t.position.y < field_height * 0.3 || t.position.y > field_height * 0.7)
            .count();

        teammates_wide < 2 // Need more width in the attack
    }
}

/// Types of runs a forward can make when creating space
#[derive(Debug, Clone, Copy)]
enum SpaceCreatingRunType {
    DiagonalRun,  // Diagonal run towards goal
    WideRun,      // Wide run to stretch defense
    DeepRun,      // Deep run behind defense
    SupportRun,   // Short support run for ball retention
}