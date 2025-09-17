use nalgebra::Vector3;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};

/// Enhanced pass evaluation system with sophisticated decision-making
pub struct PassEvaluator;

impl PassEvaluator {
    /// Main evaluation function with comprehensive pass assessment
    pub fn evaluate_pass(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
        pass_range: f32,
    ) -> f32 {
        let mut score = 0.0;

        // 1. Distance-based score (20% weight)
        let distance_score = Self::evaluate_distance(ctx, teammate, pass_range);
        score += distance_score * 20.0;

        // 2. Pass safety score (25% weight)
        let safety_score = Self::evaluate_pass_safety(ctx, teammate);
        score += safety_score * 25.0;

        // 3. Teammate readiness (15% weight)
        let readiness_score = Self::evaluate_teammate_readiness(ctx, teammate);
        score += readiness_score * 15.0;

        // 4. Progressive pass value (20% weight)
        let progression_score = Self::evaluate_progression(ctx, teammate);
        score += progression_score * 20.0;

        // 5. Space availability (10% weight)
        let space_score = Self::evaluate_space_around_target(ctx, teammate);
        score += space_score * 10.0;

        // 6. Tactical advantage (10% weight)
        let tactical_score = Self::evaluate_tactical_advantage(ctx, teammate);
        score += tactical_score * 10.0;

        score
    }

    /// Evaluate distance factor with non-linear scaling
    fn evaluate_distance(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
        pass_range: f32,
    ) -> f32 {
        let distance = teammate.distance(ctx);

        if distance > pass_range {
            return 0.0; // Out of range
        }

        // Non-linear scoring: optimal at medium range
        let optimal_distance = pass_range * 0.4; // 40% of max range is optimal
        let distance_ratio = distance / pass_range;

        if distance <= optimal_distance {
            // Close passes: good but not optimal
            1.0 - (optimal_distance - distance) / optimal_distance * 0.3
        } else if distance <= pass_range * 0.7 {
            // Medium range: optimal
            1.0
        } else {
            // Long passes: progressively harder
            1.0 - (distance_ratio - 0.7) * 2.0
        }
    }

    /// Evaluate pass safety considering interception risk
    fn evaluate_pass_safety(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        let player_pos = ctx.player.position;
        let teammate_pos = teammate.position;
        let pass_vector = teammate_pos - player_pos;
        let pass_distance = pass_vector.magnitude();

        if pass_distance == 0.0 {
            return 0.0;
        }

        let pass_direction = pass_vector.normalize();
        let mut safety_score = 1.0;

        // Check for opponents in passing lane
        let opponents_in_lane = ctx.players().opponents().all()
            .filter(|opponent| {
                Self::is_in_passing_lane(
                    player_pos,
                    teammate_pos,
                    opponent.position,
                    5.0 // Interception radius
                )
            })
            .count();

        // Reduce score for each opponent in lane
        safety_score -= opponents_in_lane as f32 * 0.3;

        // Check for opponent pressure on passer
        let passer_pressure = Self::calculate_pressure_on_position(ctx, player_pos, 10.0);
        safety_score -= passer_pressure * 0.2;

        // Check for opponent pressure on receiver
        let receiver_pressure = Self::calculate_pressure_on_position(ctx, teammate_pos, 8.0);
        safety_score -= receiver_pressure * 0.15;

        // Consider pass angle difficulty
        let angle_factor = Self::evaluate_pass_angle(ctx, teammate);
        safety_score *= angle_factor;

        safety_score.max(0.0)
    }

    /// Check if a point is in the passing lane
    fn is_in_passing_lane(
        from: Vector3<f32>,
        to: Vector3<f32>,
        point: Vector3<f32>,
        threshold: f32,
    ) -> bool {
        let pass_vector = to - from;
        let pass_length = pass_vector.magnitude();

        if pass_length == 0.0 {
            return false;
        }

        let pass_direction = pass_vector.normalize();
        let to_point = point - from;

        // Project point onto pass line
        let projection = to_point.dot(&pass_direction);

        // Check if projection is within pass segment
        if projection < 0.0 || projection > pass_length {
            return false;
        }

        // Calculate perpendicular distance
        let projected_point = from + pass_direction * projection;
        let perpendicular_distance = (point - projected_point).magnitude();

        perpendicular_distance <= threshold
    }

    /// Calculate pressure on a specific position
    fn calculate_pressure_on_position(
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
        radius: f32,
    ) -> f32 {
        let opponents_nearby = ctx.players().opponents().all()
            .filter(|opp| (opp.position - position).magnitude() <= radius)
            .count();

        // Convert to pressure factor (0.0 to 1.0)
        match opponents_nearby {
            0 => 0.0,
            1 => 0.3,
            2 => 0.6,
            3 => 0.85,
            _ => 1.0,
        }
    }

