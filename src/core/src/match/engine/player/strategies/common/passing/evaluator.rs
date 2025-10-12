use crate::r#match::{MatchPlayer, MatchPlayerLite, StateProcessingContext};

/// Comprehensive pass evaluation result
#[derive(Debug, Clone)]
pub struct PassEvaluation {
    /// Overall success probability [0.0 - 1.0]
    pub success_probability: f32,

    /// Risk level [0.0 - 1.0] where 1.0 is highest risk
    pub risk_level: f32,

    /// Expected value of the pass
    pub expected_value: f32,

    /// Breakdown of factors
    pub factors: PassFactors,

    /// Whether this pass is recommended
    pub is_recommended: bool,
}

#[derive(Debug, Clone)]
pub struct PassFactors {
    pub distance_factor: f32,
    pub angle_factor: f32,
    pub pressure_factor: f32,
    pub receiver_positioning: f32,
    pub passer_ability: f32,
    pub receiver_ability: f32,
    pub tactical_value: f32,
}

pub struct PassEvaluator;

impl PassEvaluator {
    /// Evaluate a potential pass from one player to another
    pub fn evaluate_pass(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        receiver: &MatchPlayerLite,
    ) -> PassEvaluation {
        let pass_vector = receiver.position - passer.position;
        let pass_distance = pass_vector.norm();

        // Calculate individual factors
        let distance_factor = Self::calculate_distance_factor(pass_distance, passer);
        let angle_factor = Self::calculate_angle_factor(ctx, passer, receiver);
        let pressure_factor = Self::calculate_pressure_factor(ctx, passer);
        let receiver_positioning = Self::calculate_receiver_positioning(ctx, receiver);
        let passer_ability = Self::calculate_passer_ability(ctx, passer, pass_distance);
        let receiver_ability = Self::calculate_receiver_ability(ctx, receiver);
        let tactical_value = Self::calculate_tactical_value(ctx, receiver);

        let factors = PassFactors {
            distance_factor,
            angle_factor,
            pressure_factor,
            receiver_positioning,
            passer_ability,
            receiver_ability,
            tactical_value,
        };

        // Calculate success probability using weighted factors
        let success_probability = Self::calculate_success_probability(&factors);

        // Calculate risk level (inverse of some success factors)
        let risk_level = Self::calculate_risk_level(&factors);

        // Calculate expected value considering success probability and tactical value
        let expected_value = success_probability * tactical_value;

        // Determine if pass is recommended based on thresholds
        let is_recommended = success_probability > 0.6 && risk_level < 0.7;

        PassEvaluation {
            success_probability,
            risk_level,
            expected_value,
            factors,
            is_recommended,
        }
    }

    /// Calculate how distance affects pass success
    fn calculate_distance_factor(distance: f32, passer: &MatchPlayer) -> f32 {
        let optimal_range = passer.skills.technical.passing * 2.5;
        let max_effective_range = passer.skills.technical.passing * 5.0;

        if distance <= optimal_range {
            // Short to medium passes - very high success
            1.0 - (distance / optimal_range * 0.1)
        } else if distance <= max_effective_range {
            // Long passes - declining success
            let excess = distance - optimal_range;
            let range = max_effective_range - optimal_range;
            0.9 - (excess / range * 0.5)
        } else {
            // Very long passes - poor success rate
            let excess = distance - max_effective_range;
            (0.4 - (excess / 100.0).min(0.3)).max(0.1)
        }
    }

    /// Calculate how the angle between passer's facing and pass direction affects success
    fn calculate_angle_factor(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        let pass_direction = (receiver.position - passer.position).normalize();
        let passer_velocity = ctx.tick_context.positions.players.velocity(passer.id);

        if passer_velocity.norm() < 0.1 {
            // Standing still - can pass in any direction easily
            return 0.95;
        }

        let facing_direction = passer_velocity.normalize();
        let dot_product = pass_direction.dot(&facing_direction);

        // Convert dot product to angle factor
        // 1.0 = same direction, -1.0 = opposite direction
        if dot_product > 0.7 {
            // Forward passes - easiest
            1.0
        } else if dot_product > 0.0 {
            // Diagonal passes - moderate difficulty
            0.8 + (dot_product * 0.2)
        } else if dot_product > -0.5 {
            // Sideways to backward passes - harder
            0.6 + ((dot_product + 0.5) * 0.4)
        } else {
            // Backward passes while moving forward - very difficult
            0.5 + ((dot_product + 1.0) * 0.2)
        }
    }

    /// Calculate pressure on the passer from opponents
    fn calculate_pressure_factor(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
    ) -> f32 {
        const PRESSURE_RADIUS: f32 = 15.0;

        let nearby_opponents: Vec<(u32, f32)> = ctx.tick_context
            .distances
            .opponents(passer.id, PRESSURE_RADIUS)
            .collect();

        if nearby_opponents.is_empty() {
            return 1.0; // No pressure
        }

        // Calculate pressure based on closest opponent and number of opponents
        let closest_distance = nearby_opponents
            .iter()
            .map(|(_, dist)| *dist)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(PRESSURE_RADIUS);

        let num_opponents = nearby_opponents.len() as f32;

        // Pressure from distance
        let distance_pressure = (closest_distance / PRESSURE_RADIUS).clamp(0.0, 1.0);

        // Additional pressure from multiple opponents
        let number_pressure = (1.0 - (num_opponents - 1.0) * 0.15).max(0.5);

        // Mental attributes help under pressure
        let composure_factor = passer.skills.mental.composure / 20.0;
        let decision_factor = passer.skills.mental.decisions / 20.0;

        let base_pressure = distance_pressure * number_pressure;
        let pressure_with_mentals = base_pressure + (1.0 - base_pressure) * composure_factor * decision_factor;

        pressure_with_mentals.clamp(0.3, 1.0)
    }

