use crate::r#match::{MatchPlayer, MatchPlayerLite, PlayerSide, StateProcessingContext};

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
        let passing_skill = passer.skills.technical.passing;
        let vision_skill = passer.skills.mental.vision;
        let technique_skill = passer.skills.technical.technique;

        // Vision and technique extend effective passing range
        let vision_bonus = (vision_skill / 20.0) * 1.5;
        let _technique_bonus = (technique_skill / 20.0) * 0.5;

        let optimal_range = passing_skill * (2.5 + vision_bonus);
        let max_effective_range = passing_skill * (5.0 + vision_bonus * 2.0);
        let ultra_long_threshold = 200.0;
        let extreme_long_threshold = 300.0;

        if distance <= optimal_range {
            // Short to medium passes - very high success
            1.0 - (distance / optimal_range * 0.1)
        } else if distance <= max_effective_range {
            // Long passes (60-100m) - declining success (less penalty with high vision)
            let excess = distance - optimal_range;
            let range = max_effective_range - optimal_range;
            let base_decline = 0.9 - (excess / range * 0.5);
            // Vision reduces the decline penalty
            base_decline + (vision_skill / 20.0 * 0.1)
        } else if distance <= ultra_long_threshold {
            // Very long passes (100-200m) - vision and technique critical
            let excess = distance - max_effective_range;
            let range = ultra_long_threshold - max_effective_range;
            let skill_factor = (vision_skill / 20.0 * 0.6) + (technique_skill / 20.0 * 0.3);

            let base_factor = 0.5 - (excess / range * 0.25);
            (base_factor + skill_factor * 0.3).clamp(0.2, 0.7)
        } else if distance <= extreme_long_threshold {
            // Ultra-long passes (200-300m) - only elite players can execute
            let skill_factor = (vision_skill / 20.0 * 0.7) + (technique_skill / 20.0 * 0.3);

            // Require high skills for these passes
            if skill_factor > 0.7 {
                // Elite passer - can attempt with decent success
                (0.4 + skill_factor * 0.2).clamp(0.3, 0.6)
            } else if skill_factor > 0.5 {
                // Good passer - risky but possible
                (0.3 + skill_factor * 0.15).clamp(0.2, 0.45)
            } else {
                // Average/poor passer - very low success
                0.15
            }
        } else {
            // Extreme long passes (300m+) - goalkeeper clearances, desperate plays
            let skill_factor = (vision_skill / 20.0 * 0.5) + (technique_skill / 20.0 * 0.35) + (passing_skill / 20.0 * 0.15);

            // Only world-class passers have reasonable success
            if skill_factor > 0.8 {
                0.5 // Elite - still challenging
            } else if skill_factor > 0.6 {
                0.35 // Good - very risky
            } else {
                0.2 // Poor - mostly luck
            }
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
        const VERY_CLOSE_RADIUS: f32 = 3.0;
        const CLOSE_RADIUS: f32 = 7.0;
        const MEDIUM_RADIUS: f32 = 12.0;

        // Check opponents at multiple distance ranges for nuanced space evaluation
        let all_opponents: Vec<(u32, f32)> = ctx.tick_context
            .distances
            .opponents(receiver.id, MEDIUM_RADIUS)
            .collect();

        // Count opponents in each zone
        let very_close_opponents = all_opponents.iter()
            .filter(|(_, dist)| *dist < VERY_CLOSE_RADIUS)
            .count();

        let close_opponents = all_opponents.iter()
            .filter(|(_, dist)| *dist >= VERY_CLOSE_RADIUS && *dist < CLOSE_RADIUS)
            .count();

        let medium_opponents = all_opponents.iter()
            .filter(|(_, dist)| *dist >= CLOSE_RADIUS && *dist < MEDIUM_RADIUS)
            .count();

        // Calculate space quality with heavy penalties for nearby opponents
        let space_factor = if very_close_opponents > 0 {
            // Very tightly marked - poor option
            0.2 - (very_close_opponents as f32 * 0.1).min(0.15)
        } else if close_opponents > 0 {
            // Marked but manageable
            0.6 - (close_opponents as f32 * 0.15).min(0.3)
        } else if medium_opponents > 0 {
            // Some pressure but good space
            0.85 - (medium_opponents as f32 * 0.1).min(0.2)
        } else {
            // Completely free - excellent option
            1.0
        };

        // Check if receiver is moving into space or standing still
        let receiver_velocity = ctx.tick_context.positions.players.velocity(receiver.id);
        let movement_factor = if receiver_velocity.norm() > 1.5 {
            // Moving into space - excellent
            1.15
        } else if receiver_velocity.norm() > 0.5 {
            // Some movement - good
            1.05
        } else {
            // Standing still - acceptable but not ideal
            0.95
        };

        // Off the ball movement skill affects positioning quality
        let players = ctx.player();
        let skills = players.skills(receiver.id);

        let off_ball_factor = skills.mental.off_the_ball / 20.0;
        let positioning_factor = skills.mental.positioning / 20.0;

        (space_factor * movement_factor * (0.7 + off_ball_factor * 0.15 + positioning_factor * 0.15)).clamp(0.1, 1.0)
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
        let passer_position = ctx.player.position;

        // Determine which direction is forward based on player side
        let forward_direction_multiplier = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,  // Left team attacks right (positive X)
            Some(PlayerSide::Right) => -1.0, // Right team attacks left (negative X)
            None => 1.0,
        };

        // Calculate actual forward progress (positive = forward, negative = backward)
        let field_width = ctx.context.field_size.width as f32;
        let forward_progress = ((receiver_position.x - ball_position.x) * forward_direction_multiplier) / field_width;

        // Strong penalty for backward passes, strong reward for forward
        let forward_value = if forward_progress < 0.0 {
            // Backward pass - heavy penalty unless under extreme pressure
            let pressure_factor = 1.0 - ctx.player.skills.mental.composure / 20.0;
            forward_progress * 2.0 * pressure_factor.max(0.3) // -0.6 to -0.1
        } else {
            // Forward pass - strong reward
            forward_progress * 1.5 // Up to 1.5
        };

        // Distance bonus: prefer passes of 20-50m over very short (< 15m) or very long
        let pass_distance = (receiver_position - passer_position).norm();
        let distance_value = if pass_distance < 10.0 {
            // Very short pass - only good under pressure
            0.3
        } else if pass_distance < 20.0 {
            // Short pass - acceptable
            0.6
        } else if pass_distance < 50.0 {
            // Ideal passing range - good progression
            1.0
        } else if pass_distance < 80.0 {
            // Long pass - still valuable
            0.8
        } else if pass_distance < 150.0 {
            // Very long pass - situational
            0.6
        } else {
            // Extreme distance - only with high vision
            let vision_skill = ctx.player.skills.mental.vision / 20.0;
            0.4 * vision_skill
        };

        // Long cross-field passes - reward vision players for switching play
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let technique_skill = ctx.player.skills.technical.technique / 20.0;

        let long_pass_bonus = if pass_distance > 300.0 {
            // Extreme distance (300m+) - goalkeeper goal kicks, desperate clearances
            (vision_skill * 0.5 + technique_skill * 0.3) * 0.5
        } else if pass_distance > 200.0 {
            // Ultra-long diagonal (200-300m) - switches play across entire field
            (vision_skill * 0.45 + technique_skill * 0.25) * 0.4
        } else if pass_distance > 100.0 {
            // Very long cross-field switch (100-200m) - valuable for high vision players
            vision_skill * 0.3
        } else if pass_distance > 60.0 {
            // Long pass (60-100m) - some bonus for vision
            vision_skill * 0.15
        } else {
            0.0
        };

        // Passes to advanced positions are more valuable
        let position_value = match receiver.tactical_positions.position_group() {
            crate::PlayerFieldPositionGroup::Forward => 1.0,
            crate::PlayerFieldPositionGroup::Midfielder => 0.7,
            crate::PlayerFieldPositionGroup::Defender => 0.4,
            crate::PlayerFieldPositionGroup::Goalkeeper => 0.2,
        };

        // Weighted combination - heavily favor forward progress and good distance
        let tactical_value =
            forward_value * 0.50 +        // Increased from 0.35 - heavily favor forward
            distance_value * 0.25 +       // New - prefer medium-distance passes
            position_value * 0.15 +       // Reduced from 0.25
            long_pass_bonus * 0.10;       // Reduced from 0.15

        tactical_value.clamp(0.1, 1.3)
    }

    /// Calculate overall success probability from factors
    fn calculate_success_probability(factors: &PassFactors) -> f32 {
        // Weighted combination of all factors
        // Receiver positioning is now much more important - free players are better targets
        let probability =
            factors.distance_factor * 0.15 +
                factors.angle_factor * 0.12 +
                factors.pressure_factor * 0.12 +
                factors.receiver_positioning * 0.30 +  // Significantly increased from 0.10
                factors.passer_ability * 0.13 +
                factors.receiver_ability * 0.10 +
                factors.tactical_value * 0.08;  // Also consider tactical value

        probability.clamp(0.1, 0.99)
    }

    /// Calculate overall risk level
    fn calculate_risk_level(factors: &PassFactors) -> f32 {
        // Risk is inverse of safety factors
        // Poor receiver positioning (crowded by opponents) is now a major risk
        let risk =
            (1.0 - factors.distance_factor) * 0.20 +
                (1.0 - factors.pressure_factor) * 0.20 +
                (1.0 - factors.receiver_positioning) * 0.40 +  // Increased from 0.20
                (1.0 - factors.receiver_ability) * 0.20;

        risk.clamp(0.0, 1.0)
    }

    /// Calculate interception risk from opponents along the pass path
    fn calculate_interception_risk(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        let pass_vector = receiver.position - passer.position;
        let pass_distance = pass_vector.norm();
        let pass_direction = pass_vector.normalize();

        // Check for opponents who could intercept the pass
        let intercepting_opponents = ctx.players().opponents().all()
            .filter(|opponent| {
                let to_opponent = opponent.position - passer.position;
                let projection_distance = to_opponent.dot(&pass_direction);

                // Only consider opponents between passer and receiver
                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                // Calculate perpendicular distance from pass line
                let projected_point = passer.position + pass_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).norm();

                // Consider opponent's interception ability
                let players = ctx.player();
                let opponent_skills = players.skills(opponent.id);
                let interception_ability = opponent_skills.technical.tackling / 20.0;
                let anticipation = opponent_skills.mental.anticipation / 20.0;

                // Better opponents can intercept from further away
                let effective_radius = 3.0 + (interception_ability + anticipation) * 2.0;

                perp_distance < effective_radius
            })
            .count();

        // Convert count to risk factor
        if intercepting_opponents == 0 {
            0.0  // No risk
        } else if intercepting_opponents == 1 {
            0.3  // Moderate risk
        } else if intercepting_opponents == 2 {
            0.6  // High risk
        } else {
            0.9  // Very high risk
        }
    }

    /// Find the best pass option from available teammates with skill-based personality
    pub fn find_best_pass_option(
        ctx: &StateProcessingContext,
        max_distance: f32,
    ) -> Option<MatchPlayerLite> {
        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        // Determine player's passing personality based on skills
        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let flair_skill = ctx.player.skills.mental.flair / 20.0;
        let decision_skill = ctx.player.skills.mental.decisions / 20.0;
        let composure_skill = ctx.player.skills.mental.composure / 20.0;
        let teamwork_skill = ctx.player.skills.mental.teamwork / 20.0;
        let _anticipation_skill = ctx.player.skills.mental.anticipation / 20.0;

        // Define passing personalities
        let is_playmaker = vision_skill > 0.75 && flair_skill > 0.65; // Creative, through balls
        let is_direct = flair_skill > 0.7 && pass_skill > 0.65; // Risky, aggressive forward passes
        let is_conservative = decision_skill < 0.5 || composure_skill < 0.5; // Safe, sideways passes
        let is_team_player = teamwork_skill > 0.75 && pass_skill > 0.65; // Finds best positioned teammates
        let is_pragmatic = decision_skill > 0.75 && pass_skill > 0.6; // Smart, calculated passes

        // Calculate minimum pass distance based on pressure
        let nearby_opponents = ctx.players().opponents().nearby(15.0).count();
        let under_pressure = nearby_opponents >= 2;

        let min_pass_distance = if under_pressure {
            // Under pressure, allow shorter passes
            5.0
        } else {
            // Not under pressure, prefer passes with some distance
            12.0
        };

        // Get previous ball owner to prevent ping-pong passes
        let previous_owner_id = ctx.ball().previous_owner_id();

        for teammate in ctx.players().teammates().nearby(max_distance) {
            // PING-PONG PREVENTION: Don't pass back to the player who just passed to you
            if let Some(prev_owner) = previous_owner_id {
                if teammate.id == prev_owner {
                    continue; // Skip this teammate
                }
            }

            let pass_distance = (teammate.position - ctx.player.position).norm();

            // MINIMUM DISTANCE FILTER: Skip teammates that are too close unless under pressure
            if pass_distance < min_pass_distance {
                continue;
            }

            let evaluation = Self::evaluate_pass(ctx, ctx.player, &teammate);
            let interception_risk = Self::calculate_interception_risk(ctx, ctx.player, &teammate);

            // Base positioning bonus
            let positioning_bonus = evaluation.factors.receiver_positioning * 2.0;

            // Skill-based space quality evaluation
            let space_quality = if is_conservative {
                // Conservative players HEAVILY penalize any crowding
                if evaluation.factors.receiver_positioning > 0.9 {
                    2.0 // Only completely free players
                } else if evaluation.factors.receiver_positioning > 0.7 {
                    1.2
                } else {
                    0.3 // Avoid any pressure
                }
            } else if is_playmaker {
                // Playmakers trust teammates to handle some pressure
                if evaluation.factors.receiver_positioning > 0.8 {
                    1.6
                } else if evaluation.factors.receiver_positioning > 0.5 {
                    1.3 // Still okay with moderate space
                } else {
                    0.8
                }
            } else if is_direct {
                // Direct players less concerned about space, more about attacking position
                if evaluation.factors.receiver_positioning > 0.6 {
                    1.5
                } else {
                    1.0 // Will attempt tighter passes
                }
            } else {
                // Standard space evaluation
                if evaluation.factors.receiver_positioning > 0.8 {
                    1.5
                } else if evaluation.factors.receiver_positioning > 0.6 {
                    1.2
                } else if evaluation.factors.receiver_positioning > 0.4 {
                    1.0
                } else {
                    0.6
                }
            };

            // Skill-based interception risk tolerance
            let risk_tolerance = if is_direct {
                0.3 // Willing to take risks
            } else if is_conservative {
                0.8 // Avoid any risk
            } else if is_playmaker {
                0.4 // Moderate risk for creative passes
            } else {
                0.5 // Standard risk avoidance
            };

            let interception_penalty = 1.0 - (interception_risk * risk_tolerance);

            // Add distance preference bonus - reward passes in the 15-40m range
            let optimal_distance_bonus = if under_pressure {
                // Under pressure, all safe passes are good
                1.0
            } else if pass_distance >= 15.0 && pass_distance <= 40.0 {
                // Optimal passing range for build-up play
                1.3
            } else if pass_distance < 15.0 {
                // Too short - discouraged unless necessary
                0.7
            } else {
                // Longer passes - still good but not optimal
                1.0
            };

            // Distance preference based on personality
            let distance_preference = if is_playmaker {
                // Playmakers love long through balls and switches
                if pass_distance > 300.0 {
                    // Extreme passes - only if vision is elite
                    if vision_skill > 0.85 {
                        1.8 // World-class playmaker - go for spectacular passes
                    } else {
                        0.8 // Too risky for most
                    }
                } else if pass_distance > 200.0 {
                    // Ultra-long switches - playmaker specialty
                    if vision_skill > 0.75 {
                        1.6 // High vision - loves these passes
                    } else {
                        1.1
                    }
                } else if pass_distance > 100.0 {
                    1.5 // Very long passes - excellent for playmakers
                } else if pass_distance > 80.0 {
                    1.4
                } else if pass_distance > 50.0 {
                    1.2
                } else {
                    1.0
                }
            } else if is_direct {
                // Direct players prefer forward passes of any length
                let forward_progress = teammate.position.x - ctx.player.position.x;
                if forward_progress > 0.0 {
                    1.3
                } else {
                    0.6 // Avoid backward passes
                }
            } else if is_conservative {
                // Conservative players prefer short, safe passes
                if pass_distance < 30.0 {
                    1.4
                } else if pass_distance < 50.0 {
                    1.0
                } else {
                    0.7 // Avoid long passes
                }
            } else if is_team_player {
                // Team players maximize teammate positioning
                1.0 + (evaluation.factors.receiver_positioning * 0.3)
            } else if is_pragmatic {
                // Pragmatic players balance all factors
                if evaluation.expected_value > 0.6 {
                    1.3 // Good tactical value
                } else {
                    1.0
                }
            } else {
                1.0
            };

            // Calculate final score with personality-based weighting
            let score = if evaluation.factors.pressure_factor < 0.5 {
                // Under heavy pressure - personality affects decision
                if is_conservative {
                    // Conservative: safety is paramount
                    (evaluation.success_probability * 2.0 + positioning_bonus) * interception_penalty * space_quality * optimal_distance_bonus
                } else if is_direct {
                    // Direct: still look for forward options
                    (evaluation.expected_value * 1.5 + positioning_bonus * 0.3) * interception_penalty * space_quality * distance_preference * optimal_distance_bonus
                } else {
                    // Others: prioritize safety AND space
                    (evaluation.success_probability + positioning_bonus) * interception_penalty * space_quality * 1.3 * optimal_distance_bonus
                }
            } else {
                // Normal situation - personality-based preferences apply
                if is_playmaker {
                    // Playmakers prioritize tactical value and vision
                    (evaluation.expected_value * 1.3 + positioning_bonus * 0.4) * interception_penalty * space_quality * distance_preference * optimal_distance_bonus
                } else if is_direct {
                    // Direct players maximize attack
                    (evaluation.expected_value * 1.4 + evaluation.factors.tactical_value * 0.5) * interception_penalty * space_quality * distance_preference * optimal_distance_bonus
                } else if is_team_player {
                    // Team players maximize receiver's situation
                    (evaluation.success_probability + positioning_bonus * 0.8) * interception_penalty * space_quality * distance_preference * optimal_distance_bonus
                } else if is_conservative {
                    // Conservative: success probability is key
                    (evaluation.success_probability * 1.5 + positioning_bonus * 0.3) * interception_penalty * space_quality * distance_preference * optimal_distance_bonus
                } else if is_pragmatic {
                    // Pragmatic: balanced approach
                    (evaluation.expected_value * 1.2 + positioning_bonus * 0.5) * interception_penalty * space_quality * distance_preference * optimal_distance_bonus
                } else {
                    // Standard scoring
                    (evaluation.expected_value + positioning_bonus * 0.5) * interception_penalty * space_quality * optimal_distance_bonus
                }
            };

            // Personality-based acceptance threshold
            let is_acceptable = if is_conservative {
                evaluation.success_probability > 0.7 && evaluation.factors.receiver_positioning > 0.75
            } else if is_direct {
                evaluation.success_probability > 0.5 && evaluation.factors.tactical_value > 0.5
            } else if is_playmaker {
                evaluation.success_probability > 0.55 || (evaluation.factors.tactical_value > 0.7 && pass_distance > 60.0)
            } else {
                evaluation.is_recommended || (evaluation.factors.receiver_positioning > 0.7 && evaluation.success_probability > 0.5)
            };

            if score > best_score && is_acceptable {
                best_score = score;
                best_option = Some(teammate);
            }
        }

        best_option
    }
}