    /// Evaluate the angle of the pass (behind, square, forward)
    fn evaluate_pass_angle(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        let player_velocity = ctx.player.velocity;
        let to_teammate = teammate.position - ctx.player.position;

        if player_velocity.magnitude() < 0.1 || to_teammate.magnitude() < 0.1 {
            return 1.0; // No movement, any angle is fine
        }

        let velocity_normalized = player_velocity.normalize();
        let pass_normalized = to_teammate.normalize();
        let dot_product = velocity_normalized.dot(&pass_normalized);

        // Forward passes while moving forward: easier
        if dot_product > 0.5 {
            1.0
        }
        // Square passes: moderate
        else if dot_product > -0.5 {
            0.85
        }
        // Backward passes while moving forward: harder
        else {
            0.7
        }
    }

    /// Evaluate teammate's readiness to receive
    fn evaluate_teammate_readiness(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        let mut readiness = 1.0;

        // Check if teammate is moving
        let teammate_velocity = teammate.velocity(ctx);
        let speed = teammate_velocity.magnitude();

        // Stationary teammates are more ready
        if speed < 2.0 {
            readiness *= 1.0;
        } else if speed < 5.0 {
            readiness *= 0.9;
        } else {
            readiness *= 0.75; // Fast-moving teammates harder to find
        }

        // Check if teammate is facing towards passer (approximation)
        if speed > 0.1 {
            let to_passer = ctx.player.position - teammate.position;
            let facing_score = teammate_velocity.normalize().dot(&to_passer.normalize());

            if facing_score > 0.0 {
                // Moving towards passer: good
                readiness *= 1.1;
            } else {
                // Moving away: less ready
                readiness *= 0.9;
            }
        }

        // Check if teammate is marked
        let marking_pressure = Self::calculate_pressure_on_position(ctx, teammate.position, 5.0);
        readiness *= (1.0 - marking_pressure * 0.5);

        readiness.min(1.0).max(0.0)
    }

    /// Evaluate if pass progresses play towards goal
    fn evaluate_progression(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_to_goal = (goal_pos - ctx.player.position).magnitude();
        let teammate_to_goal = (goal_pos - teammate.position).magnitude();

        // Calculate progression distance
        let progression = player_to_goal - teammate_to_goal;

        if progression > 50.0 {
            // Significant forward progression
            1.0
        } else if progression > 20.0 {
            // Moderate progression
            0.8
        } else if progression > 0.0 {
            // Slight progression
            0.6
        } else if progression > -20.0 {
            // Lateral or slight backward
            0.4
        } else {
            // Significant backward pass
            0.2
        }
    }

    /// Evaluate space around the target teammate
    fn evaluate_space_around_target(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        // Count opponents in different radius zones
        let close_opponents = ctx.players().opponents().all()
            .filter(|opp| (opp.position - teammate.position).magnitude() <= 5.0)
            .count();

        let medium_opponents = ctx.players().opponents().all()
            .filter(|opp| {
                let dist = (opp.position - teammate.position).magnitude();
                dist > 5.0 && dist <= 15.0
            })
            .count();

        // Calculate space score
        let close_penalty = close_opponents as f32 * 0.4;
        let medium_penalty = medium_opponents as f32 * 0.1;

        (1.0 - close_penalty - medium_penalty).max(0.0)
    }

    /// Evaluate tactical advantage of the pass
    fn evaluate_tactical_advantage(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        let mut tactical_score = 0.5; // Base score

        // 1. Check for numerical superiority in target area
        let target_area_advantage = Self::evaluate_numerical_superiority(ctx, teammate.position, 20.0);
        tactical_score += target_area_advantage * 0.2;

        // 2. Check if pass switches play
        if Self::is_switch_of_play(ctx, teammate) {
            tactical_score += 0.2;
        }

        // 3. Check if pass breaks lines
        if Self::breaks_defensive_lines(ctx, teammate) {
            tactical_score += 0.3;
        }

        // 4. Position-specific bonuses
        tactical_score += Self::position_specific_bonus(ctx, teammate);

        tactical_score.min(1.0)
    }

    /// Check for numerical superiority in an area
    fn evaluate_numerical_superiority(
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
        radius: f32,
    ) -> f32 {
        let teammates = ctx.players().teammates().all()
            .filter(|t| (t.position - position).magnitude() <= radius)
            .count();

        let opponents = ctx.players().opponents().all()
            .filter(|o| (o.position - position).magnitude() <= radius)
            .count();

        let difference = teammates as i32 - opponents as i32;

        match difference {
            d if d >= 2 => 1.0,   // Strong superiority
            1 => 0.7,              // Slight superiority
            0 => 0.5,              // Equal
            -1 => 0.3,             // Slight inferiority
            _ => 0.0,              // Strong inferiority
        }
    }