    /// Evaluate receiver's positioning quality
    fn calculate_receiver_positioning(
        ctx: &StateProcessingContext,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        const SPACE_CHECK_RADIUS: f32 = 10.0;

        // Check how much space the receiver has
        let nearby_opponents: Vec<(u32, f32)> = ctx.tick_context
            .distances
            .opponents(receiver.id, SPACE_CHECK_RADIUS)
            .collect();

        let space_factor = if nearby_opponents.is_empty() {
            1.0 // Lots of space
        } else {
            let closest_distance = nearby_opponents
                .iter()
                .map(|(_, dist)| *dist)
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(SPACE_CHECK_RADIUS);

            (closest_distance / SPACE_CHECK_RADIUS).clamp(0.4, 1.0)
        };

        // Check if receiver is moving into space or standing still
        let receiver_velocity = ctx.tick_context.positions.players.velocity(receiver.id);
        let movement_factor = if receiver_velocity.norm() > 1.0 {
            // Moving - better positioning
            1.1
        } else {
            // Standing still
            0.9
        };

        // Off the ball movement skill affects positioning quality
        let players = ctx.player();
        let skills = players.skills(receiver.id);

        let off_ball_factor = skills.mental.off_the_ball / 20.0;

        (space_factor * movement_factor * (0.8 + off_ball_factor * 0.2)).min(1.0)
    }

    /// Calculate passer's ability to execute this pass
    fn calculate_passer_ability(_ctx: &StateProcessingContext, passer: &MatchPlayer, distance: f32) -> f32 {
        let passing_skill = passer.skills.technical.passing / 20.0;
        let technique_skill = passer.skills.technical.technique / 20.0;
        let vision_skill = passer.skills.mental.vision / 20.0;

        // For short passes, technique matters more
        // For long passes, passing skill matters more
        let short_pass_weight = 1.0 - (distance / 100.0).min(1.0);

        let ability =
            passing_skill * (0.5 + short_pass_weight * 0.2) +
                technique_skill * (0.3 + short_pass_weight * 0.2) +
                vision_skill * 0.2;

        // Condition affects ability
        let condition_factor = passer.player_attributes.condition as f32 / 10000.0;

        (ability * condition_factor).clamp(0.3, 1.0)
    }

    /// Calculate receiver's ability to control the pass
    fn calculate_receiver_ability(ctx: &StateProcessingContext, receiver: &MatchPlayerLite) -> f32 {
        let players = ctx.player();
        let skills = players.skills(receiver.id);

        let first_touch = skills.technical.first_touch / 20.0;
        let technique = skills.technical.technique / 20.0;
        let anticipation = skills.mental.anticipation / 20.0;

        let ability = first_touch * 0.5 + technique * 0.3 + anticipation * 0.2;

        // Condition affects ability
        let player_attributes = players.attributes(receiver.id);
        let condition_factor = player_attributes.condition as f32 / 10000.0;

        (ability * condition_factor).clamp(0.3, 1.0)
    }

    /// Calculate tactical value of the pass
    fn calculate_tactical_value(
        ctx: &StateProcessingContext,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        let ball_position = ctx.tick_context.positions.ball.position;
        let receiver_position = receiver.position;

        // Value increases as we move toward opponent's goal
        let field_width = ctx.context.field_size.width as f32;
        let progress_value = receiver_position.x / field_width;

        // Passes that move the ball forward are more valuable
        let forward_progress = (receiver_position.x - ball_position.x) / field_width;
        let forward_value = forward_progress.max(0.0) * 0.5;

        // Passes to advanced positions are more valuable

        let position_value = match receiver.tactical_positions.position_group() {
            crate::PlayerFieldPositionGroup::Forward => 1.0,
            crate::PlayerFieldPositionGroup::Midfielder => 0.7,
            crate::PlayerFieldPositionGroup::Defender => 0.4,
            crate::PlayerFieldPositionGroup::Goalkeeper => 0.2,
        };

        // Weighted combination
        let tactical_value =
            progress_value * 0.3 +
                forward_value * 0.4 +
                position_value * 0.3;

        tactical_value.clamp(0.2, 1.0)
    }

    /// Calculate overall success probability from factors
    fn calculate_success_probability(factors: &PassFactors) -> f32 {
        // Weighted combination of all factors
        let probability =
            factors.distance_factor * 0.20 +
                factors.angle_factor * 0.15 +
                factors.pressure_factor * 0.15 +
                factors.receiver_positioning * 0.10 +
                factors.passer_ability * 0.15 +
                factors.receiver_ability * 0.10;

        probability.clamp(0.1, 0.99)
    }

    /// Calculate overall risk level
    fn calculate_risk_level(factors: &PassFactors) -> f32 {
        // Risk is inverse of safety factors
        let risk =
            (1.0 - factors.distance_factor) * 0.25 +
                (1.0 - factors.pressure_factor) * 0.25 +
                (1.0 - factors.receiver_positioning) * 0.20;

        risk.clamp(0.0, 1.0)
    }

    /// Find the best pass option from available teammates
    pub fn find_best_pass_option(
        ctx: &StateProcessingContext,
        max_distance: f32,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        for teammate in teammates.nearby(max_distance) {
            let evaluation = Self::evaluate_pass(ctx, ctx.player, &teammate);

            // Score based on expected value, but prefer safer passes when under pressure
            let score = if evaluation.factors.pressure_factor < 0.5 {
                // Under heavy pressure - prioritize safety
                evaluation.success_probability * 1.5
            } else {
                // Normal situation - consider expected value
                evaluation.expected_value
            };

            if score > best_score && evaluation.is_recommended {
                best_score = score;
                best_option = Some(teammate);
            }
        }

        best_option
    }
}