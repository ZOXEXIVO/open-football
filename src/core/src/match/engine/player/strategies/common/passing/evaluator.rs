use crate::r#match::{MatchPlayerLite, StateProcessingContext, MATCH_TIME_MS};

/// Unified system for evaluating pass quality across all player types
pub struct PassEvaluator;

impl PassEvaluator {
    /// Calculate an overall score for a potential pass to a teammate
    pub fn evaluate_pass(
        ctx: &StateProcessingContext,
        target: &MatchPlayerLite,
        max_score: f32,
    ) -> f32 {
        // Base parameters for weighting different factors
        let weights = PassWeights {
            progression: 0.25,      // Forward progression importance
            space: 0.20,            // Open space around target
            risk: 0.30,             // Risk of interception (negative)
            pass_skill_match: 0.15, // How well the pass matches player skills
            tactical: 0.10,         // Tactical alignment with team strategy
        };

        // Calculate individual component scores
        let progression_score = Self::calculate_progression_score(ctx, target);
        let space_score = Self::calculate_space_score(ctx, target);
        let risk_score = Self::calculate_risk_score(ctx, target);
        let skill_match_score = Self::calculate_skill_match_score(ctx, target);
        let tactical_score = Self::calculate_tactical_score(ctx, target);

        // Game state modifiers
        let game_state_modifier = Self::game_state_modifier(ctx);

        // Combine scores with weights and apply game state modifier
        let weighted_score = (
            progression_score * weights.progression +
                space_score * weights.space +
                risk_score * weights.risk +
                skill_match_score * weights.pass_skill_match +
                tactical_score * weights.tactical
        ) * game_state_modifier;

        // Normalize score to desired range
        weighted_score * max_score
    }

    /// Calculate how much the pass progresses the ball forward
    fn calculate_progression_score(ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        let player_position = ctx.player.position;
        let target_position = target.position;
        let goal_position = ctx.player().opponent_goal_position();

        // Calculate distances to goal
        let player_to_goal = (goal_position - player_position).magnitude();
        let target_to_goal = (goal_position - target_position).magnitude();

        // Calculate progression as reduction in distance to goal
        let progression = (player_to_goal - target_to_goal) / player_to_goal;

        // Normalize to 0-1 range
        (progression + 1.0) / 2.0
    }

    /// Calculate how much open space is around the target player
    fn calculate_space_score(ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        let space_radius = 10.0;
        let opponents_nearby = ctx.players().opponents().all()
            .filter(|opponent| {
                let distance = (opponent.position - target.position).magnitude();
                distance <= space_radius
            })
            .count();

        // More space = higher score (inverse relationship to opponent count)
        let max_opponents = 3; // Reasonable maximum for normalization
        1.0 - (opponents_nearby as f32 / max_opponents as f32).min(1.0)
    }

    /// Calculate how well the pass matches the player's skills
    fn calculate_skill_match_score(ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> f32 {
        let passer_skills = &ctx.player.skills;

        // Get relevant skills for passing with more weight on vision and passing
        let pass_accuracy = passer_skills.technical.passing / 20.0;
        let vision = passer_skills.mental.vision / 20.0;
        let composure = passer_skills.mental.composure / 20.0;
        let decision = passer_skills.mental.decisions / 20.0;

        // Target player skills
        let player = ctx.player();
        let target_skills = player.skills(teammate.id);
        let target_first_touch = target_skills.technical.first_touch / 20.0;
        let target_control = target_skills.technical.technique / 20.0;

        // Pass distance affects skill requirement
        let pass_distance = (teammate.position - ctx.player.position).magnitude();
        let distance_difficulty = (pass_distance / 40.0).min(1.0);

        // Calculate passer capability with greater emphasis on vision and passing
        let player_skill = (pass_accuracy * 0.5) + (vision * 0.3) + (composure * 0.1) + (decision * 0.1);

        // Calculate receiver capability
        let receiver_skill = (target_first_touch * 0.6) + (target_control * 0.4);

        // Higher score when both passer and receiver have good skills
        let required_skill = 0.3 + distance_difficulty * 0.7;
        let pass_capability = (player_skill / required_skill).min(1.5);

        // Combined skill match score with more weight on passer capability
        (pass_capability * 0.7) + (receiver_skill * 0.3)
    }
    
    /// Calculate the risk of pass interception
    fn calculate_risk_score(ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        let player_position = ctx.player.position;
        let target_position = target.position;
        let pass_direction = (target_position - player_position).normalize();
        let pass_distance = (target_position - player_position).magnitude();

        // Look for opponents in the passing lane
        let opponents_in_lane = ctx.players().opponents().all()
            .filter(|opponent| {
                // Vector from player to opponent
                let to_opponent = opponent.position - player_position;

                // Project opponent position onto pass direction
                let projection_dist = to_opponent.dot(&pass_direction);

                // Only consider opponents between passer and target
                if projection_dist <= 0.0 || projection_dist >= pass_distance {
                    return false;
                }

                // Calculate perpendicular distance from passing lane
                let projected_point = player_position + pass_direction * projection_dist;
                let perp_distance = (opponent.position - projected_point).magnitude();

                // Consider opponents close to passing lane
                let intercept_width = 3.0 + (projection_dist / pass_distance) * 2.0;
                perp_distance < intercept_width
            })
            .count();

        // Calculate risk based on opponents in lane
        let max_opponents = 2; // Max expected opponents in lane
        let interception_risk = (opponents_in_lane as f32 / max_opponents as f32).min(1.0);

        // Factor in pass distance (longer = riskier)
        let distance_factor = 1.0 - (pass_distance / 50.0).min(1.0).max(0.0);

        // Combined risk score (higher is better - less risky)
        (1.0 - interception_risk) * 0.7 + distance_factor * 0.3
    }

    /// Calculate how well the pass aligns with tactical objectives
    fn calculate_tactical_score(ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        let team = ctx.team();
        let tactics = team.tactics();

        // Basic score based on formation and mentality
        let mut score: f32 = 0.5;

        // Adjust based on tactical alignment
        match tactics.tactic_type {
            // For counter-attacking tactics, prefer longer, progressive passes
            crate::MatchTacticType::T442 => {
                let progression = Self::calculate_progression_score(ctx, target);
                if progression > 0.6 {
                    score += 0.3;
                }
            },

            // Default behavior for other tactics
            _ => {}
        }

        // Consider game phase and player positions
        if target.tactical_positions.is_forward() && ctx.ball().on_own_side() {
            // Long balls to forwards when in defensive third
            score += 0.2;
        }

        score.min(1.0f32)
    }

    /// Modify passing decisions based on game state
    fn game_state_modifier(ctx: &StateProcessingContext) -> f32 {
        let time_elapsed = ctx.context.time.time as f32;
        let total_time = MATCH_TIME_MS as f32;
        let game_progress = time_elapsed / total_time;

        // Adjust passing strategy based on score
        let score_situation = if ctx.team().is_leading() {
            if game_progress > 0.8 {
                // Protect lead late in game - safer passes
                0.8
            } else {
                // Maintain lead - balanced approach
                1.0
            }
        } else if ctx.team().is_loosing() {
            if game_progress > 0.7 {
                // Chasing game late - riskier, progressive passes
                1.3
            } else {
                // Still time to recover - slightly more aggressive
                1.1
            }
        } else {
            // Tied game - normal behavior
            1.0
        };

        score_situation
    }
}

/// Weights for different passing factors
struct PassWeights {
    progression: f32,
    space: f32,
    risk: f32,
    pass_skill_match: f32,
    tactical: f32,
}