    /// Check if pass represents a switch of play
    fn is_switch_of_play(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let field_width = ctx.context.field_size.width as f32;
        let y_difference = (ctx.player.position.y - teammate.position.y).abs();

        // Consider it a switch if moving across more than 40% of field width
        y_difference > field_width * 0.4
    }

    /// Check if pass breaks defensive lines
    fn breaks_defensive_lines(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let player_x = ctx.player.position.x;
        let teammate_x = teammate.position.x;

        // Count opponents between passer and receiver (x-axis)
        let opponents_between = ctx.players().opponents().all()
            .filter(|opp| {
                let opp_x = opp.position.x;
                (opp_x > player_x.min(teammate_x) && opp_x < player_x.max(teammate_x))
            })
            .count();

        // If passing through 2+ opponents, likely breaking lines
        opponents_between >= 2
    }

    /// Position-specific tactical bonuses
    fn position_specific_bonus(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        use crate::PlayerPositionType;

        match teammate.tactical_positions {
            // Passes to strikers in advanced positions
            PlayerPositionType::Striker | PlayerPositionType::ForwardCenter => {
                if Self::is_in_attacking_third(ctx, teammate.position) {
                    0.15
                } else {
                    0.05
                }
            },
            // Passes to attacking midfielders in space
            PlayerPositionType::AttackingMidfielderCenter |
            PlayerPositionType::AttackingMidfielderLeft |
            PlayerPositionType::AttackingMidfielderRight => {
                let space = Self::evaluate_space_around_target(ctx, teammate);
                space * 0.1
            },
            // Passes to wide players when isolated
            PlayerPositionType::MidfielderLeft |
            PlayerPositionType::MidfielderRight |
            PlayerPositionType::WingbackLeft |
            PlayerPositionType::WingbackRight => {
                if Self::is_wide_and_free(ctx, teammate) {
                    0.1
                } else {
                    0.0
                }
            },
            _ => 0.0,
        }
    }

    /// Check if position is in attacking third
    fn is_in_attacking_third(ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let field_length = ctx.context.field_size.width as f32;
        let attacking_third_start = field_length * 0.67;

        // Assuming attacking towards higher x values
        position.x >= attacking_third_start
    }

    /// Check if wide player is free
    fn is_wide_and_free(ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let is_wide = teammate.position.y < field_height * 0.2 ||
            teammate.position.y > field_height * 0.8;

        if !is_wide {
            return false;
        }

        // Check for space
        let opponents_nearby = ctx.players().opponents().all()
            .filter(|opp| (opp.position - teammate.position).magnitude() <= 10.0)
            .count();

        opponents_nearby == 0
    }

    /// Advanced evaluation for through balls
    pub fn evaluate_through_ball(
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
        target_position: Vector3<f32>,
    ) -> f32 {
        let mut score = 0.0;

        // Check if teammate can reach the target position
        let teammate_to_target = (target_position - teammate.position).magnitude();
        let teammate_speed = ctx.context.players.by_id(teammate.id)
            .map(|p| p.skills.physical.pace)
            .unwrap_or(10.0);

        // Check for offside
        if Self::would_be_offside(ctx, target_position) {
            return 0.0;
        }

        // Calculate if teammate can reach ball before opponents
        let time_to_reach = teammate_to_target / teammate_speed;
        let mut can_reach_first = true;

        for opponent in ctx.players().opponents().all() {
            let opp_to_target = (target_position - opponent.position).magnitude();
            let opp_speed = 12.0; // Assume average opponent speed
            let opp_time = opp_to_target / opp_speed;

            if opp_time < time_to_reach * 0.9 {
                can_reach_first = false;
                break;
            }
        }

        if !can_reach_first {
            return 0.0;
        }

        // Base score for viable through ball
        score = 50.0;

        // Add bonus for space behind defense
        let space_score = Self::evaluate_space_around_target(ctx, teammate);
        score += space_score * 30.0;

        // Add bonus for goal proximity
        let goal_distance = (target_position - ctx.player().opponent_goal_position()).magnitude();
        if goal_distance < 30.0 {
            score += 20.0;
        }

        score
    }

    /// Simple offside check (would need proper implementation)
    fn would_be_offside(
        ctx: &StateProcessingContext,
        target_position: Vector3<f32>,
    ) -> bool {
        // Simplified: check if teammate would be behind last defender
        let last_defender_x = ctx.players().opponents().all()
            .map(|opp| opp.position.x)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        target_position.x > last_defender_x + 5.0 // Buffer for offside
    }
}