use nalgebra::Vector3;

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler};

const MAX_DEFENSIVE_LINE_DEVIATION: f32 = 40.0;  // Reduced from 50.0 - tighter defensive line
const BALL_PROXIMITY_THRESHOLD: f32 = 120.0; // Increased from 100.0 - react earlier to ball
const MARKING_DISTANCE_THRESHOLD: f32 = 40.0; // Increased from 30.0 - pick up attackers earlier
const PRESSING_DISTANCE_THRESHOLD: f32 = 50.0; // Distance to start pressing ball carrier
const DANGEROUS_RUN_SCAN_DISTANCE: f32 = 80.0; // Distance to scan for dangerous runs
const DANGEROUS_RUN_SPEED: f32 = 2.5; // Minimum speed to consider a dangerous run
const DANGEROUS_RUN_ANGLE: f32 = 0.6; // Minimum alignment toward goal

#[derive(Default)]
pub struct DefenderHoldingLineState {}

impl StateProcessingHandler for DefenderHoldingLineState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Calculate the defensive line position (x-axis: goal-to-goal)
        let defensive_line_position = self.calculate_defensive_line_position(ctx);

        // 2. Calculate the distance from the defender to the defensive line
        let distance_from_line = (ctx.player.position.x - defensive_line_position).abs();

        // 3. If the defender is too far from the defensive line, switch to Running state
        if distance_from_line > MAX_DEFENSIVE_LINE_DEVIATION {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Priority: Press opponent with ball if close and we're the best positioned
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance = opponent_with_ball.distance(ctx);
            if distance < PRESSING_DISTANCE_THRESHOLD {
                // Check if we're the best defender to press
                if ctx.player().defensive().is_best_defender_for_opponent(&opponent_with_ball) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        // CRITICAL: Check for dangerous runs - break from line to track attackers
        // This prevents defenders from standing still while attackers run past
        if let Some(dangerous_runner) = self.scan_for_dangerous_runs(ctx) {
            let distance_to_runner = dangerous_runner.distance(ctx);
            // Track if we're the best positioned defender OR if runner is very close (< 25m)
            // The close distance check ensures someone tracks even if coordination disagrees
            if ctx.player().defensive().is_best_defender_for_opponent(&dangerous_runner)
                || distance_to_runner < 25.0
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        if ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting
            ));
        }

        if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD {
            let opponent_nearby = self.is_opponent_nearby(ctx);
            return Some(StateChangeResult::with_defender_state(if opponent_nearby {
                DefenderState::Marking
            } else {
                DefenderState::Intercepting
            }));
        }

        // 6. Check if we should set up an offside trap
        if self.should_set_offside_trap(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::OffsideTrap,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        // For now, return None to indicate no state change
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let current_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;

        // Calculate target position based on zonal coverage
        let target_position = self.calculate_zonal_position(ctx, ball_position);

        let to_target = target_position - current_position;
        let distance = to_target.magnitude();

        // Define thresholds for movement
        const MIN_DISTANCE_THRESHOLD: f32 = 2.0;
        const SLOWING_DISTANCE: f32 = 10.0;

        // Base movement speed - walking/jogging pace for positional adjustments
        let pace_influence = (ctx.player.skills.physical.pace / 20.0).clamp(0.6, 1.2);
        let base_speed = 1.5 * pace_influence;

        if distance > MIN_DISTANCE_THRESHOLD {
            let direction = to_target.normalize();

            // Speed factor - slow down as approaching target
            let speed_factor = if distance > SLOWING_DISTANCE {
                1.0
            } else {
                (distance / SLOWING_DISTANCE).clamp(0.25, 1.0)
            };

            Some(direction * base_speed * speed_factor)
        } else {
            // In position - stay still (no artificial jitter)
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Holding line involves minimal movement - allows for recovery
        DefenderCondition::with_velocity(ActivityIntensity::Recovery).process(ctx);
    }
}

