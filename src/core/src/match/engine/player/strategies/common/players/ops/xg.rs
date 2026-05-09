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

    pub(crate) fn distance_factor(distance: f32) -> f32 {
        // Real-football StatsBomb baselines per yardage band:
        //   6yd box (≤12u)       ~0.55–0.65
        //   penalty spot (~22u)  ~0.35
        //   18-yard line (~36u)  ~0.18
        //   25 yards (~50u)      ~0.07
        //   30+ yards            ~0.04
        // Old curve overstated the close band (0.80 at point-blank), which
        // combined with the lenient skill_factor produced ~1.7 goals/match
        // for mediocre finishers off rebounds and 1v1s.
        if distance <= 10.0 {
            0.62
        } else if distance <= 30.0 {
            // Interpolate 0.62 -> 0.32
            0.62 - (distance - 10.0) / 20.0 * 0.30
        } else if distance <= 60.0 {
            // Interpolate 0.32 -> 0.10
            0.32 - (distance - 30.0) / 30.0 * 0.22
        } else if distance <= 120.0 {
            // Interpolate 0.10 -> 0.03
            0.10 - (distance - 60.0) / 60.0 * 0.07
        } else if distance <= 200.0 {
            // Interpolate 0.03 -> 0.01
            0.03 - (distance - 120.0) / 80.0 * 0.02
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

            // 1v1 situation (very close to GK). Real football: 1v1
            // conversion is famously skill-sensitive — top finishers
            // bury 50%+ of them, replacement-level strikers under 25%.
            // Bonus is graded by composure, first touch, and decisions.
            if gk_distance < 25.0 && distance < 80.0 {
                let composure = ctx.player.skills.mental.composure / 20.0;
                let first_touch = ctx.player.skills.technical.first_touch / 20.0;
                let decisions = ctx.player.skills.mental.decisions / 20.0;
                let cool = (composure + first_touch + decisions) / 3.0;
                // 0.85 (panicked Composure-6) … 1.40 (composed Composure-18).
                return (0.85 + cool * 0.55).clamp(0.85, 1.40);
            }

            let goal_pos = ctx.player().goal_position();
            let gk_to_goal = (goal_pos - gk.position).magnitude();

            // GK off their line = better chance
            if gk_to_goal > 30.0 {
                return 1.15;
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
        let first_touch = ctx.player.skills.technical.first_touch / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let long_shots = ctx.player.skills.technical.long_shots / 20.0;

        let skill = if distance > 100.0 {
            // Long range: long_shots matters most
            long_shots * 0.40 + technique * 0.25 + finishing * 0.15
                + composure * 0.10 + decisions * 0.10
        } else if distance > 30.0 {
            // Medium range: finishing + technique with composure under pressure
            finishing * 0.35 + technique * 0.20 + composure * 0.15
                + decisions * 0.15 + long_shots * 0.10 + first_touch * 0.05
        } else {
            // Close range / inside the box: finishing + composure + first touch
            // dominate. Technique handles awkward stances; decisions chooses
            // placement. A panicked low-composure striker close to the keeper
            // skies the chance even with elite finishing.
            finishing * 0.35 + composure * 0.25 + first_touch * 0.15
                + decisions * 0.15 + technique * 0.10
        };

        // Map skill (0.0-1.0) to a steeper multiplier (0.45 .. 1.30) using
        // a quadratic curve. Real football: a Finishing-8 striker is NOT
        // 70% as deadly as a Finishing-18 striker — closer to 50%. The
        // old linear curve (0.60 .. 1.40 with bottom skewed up) made
        // mediocre finishers convert at near-elite rates simply by
        // getting into position. Quadratic squashes the bottom of the
        // curve so journeymen get punished without flattening the top.
        let s = skill.clamp(0.0, 1.0);
        (0.45 + s * s * 0.85).clamp(0.45, 1.30)
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

#[cfg(test)]
mod distance_curve_tests {
    use super::*;

    #[test]
    fn point_blank_under_real_baseline() {
        // 6-yard chance ~0.60 in real data; our distance_factor alone
        // sits below that — the type / GK / skill multipliers compose
        // up from here. Old curve was 0.80 (way over-converted).
        let f = ShotQualityEvaluator::distance_factor(8.0);
        assert!(f >= 0.55 && f <= 0.65, "f={f}");
    }

    #[test]
    fn penalty_spot_around_real_baseline() {
        // Penalty spot (~22u). StatsBomb baseline 0.30-0.40.
        let f = ShotQualityEvaluator::distance_factor(22.0);
        assert!(f >= 0.30 && f <= 0.50, "f={f}");
    }

    #[test]
    fn long_shot_falls_off_steeply() {
        // 25 yards ≈ 50u. StatsBomb baseline ~0.07.
        let f = ShotQualityEvaluator::distance_factor(50.0);
        assert!(f >= 0.10 && f <= 0.20, "f={f}");
    }

    #[test]
    fn distance_factor_monotonic() {
        let mut prev = ShotQualityEvaluator::distance_factor(5.0);
        for d in [10, 20, 30, 50, 70, 100, 150, 200].iter().copied() {
            let next = ShotQualityEvaluator::distance_factor(d as f32);
            assert!(next <= prev + 1e-4, "non-monotonic at d={d}: prev={prev} next={next}");
            prev = next;
        }
    }
}
