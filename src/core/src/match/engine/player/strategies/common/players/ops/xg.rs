use crate::r#match::StateProcessingContext;

pub const MIN_XG_THRESHOLD: f32 = 0.08;
pub const GOOD_XG_THRESHOLD: f32 = 0.12;
pub const EXCELLENT_XG_THRESHOLD: f32 = 0.25;

/// Type of the shot being evaluated. Drives the per-type xG multiplier.
/// Real-football xG models distinguish header from foot, free kick from
/// open play, rebound from build-up — each has a meaningfully different
/// conversion rate at the same distance/angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotType {
    FootOpenPlay,
    Header,
    Volley,
    OneVOne,
    Cutback,
    SetPieceHeader,
    LongShot,
    Rebound,
    Penalty,
    DirectFreeKick,
}

impl ShotType {
    pub fn xg_multiplier(self) -> f32 {
        match self {
            ShotType::FootOpenPlay => 1.00,
            ShotType::Header => 0.55,
            ShotType::Volley => 0.75,
            ShotType::OneVOne => 1.20,
            ShotType::Cutback => 1.25,
            ShotType::SetPieceHeader => 0.55,
            ShotType::LongShot => 1.00,
            ShotType::Rebound => 1.15,
            ShotType::Penalty => 0.76,
            ShotType::DirectFreeKick => 0.55,
        }
    }
}

pub struct ShotQualityEvaluator;

impl ShotQualityEvaluator {
    /// Evaluate shot quality and return an xG value (0.0 - 1.0).
    /// Calls into `evaluate_with_type` with FootOpenPlay default.
    pub fn evaluate(ctx: &StateProcessingContext) -> f32 {
        Self::evaluate_with_type(ctx, ShotType::FootOpenPlay)
    }

    /// Evaluate shot quality for a specific shot type. Headers /
    /// volleys / cutbacks / rebounds / set pieces all have different
    /// conversion rates at the same geometry.
    pub fn evaluate_with_type(ctx: &StateProcessingContext, shot_type: ShotType) -> f32 {
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

        // 7. Shot-type multiplier
        let type_factor = shot_type.xg_multiplier();

        // Combine all factors
        let mut xg = distance_factor
            * angle_factor
            * gk_factor
            * pressure_factor
            * clear_factor
            * skill_factor
            * type_factor;

        // Penalty has a fixed expected value regardless of the geometry
        // factors (the kicker's only opponent is the keeper from 11m).
        if shot_type == ShotType::Penalty {
            xg = 0.76 * skill_factor.clamp(0.85, 1.10);
        }

        // Long-shot cap: anything over 120u with no special multiplier
        // tops out at 0.06.
        if shot_type == ShotType::LongShot && distance > 120.0 {
            xg = xg.min(0.06);
        }

        // Direct free kick: 0.03-0.12 based on distance + skill.
        if shot_type == ShotType::DirectFreeKick {
            let dist_score = (1.0 - (distance.clamp(60.0, 200.0) - 60.0) / 140.0).clamp(0.0, 1.0);
            let skill = (ctx.player.skills.technical.free_kicks / 20.0).clamp(0.0, 1.0);
            xg = (0.03 + dist_score * 0.05 + skill * 0.04).clamp(0.03, 0.12);
        }

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

    fn angle_factor(
        player_y: f32,
        goal_y: f32,
        distance: f32,
        ctx: &StateProcessingContext,
    ) -> f32 {
        let field_height = ctx.context.field_size.height as f32;
        let goal_half_width = 29.0; // ~3.66m half-width = 7.32m real goal width

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
        // Single scan at max distance, bucket by distance
        let mut very_close = 0;
        let mut close = 0;
        for (_id, dist) in ctx.tick_context.grid.opponents(ctx.player.id, 10.0) {
            if dist <= 5.0 {
                very_close += 1;
            }
            close += 1;
        }

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

#[allow(dead_code, unused_imports)]
mod shot_type_tests {
    use super::*;

    #[test]
    fn header_xg_is_lower_than_open_play() {
        assert!(ShotType::Header.xg_multiplier() < ShotType::FootOpenPlay.xg_multiplier());
    }

    #[test]
    fn cutback_xg_higher_than_one_v_one() {
        // Cutbacks are the highest-quality chance type.
        assert!(ShotType::Cutback.xg_multiplier() > ShotType::OneVOne.xg_multiplier());
    }

    #[test]
    fn rebound_xg_above_open_play() {
        assert!(ShotType::Rebound.xg_multiplier() > ShotType::FootOpenPlay.xg_multiplier());
    }

    #[test]
    fn penalty_xg_close_to_real_world() {
        // Real-world penalty conversion ~76%.
        let m = ShotType::Penalty.xg_multiplier();
        assert!((m - 0.76).abs() < 0.01);
    }
}
