use nalgebra::Vector3;

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};

const MAX_DEFENSIVE_LINE_DEVIATION: f32 = 35.0;  // Tighter line — less room for attackers
const BALL_PROXIMITY_THRESHOLD: f32 = 150.0; // React to ball from further out
const MARKING_DISTANCE_THRESHOLD: f32 = 50.0; // Pick up attackers from further away
const PRESSING_DISTANCE_THRESHOLD: f32 = 60.0; // Step out to press ball carrier earlier
const DANGEROUS_RUN_SCAN_DISTANCE: f32 = 100.0; // Scan wider for dangerous runs
const DANGEROUS_RUN_SPEED: f32 = 2.0; // Detect slower dangerous runs too
const DANGEROUS_RUN_ANGLE: f32 = 0.5; // Wider angle detection for goal-bound runs

#[derive(Default, Clone)]
pub struct DefenderHoldingLineState {}

impl StateProcessingHandler for DefenderHoldingLineState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // BOX EMERGENCY — ball is in our penalty area with an opposing
        // carrier. Break shape and engage. The two closest defenders
        // attack; the rest hold line so the far side isn't exposed.
        if ctx.player().defensive().is_box_emergency_for_me() {
            if let Some(carrier) = ctx.players().opponents().with_ball().next() {
                let d = carrier.distance(ctx);
                if d < 25.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

        // STEP UP — attacker is approaching the penalty area and I'm
        // the closest defender. Meet them outside the box instead of
        // collapsing deep. Real football: defenders engage at the 18-yard
        // line, not at the 6-yard line.
        if ctx.player().defensive().should_step_up_to_meet_attacker() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

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

        // Loose ball nearby — go claim it directly
        if !ctx.ball().is_owned() && ctx.ball().distance() < 40.0 && ctx.ball().speed() < 3.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // Role-driven engagement: if the opponent has the ball, break
        // from the line according to our defensive role. A counter-press
        // window widens the trigger so even distant defenders commit to
        // chasing just after we lose possession.
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance = opponent_with_ball.distance(ctx);

            // Tackle range — but only if I'm the Primary closer than
            // any teammate. Otherwise I hold the line while the closer
            // defender engages. Stops the whole back four lunging at
            // the same carrier.
            let is_primary = matches!(
                ctx.player().defensive().defensive_role_for_ball_carrier(),
                DefensiveRole::Primary
            );
            if distance < 25.0 && is_primary {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            let counter_press_active = ctx.team().has_just_lost_possession();
            let counter_press_range = if counter_press_active {
                40.0 + ctx.team().tactics().counter_press_intensity() * 60.0
            } else {
                0.0
            };

            match ctx.player().defensive().defensive_role_for_ball_carrier() {
                DefensiveRole::Primary => {
                    if distance < PRESSING_DISTANCE_THRESHOLD
                        || (counter_press_active && ctx.ball().distance() < counter_press_range)
                    {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Pressing,
                        ));
                    }
                    // Primary but out of range — chase via Running so we
                    // can close the gap instead of holding the line.
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Running,
                    ));
                }
                DefensiveRole::Cover => {
                    if distance < 100.0 && ctx.ball().on_own_side() {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Covering,
                        ));
                    }
                }
                DefensiveRole::Help => {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Marking,
                    ));
                }
                DefensiveRole::Hold => {
                    // Stay on the line — fall through to run/guard checks
                    // for secondary threats below.
                }
            }
        }

        // Break line to track dangerous runners if we're the best
        // positioned defender for them (no ball carrier scenario, or
        // our role was Hold).
        if let Some(dangerous_runner) = self.scan_for_dangerous_runs(ctx) {
            let distance_to_runner = dangerous_runner.distance(ctx);
            if distance_to_runner < 25.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Guard unmarked attackers in our zone who are trying to get open
        if ctx.ball().on_own_side() {
            if let Some(unmarked) = ctx.player().defensive().find_unmarked_opponent(MARKING_DISTANCE_THRESHOLD * 2.0) {
                let dist = unmarked.distance(ctx);
                if dist < 60.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Guarding,
                    ));
                }
            }
        }

        // React to balls played behind the defensive line (through balls)
        if !ctx.ball().is_owned() && ctx.ball().speed() > 1.0 {
            let ball_pos = ctx.tick_context.positions.ball.position;
            let ball_vel = ctx.tick_context.positions.ball.velocity;
            let own_goal = ctx.ball().direction_to_own_goal();
            let to_goal = (own_goal - ball_pos).normalize();
            let ball_dir = ball_vel.normalize();
            let heading_toward_goal = ball_dir.dot(&to_goal);

            // Ball is moving toward our goal and is close enough to react
            if heading_toward_goal > 0.4 && ctx.ball().distance() < 200.0 {
                // Check if ball is behind us or at our level (through ball)
                let is_behind_or_level = if own_goal.x < ctx.context.field_size.width as f32 / 2.0 {
                    ball_pos.x < ctx.player.position.x + 15.0
                } else {
                    ball_pos.x > ctx.player.position.x - 15.0
                };

                if is_behind_or_level {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Intercepting,
                    ));
                }
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

        // Offside-trap detection used to transition into a dedicated
        // DefenderState::OffsideTrap that was a pass-through — it just
        // bounced back to HoldingLine. Staying in HoldingLine with the
        // same zonal-line logic is the simpler model; if we want trap
        // pressing later, reintroduce as a team-level flag (Phase 2).
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
        const MIN_DISTANCE_THRESHOLD: f32 = 1.0;
        const SLOWING_DISTANCE: f32 = 5.0;

        // Base movement speed - jogging pace for positional adjustments
        let pace_influence = (ctx.player.skills.physical.pace / 20.0).clamp(0.6, 1.2);
        let base_speed = 3.0 * pace_influence;

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
        // Compactness: higher = defenders squeeze toward center/ball side
        let compactness = ctx.team().tactics().compactness();

        let target_y = if let Some(opponent) = nearest_opponent_in_zone {
            // Track opponent laterally but don't go too far from zone
            let opponent_y = opponent.position.y;
            let max_drift = 25.0;
            let drift = (opponent_y - tactical_position.y).clamp(-max_drift, max_drift);
            tactical_position.y + drift
        } else {
            // Shift toward ball side — compactness amplifies this shift
            let ball_offset = ball_position.y - field_center_y;
            let shift = ball_offset * (0.08 + compactness * 0.12);
            tactical_position.y + shift
        };

        Vector3::new(target_x, target_y, 0.0)
    }

    /// Calculates the defensive line position based on team tactics and defender positions.
    /// Uses tactical defensive line height to bias the position forward or deep.
    fn calculate_defensive_line_position(&self, ctx: &StateProcessingContext) -> f32 {
        let (sum_x, count) = ctx.players().teammates().defenders()
            .map(|p| p.position.x)
            .fold((0.0f32, 0u32), |(s, c), x| (s + x, c + 1));

        let avg_x = if count > 0 { sum_x / count as f32 } else { ctx.player.position.x };

        // Apply tactical bias: high line pushes defenders forward, deep block pulls them back
        let line_height = ctx.team().tactics().defensive_line_height();
        let own_goal = ctx.ball().direction_to_own_goal();
        let field_width = ctx.context.field_size.width as f32;

        // line_height 0.0 = stay near own goal, 1.0 = push toward halfway
        // Bias range: ±40 units from average position
        let halfway = field_width / 2.0;
        let toward_halfway = (halfway - own_goal.x).signum();
        let tactical_bias = (line_height - 0.5) * 80.0 * toward_halfway;

        avg_x + tactical_bias
    }

    /// Checks if an opponent player is nearby within the MARKING_DISTANCE_THRESHOLD.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(MARKING_DISTANCE_THRESHOLD)
    }

    /// Scan for opponents making dangerous runs toward goal
    fn scan_for_dangerous_runs(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let own_goal_position = ctx.ball().direction_to_own_goal();

        ctx.players()
            .opponents()
            .nearby(DANGEROUS_RUN_SCAN_DISTANCE)
            .filter(|opp| {
                let velocity = opp.velocity(ctx);
                let speed = velocity.norm();

                if speed < DANGEROUS_RUN_SPEED {
                    return false;
                }

                let to_goal = (own_goal_position - opp.position).normalize();
                let velocity_dir = velocity.normalize();
                let alignment = velocity_dir.dot(&to_goal);

                if alignment < DANGEROUS_RUN_ANGLE {
                    return false;
                }

                let defender_x = ctx.player.position.x;
                let is_ahead_or_close = if own_goal_position.x < ctx.context.field_size.width as f32 / 2.0 {
                    opp.position.x < defender_x + 30.0
                } else {
                    opp.position.x > defender_x - 30.0
                };

                alignment >= DANGEROUS_RUN_ANGLE && is_ahead_or_close
            })
            .min_by(|a, b| {
                let dist_a = a.distance(ctx);
                let dist_b = b.distance(ctx);
                dist_a.partial_cmp(&dist_b).unwrap()
            })
    }

}