impl DefenderHoldingLineState {
    /// Calculate zonal defensive position - creates natural staggered formation
    /// Each defender positions based on their assigned opponent or zone
    fn calculate_zonal_position(&self, ctx: &StateProcessingContext, ball_position: Vector3<f32>) -> Vector3<f32> {
        let tactical_position = ctx.player.start_position;
        let current_position = ctx.player.position;
        let own_goal = ctx.ball().direction_to_own_goal();
        let field_center_y = ctx.context.field_size.height as f32 / 2.0;

        // Determine if this defender is a wide defender (fullback) or central
        let distance_from_center = (tactical_position.y - field_center_y).abs();
        let is_wide_defender = distance_from_center > 40.0;

        // Find the nearest opponent in this defender's zone
        let zone_half_width = 50.0;
        let nearest_opponent_in_zone = ctx.players().opponents().nearby(100.0)
            .filter(|opp| {
                let lateral_dist = (opp.position.y - tactical_position.y).abs();
                lateral_dist < zone_half_width
            })
            .min_by(|a, b| {
                let dist_a = (a.position - current_position).magnitude();
                let dist_b = (b.position - current_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        // DEPTH (X) CALCULATION
        let target_x = if let Some(opponent) = nearest_opponent_in_zone {
            // Track the opponent - position between them and goal
            let opponent_x = opponent.position.x;
            let goal_x = own_goal.x;

            // Position goal-side of opponent (between opponent and goal)
            let goal_direction = (goal_x - opponent_x).signum();
            let marking_offset = 8.0 * goal_direction; // Stay 8 units goal-side

            opponent_x + marking_offset
        } else {
            // No opponent in zone - use base tactical position with ball influence
            let ball_influence = (ball_position.x - tactical_position.x) * 0.2;

            // Wide defenders push up more when ball is on their side
            let wide_push = if is_wide_defender {
                let ball_on_my_side = (ball_position.y - field_center_y).signum()
                    == (tactical_position.y - field_center_y).signum();
                if ball_on_my_side { 15.0 } else { -5.0 }
            } else {
                0.0
            };

            tactical_position.x + ball_influence + wide_push
        };

        // LATERAL (Y) CALCULATION
        let target_y = if let Some(opponent) = nearest_opponent_in_zone {
            // Track opponent laterally but don't go too far from zone
            let opponent_y = opponent.position.y;
            let max_drift = 25.0;
            let drift = (opponent_y - tactical_position.y).clamp(-max_drift, max_drift);
            tactical_position.y + drift
        } else {
            // Shift toward ball side
            let ball_offset = ball_position.y - field_center_y;
            let shift = ball_offset * 0.12;
            tactical_position.y + shift
        };

        Vector3::new(target_x, target_y, 0.0)
    }

    /// Calculates the defensive line position based on team tactics and defender positions.
    /// Returns the average x-position (goal-to-goal axis) of defenders.
    fn calculate_defensive_line_position(&self, ctx: &StateProcessingContext) -> f32 {
        let defenders: Vec<MatchPlayerLite> = ctx.players().teammates().defenders().collect();

        let sum_x_positions: f32 = defenders.iter().map(|p| p.position.x).sum();
        sum_x_positions / defenders.len() as f32
    }

    /// Checks if an opponent player is nearby within the MARKING_DISTANCE_THRESHOLD.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(MARKING_DISTANCE_THRESHOLD)
    }

    /// Scan for opponents making dangerous runs toward goal
    fn scan_for_dangerous_runs(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let own_goal_position = ctx.ball().direction_to_own_goal();

        let dangerous_runners: Vec<MatchPlayerLite> = ctx
            .players()
            .opponents()
            .nearby(DANGEROUS_RUN_SCAN_DISTANCE)
            .filter(|opp| {
                let velocity = opp.velocity(ctx);
                let speed = velocity.norm();

                // Must be moving at significant speed
                if speed < DANGEROUS_RUN_SPEED {
                    return false;
                }

                // Check if running toward our goal
                let to_goal = (own_goal_position - opp.position).normalize();
                let velocity_dir = velocity.normalize();
                let alignment = velocity_dir.dot(&to_goal);

                if alignment < DANGEROUS_RUN_ANGLE {
                    return false;
                }

                // Check if attacker is in dangerous position relative to this defender
                let defender_x = ctx.player.position.x;
                let is_ahead_or_close = if own_goal_position.x < ctx.context.field_size.width as f32 / 2.0 {
                    opp.position.x < defender_x + 30.0 // Attacker is ahead or close
                } else {
                    opp.position.x > defender_x - 30.0
                };

                alignment >= DANGEROUS_RUN_ANGLE && is_ahead_or_close
            })
            .collect();

        // Return the closest dangerous runner
        dangerous_runners
            .iter()
            .min_by(|a, b| {
                let dist_a = a.distance(ctx);
                let dist_b = b.distance(ctx);
                dist_a.partial_cmp(&dist_b).unwrap()
            })
            .copied()
    }

    /// Determines if the team should set up an offside trap.
    fn should_set_offside_trap(&self, ctx: &StateProcessingContext) -> bool {
        // Check if opponents are positioned ahead of the defensive line
        let defensive_line_position = self.calculate_defensive_line_position(ctx);

        let opponents_ahead = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                if ctx.player.side == Some(PlayerSide::Left) {
                    opponent.position.x < defensive_line_position
                } else {
                    opponent.position.x > defensive_line_position
                }
            })
            .count();

        // If multiple opponents are ahead, consider setting up an offside trap
        opponents_ahead >= 2
    }
}
