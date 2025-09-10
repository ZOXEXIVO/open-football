use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const TACKLE_RANGE: f32 = 30.0; // Distance to attempt tackling
const PRESS_RANGE: f32 = 100.0; // Distance to press opponent
const FREE_SPACE_RADIUS: f32 = 15.0; // Radius to check for free space
const ATTACK_SUPPORT_TIME_LIMIT: u64 = 300; // Max time to stay in support without action

#[derive(Default)]
pub struct MidfielderAttackSupportingState {}

impl StateProcessingHandler for MidfielderAttackSupportingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If player has the ball, transition to running with ball
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // If team loses possession, switch to defensive duties
        if ctx.team().is_control_ball() {
            // Ball coming towards player - try to intercept
            if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // If ball is too far away, return to position
            if ctx.ball().distance() < 300.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::CreatingSpace,
                ));
            }

            // If we've been supporting for too long without contributing, reassess
            if ctx.in_state_time > ATTACK_SUPPORT_TIME_LIMIT {
                // Check if we're too far from our position
                if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Returning,
                    ));
                }

                // Otherwise continue running to find better support position
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }
        } else {
            // Check if we should tackle or press
            if ctx.ball().distance() < TACKLE_RANGE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }

            if ctx.ball().distance() < PRESS_RANGE && ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // Otherwise return to defensive position
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate the best attacking support position
        let target_position = self.calculate_optimal_support_position(ctx);

        Some(
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 30.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderAttackSupportingState {
    /// Calculate the optimal position to support the attack
    fn calculate_optimal_support_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine attacking direction based on team side
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,  // Attack towards positive X
            Some(PlayerSide::Right) => -1.0, // Attack towards negative X
            None => 0.0,
        };

        // Find the ball holder if there is one
        if let Some(ball_holder) = self.find_ball_holder(ctx) {
            return self.calculate_support_for_ball_holder(ctx, &ball_holder, attacking_direction);
        }

        // If no clear ball holder, move to support general attacking play
        self.calculate_general_support_position(ctx, attacking_direction, field_width, field_height)
    }

    /// Find teammate who currently has the ball
    fn find_ball_holder(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id == ctx.player.team_id {
                    return Some(MatchPlayerLite {
                        id: owner_id,
                        position: ctx.tick_context.positions.players.position(owner_id),
                        tactical_positions: owner.tactical_position.current_position,
                    });
                }
            }
        }
        None
    }

    /// Calculate support position relative to ball holder
    fn calculate_support_for_ball_holder(
        &self,
        ctx: &StateProcessingContext,
        ball_holder: &MatchPlayerLite,
        attacking_direction: f32
    ) -> Vector3<f32> {
        let ball_holder_pos = ball_holder.position;
        let player_pos = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine what type of support run to make
        let support_type = self.determine_support_type(ctx, ball_holder);

        let target_position = match support_type {
            SupportType::ForwardOverlap => {
                // Run ahead of ball holder to receive through ball
                Vector3::new(
                    ball_holder_pos.x + (60.0 * attacking_direction),
                    ball_holder_pos.y + self.calculate_width_variation(ctx, player_pos, field_height),
                    0.0
                )
            },
            SupportType::WideSupport => {
                // Provide width by moving to touchline
                let wide_y = if player_pos.y < field_height / 2.0 {
                    field_height * 0.15  // Move to left touchline
                } else {
                    field_height * 0.85  // Move to right touchline
                };

                Vector3::new(
                    ball_holder_pos.x + (40.0 * attacking_direction),
                    wide_y,
                    0.0
                )
            },
            SupportType::CentralSupport => {
                // Support in central areas for combination play
                Vector3::new(
                    ball_holder_pos.x + (30.0 * attacking_direction),
                    field_height * 0.5,
                    0.0
                )
            },
            SupportType::DeepSupport => {
                // Stay deeper to recycle possession or switch play
                Vector3::new(
                    ball_holder_pos.x - (20.0 * attacking_direction),
                    ball_holder_pos.y + self.calculate_width_variation(ctx, player_pos, field_height),
                    0.0
                )
            }
        };

        // Find free space near target position
        self.find_nearest_free_space(ctx, target_position, field_width, field_height)
    }

    /// Calculate general support position when no specific ball holder
    fn calculate_general_support_position(
        &self,
        ctx: &StateProcessingContext,
        attacking_direction: f32,
        field_width: f32,
        field_height: f32
    ) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let player_pos = ctx.player.position;

        // Move towards attacking areas while maintaining good positioning
        let target_x = ball_pos.x + (50.0 * attacking_direction);

        // Vary position based on where other attacking players are
        let target_y = self.calculate_intelligent_y_position(ctx, field_height);

        let initial_target = Vector3::new(target_x, target_y, 0.0);

        // Find free space near this position
        self.find_nearest_free_space(ctx, initial_target, field_width, field_height)
    }

    /// Determine what type of support run to make
    fn determine_support_type(&self, ctx: &StateProcessingContext, ball_holder: &MatchPlayerLite) -> SupportType {
        let goal_distance = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;

        // Get player's tactical intelligence
        let vision = ctx.player.skills.mental.vision;
        let positioning = ctx.player.skills.mental.positioning;
        let off_the_ball = ctx.player.skills.mental.off_the_ball;

        let tactical_intelligence = (vision + positioning + off_the_ball) / 3.0;

        // Determine support based on game situation and player intelligence
        if goal_distance < field_width * 0.3 {
            // Close to goal - make attacking runs
            if tactical_intelligence > 15.0 && self.has_space_for_through_ball(ctx, ball_holder) {
                SupportType::ForwardOverlap
            } else if self.should_provide_width(ctx) {
                SupportType::WideSupport
            } else {
                SupportType::CentralSupport
            }
        } else if goal_distance < field_width * 0.6 {
            // Middle third - balance support and positioning
            if ball_holder.tactical_positions.is_forward() {
                SupportType::CentralSupport
            } else if self.should_provide_width(ctx) {
                SupportType::WideSupport
            } else {
                SupportType::ForwardOverlap
            }
        } else {
            // Defensive third - focus on possession and build-up
            if tactical_intelligence > 12.0 {
                SupportType::CentralSupport
            } else {
                SupportType::DeepSupport
            }
        }
    }

    /// Calculate width variation based on other players' positions
    fn calculate_width_variation(&self, ctx: &StateProcessingContext, player_pos: Vector3<f32>, field_height: f32) -> f32 {
        // Check where other attacking players are positioned
        let attacking_teammates = ctx.players().teammates().all()
            .filter(|teammate| {
                let is_ahead = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > player_pos.x,
                    Some(PlayerSide::Right) => teammate.position.x < player_pos.x,
                    None => false,
                };
                is_ahead
            })
            .collect::<Vec<_>>();

        // If teammates are clustered in center, go wide
        let center_players = attacking_teammates.iter()
            .filter(|t| (t.position.y - field_height * 0.5).abs() < field_height * 0.2)
            .count();

        if center_players >= 2 {
            // Go wide
            if player_pos.y < field_height * 0.5 {
                -30.0  // Move towards left touchline
            } else {
                30.0   // Move towards right touchline
            }
        } else {
            // Slight variation for natural movement
            (rand::random::<f32>() - 0.5) * 20.0
        }
    }

    /// Calculate intelligent Y position based on team shape
    fn calculate_intelligent_y_position(&self, ctx: &StateProcessingContext, field_height: f32) -> f32 {
        let player_pos = ctx.player.position;

        // Get positions of other attacking players
        let attacking_teammates = ctx.players().teammates().all()
            .filter(|teammate| teammate.tactical_positions.is_forward() || teammate.tactical_positions.is_midfielder())
            .collect::<Vec<_>>();

        if attacking_teammates.len() < 2 {
            // Few attackers - stay central
            return field_height * 0.5;
        }

        // Calculate average Y position of attacking teammates
        let avg_y = attacking_teammates.iter()
            .map(|t| t.position.y)
            .sum::<f32>() / attacking_teammates.len() as f32;

        // Position ourselves to create balance
        let center_y = field_height * 0.5;

        if (avg_y - center_y).abs() < field_height * 0.1 {
            // Team is central - create width
            if player_pos.y < center_y {
                field_height * 0.25  // Go to left
            } else {
                field_height * 0.75  // Go to right
            }
        } else {
            // Team has width - balance it
            if avg_y > center_y {
                field_height * 0.35  // Balance towards left
            } else {
                field_height * 0.65  // Balance towards right
            }
        }
    }

    /// Check if there's space for a through ball run
    fn has_space_for_through_ball(&self, ctx: &StateProcessingContext, ball_holder: &MatchPlayerLite) -> bool {
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0),
            Some(PlayerSide::Right) => Vector3::new(-1.0, 0.0, 0.0),
            None => Vector3::new(0.0, 0.0, 0.0),
        };

        let potential_run_end = ball_holder.position + attacking_direction * 80.0;

        // Check if there are opponents blocking this space
        let blocking_opponents = ctx.players().opponents().all()
            .filter(|opp| {
                let distance_from_run = self.distance_to_line_segment(
                    opp.position,
                    ball_holder.position,
                    potential_run_end
                );
                distance_from_run < 15.0
            })
            .count();

        blocking_opponents < 2
    }

    /// Check if team needs width in attack
    fn should_provide_width(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;

        // Count teammates in wide positions
        let wide_teammates = ctx.players().teammates().all()
            .filter(|teammate| {
                let distance_from_center = (teammate.position.y - field_height * 0.5).abs();
                distance_from_center > field_height * 0.3
            })
            .count();

        // If fewer than 2 players are wide, we should provide width
        wide_teammates < 2
    }

    /// Find nearest free space to a target position
    fn find_nearest_free_space(
        &self,
        ctx: &StateProcessingContext,
        target: Vector3<f32>,
        field_width: f32,
        field_height: f32
    ) -> Vector3<f32> {
        // Check if target position has free space
        if self.is_position_free(ctx, target) {
            return self.constrain_to_field(target, field_width, field_height);
        }

        let search_angles: [f32; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];
        let search_distances: [f32; 3] = [10.0, 20.0, 30.0];

        for &distance in &search_distances {
            for &angle in &search_angles {
                let angle_rad = angle.to_radians();
                let offset = Vector3::new(
                    angle_rad.cos() * distance,
                    angle_rad.sin() * distance,
                    0.0
                );

                let test_position = target + offset;

                if self.is_position_free(ctx, test_position) {
                    return self.constrain_to_field(test_position, field_width, field_height);
                }
            }
        }

        // If no free space found, return constrained target
        self.constrain_to_field(target, field_width, field_height)
    }

    /// Check if a position has free space (no opponents nearby)
    fn is_position_free(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let nearby_opponents = ctx.players().opponents().all()
            .filter(|opp| (opp.position - position).magnitude() < FREE_SPACE_RADIUS)
            .count();

        nearby_opponents == 0
    }

    /// Constrain position to field boundaries with margin
    fn constrain_to_field(&self, position: Vector3<f32>, field_width: f32, field_height: f32) -> Vector3<f32> {
        Vector3::new(
            position.x.clamp(field_width * 0.1, field_width * 0.9),
            position.y.clamp(field_height * 0.1, field_height * 0.9),
            0.0
        )
    }

    /// Calculate distance from point to line segment
    fn distance_to_line_segment(&self, point: Vector3<f32>, line_start: Vector3<f32>, line_end: Vector3<f32>) -> f32 {
        let line_vec = line_end - line_start;
        let point_vec = point - line_start;

        let line_len_sq = line_vec.magnitude_squared();
        if line_len_sq == 0.0 {
            return (point - line_start).magnitude();
        }

        let t = (point_vec.dot(&line_vec) / line_len_sq).clamp(0.0, 1.0);
        let projection = line_start + line_vec * t;

        (point - projection).magnitude()
    }
}

/// Types of support runs a midfielder can make
#[derive(Debug, Clone, Copy)]
enum SupportType {
    ForwardOverlap,  // Run ahead of ball holder for through balls
    WideSupport,     // Provide width on the flanks
    CentralSupport,  // Support in central areas for combination play
    DeepSupport,     // Stay deeper for possession recycling
}