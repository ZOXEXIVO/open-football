use crate::r#match::StateProcessingContext;

pub const MIN_XG_THRESHOLD: f32 = 0.04;
pub const GOOD_XG_THRESHOLD: f32 = 0.12;
pub const EXCELLENT_XG_THRESHOLD: f32 = 0.25;

pub struct ShotQualityEvaluator;

impl ShotQualityEvaluator {
    /// Evaluate shot quality and return an xG value (0.0 - 1.0)
    pub fn evaluate(ctx: &StateProcessingContext) -> f32 {
        let distance = ctx.player().goal_distance();
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().goal_position();

        // 1. Distance factor - exponential decay
        let distance_factor = Self::distance_factor(distance);

        // 2. Angle factor - visible goal angle + y-offset penalty
        let angle_factor = Self::angle_factor(player_pos.y, goal_pos.y, distance, ctx);

        // 3. Goalkeeper factor
        let gk_factor = Self::goalkeeper_factor(ctx, distance);

        // 4. Defensive pressure
        let pressure_factor = Self::pressure_factor(ctx);

        // 5. Clear shot check — partial obstruction still allows a decent chance
        let clear_factor = if ctx.player().has_clear_shot() {
            1.0
        } else {
            0.4
        };

        // 6. Player skill factor
        let skill_factor = Self::skill_factor(ctx, distance);

        // Combine all factors
        let xg = distance_factor * angle_factor * gk_factor * pressure_factor * clear_factor * skill_factor;

        xg.clamp(0.0, 0.95)
    }

    fn distance_factor(distance: f32) -> f32 {
        if distance <= 10.0 {
            0.80 // Point-blank: very high chance
        } else if distance <= 30.0 {
            // Interpolate 0.80 -> 0.45
            0.80 - (distance - 10.0) / 20.0 * 0.35
        } else if distance <= 60.0 {
            // Interpolate 0.45 -> 0.15
            0.45 - (distance - 30.0) / 30.0 * 0.30
        } else if distance <= 120.0 {
            // Interpolate 0.15 -> 0.04
            0.15 - (distance - 60.0) / 60.0 * 0.11
        } else if distance <= 200.0 {
            // Interpolate 0.04 -> 0.01
            0.04 - (distance - 120.0) / 80.0 * 0.03
        } else {
            0.005
        }
    }

    fn angle_factor(player_y: f32, goal_y: f32, distance: f32, ctx: &StateProcessingContext) -> f32 {
        let field_height = ctx.context.field_size.height as f32;
        let goal_half_width = 36.5; // ~7.32m goal width scaled

        // Calculate angle to both posts
        let y_offset = (player_y - goal_y).abs();
        let left_post_y = goal_y - goal_half_width;
        let right_post_y = goal_y + goal_half_width;

        // Visible angle of goal from player's position
        let angle_left = ((left_post_y - player_y) / distance).atan();
        let angle_right = ((right_post_y - player_y) / distance).atan();
        let visible_angle = (angle_right - angle_left).abs();

        // Normalize: max visible angle is ~0.6 rad from close range
        let angle_score = (visible_angle / 0.6).clamp(0.0, 1.0);

        // Y-offset penalty: shots from very wide positions are harder
        let width_ratio = y_offset / (field_height * 0.5);
        let width_penalty = 1.0 - (width_ratio * 0.6).min(0.7);

        angle_score * width_penalty
    }

    fn goalkeeper_factor(ctx: &StateProcessingContext, distance: f32) -> f32 {
        if let Some(gk) = ctx.players().opponents().goalkeeper().next() {
            let gk_distance = (gk.position - ctx.player.position).magnitude();

            // 1v1 situation (very close to GK)
            if gk_distance < 25.0 && distance < 80.0 {
                return 1.3; // Bonus for 1v1
            }

            let goal_pos = ctx.player().goal_position();
            let gk_to_goal = (goal_pos - gk.position).magnitude();

            // GK off their line = better chance
            if gk_to_goal > 30.0 {
                return 1.2;
            }

            // GK well-positioned = harder
            // Check if GK is on the shot line
            let shot_dir = (goal_pos - ctx.player.position).normalize();
            let to_gk = gk.position - ctx.player.position;
            let projection = to_gk.dot(&shot_dir);

            if projection > 0.0 && projection < distance {
                let gk_on_line_dist = (to_gk - shot_dir * projection).magnitude();
                if gk_on_line_dist < 5.0 {
                    return 0.6; // GK directly in path
                }
            }

            0.85 // Default GK present factor
        } else {
            1.5 // No GK spotted - open goal
        }
    }

    fn pressure_factor(ctx: &StateProcessingContext) -> f32 {
        let very_close = ctx.players().opponents().nearby(5.0).count();
        let close = ctx.players().opponents().nearby(10.0).count();

        if very_close >= 2 {
            0.2
        } else if very_close == 1 {
            0.5
        } else if close >= 2 {
            0.7
        } else if close == 1 {
            0.85
        } else {
            1.0
        }
    }

    fn skill_factor(ctx: &StateProcessingContext, distance: f32) -> f32 {
        let finishing = ctx.player.skills.technical.finishing / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let technique = ctx.player.skills.technical.technique / 20.0;
        let long_shots = ctx.player.skills.technical.long_shots / 20.0;

        let skill = if distance > 100.0 {
            // Long range: long_shots matters most
            long_shots * 0.4 + technique * 0.3 + finishing * 0.2 + composure * 0.1
        } else {
            // Close/medium range: finishing matters most
            finishing * 0.4 + composure * 0.25 + technique * 0.2 + long_shots * 0.15
        };

        // Map skill (0.0-1.0) to multiplier (0.6-1.4)
        0.6 + skill * 0.8
    }
}
