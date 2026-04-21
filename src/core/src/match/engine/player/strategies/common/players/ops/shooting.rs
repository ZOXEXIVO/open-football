use crate::club::player::traits::PlayerTrait;
use crate::r#match::{StateProcessingContext};

/// Operations for shooting decision-making
pub struct ShootingOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

// Realistic shooting distances (field is typically 840 units)
// Real football: most goals from within 18m (~36 units), rare from 30m+ (~60 units)
const MAX_SHOOTING_DISTANCE: f32 = 100.0; // ~50m - absolute max for elite long shots
const MIN_SHOOTING_DISTANCE: f32 = 1.0;
const VERY_CLOSE_RANGE_DISTANCE: f32 = 28.0; // ~14m - anyone can shoot
const CLOSE_RANGE_DISTANCE: f32 = 48.0; // ~24m - close range shots
const OPTIMAL_SHOOTING_DISTANCE: f32 = 70.0; // ~35m - ideal shooting distance
const MEDIUM_RANGE_DISTANCE: f32 = 80.0; // ~40m - medium range shots

// Shooting decision thresholds
const SHOOT_OVER_PASS_CLOSE_THRESHOLD: f32 = 36.0; // Always prefer shooting if closer than this
const SHOOT_OVER_PASS_MEDIUM_THRESHOLD: f32 = 50.0; // Shoot over pass for decent finishers
const EXCELLENT_OPPORTUNITY_CLOSE_RANGE: f32 = 60.0; // Distance for close-range excellent opportunity

// Teammate advantage thresholds (multipliers)
const TEAMMATE_ADVANTAGE_RATIO: f32 = 0.4; // Teammate must be this much closer to prevent shot

impl<'p> ShootingOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        ShootingOperationsImpl { ctx }
    }

    /// Expected-goals estimate for a shot taken right now. Mirrors the
    /// xG formula in `handle_shoot_event` so decisions use the same
    /// quality curve the post-hoc stat does. Returns 0..0.9 on a scale
    /// where 0.55 = penalty-spot chance, 0.08 = 20-yard long shot,
    /// <0.04 = hopeless spray. Used as a pre-shot gate so forwards
    /// don't burn cooldowns on low-quality attempts that real players
    /// would skip in favour of a pass.
    pub fn expected_xg(&self) -> f32 {
        let d = self.ctx.ball().distance_to_opponent_goal();
        let distance_factor = if d <= 10.0 {
            0.55
        } else if d <= 30.0 {
            0.55 - (d - 10.0) / 20.0 * 0.30
        } else if d <= 60.0 {
            0.25 - (d - 30.0) / 30.0 * 0.18
        } else if d <= 120.0 {
            0.07 - (d - 60.0) / 60.0 * 0.05
        } else {
            0.02
        };
        let finishing = (self.ctx.player.skills.technical.finishing / 20.0).clamp(0.0, 1.0);
        let skill_mult = 0.7 + finishing * 0.6; // 0.7 .. 1.3
        // Penalise a pressured / blocked shot — matches the real gameplay
        // where a defender in the corridor drastically reduces xG.
        let clarity_mult = if self.ctx.player().has_clear_shot() { 1.0 } else { 0.40 };
        (distance_factor * skill_mult * clarity_mult).clamp(0.0, 0.90)
    }

    /// Check if player is in shooting range (skill-aware)
    pub fn in_shooting_range(&self) -> bool {
        let distance_to_goal = self.ctx.ball().distance_to_opponent_goal();
        let skills = &self.ctx.player.skills;
        let shooting_skill = skills.technical.finishing / 20.0;
        let long_shot_skill = skills.technical.long_shots / 20.0;

        // Very close range - most players should shoot
        if distance_to_goal <= VERY_CLOSE_RANGE_DISTANCE {
            return shooting_skill >= 0.3; // finishing >= 6
        }

        // Close range shots — need decent finishing ability
        if distance_to_goal <= CLOSE_RANGE_DISTANCE {
            return shooting_skill >= 0.5; // finishing >= 10
        }

        // Medium range shots - requires good finishing
        if distance_to_goal <= OPTIMAL_SHOOTING_DISTANCE {
            return shooting_skill >= 0.6; // finishing >= 12
        }

        // Medium-long range shots — need good long shot ability
        if distance_to_goal <= MEDIUM_RANGE_DISTANCE {
            return long_shot_skill >= 0.65 && shooting_skill >= 0.55;
        }

        // Long range shots — elite players only
        if distance_to_goal <= MAX_SHOOTING_DISTANCE {
            return long_shot_skill >= 0.75 && shooting_skill >= 0.6;
        }

        false
    }

    /// Check for excellent shooting opportunity (clear sight, good distance, no pressure)
    pub fn has_excellent_opportunity(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let clear_shot = self.ctx.player().has_clear_shot();

        // Very close to goal - excellent opportunity if any space
        if distance <= EXCELLENT_OPPORTUNITY_CLOSE_RANGE {
            let low_pressure = !self.ctx.players().opponents().exists(5.0);
            return clear_shot && low_pressure;
        }

        // Medium to optimal range - need good angle too
        if distance > MIN_SHOOTING_DISTANCE && distance <= MEDIUM_RANGE_DISTANCE {
            let low_pressure = !self.ctx.players().opponents().exists(10.0);
            let good_angle = self.has_good_angle();

            return clear_shot && low_pressure && good_angle;
        }

        false
    }

    /// Check shooting angle quality
    pub fn has_good_angle(&self) -> bool {
        let goal_angle = self.ctx.player().goal_angle();
        // Good angle is less than 45 degrees off center
        goal_angle < std::f32::consts::PI / 4.0
    }

    /// Determine if should shoot instead of looking for pass
    pub fn should_shoot_over_pass(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let has_clear_shot = self.ctx.player().has_clear_shot();
        let skills = &self.ctx.player.skills;
        let confidence = skills.mental.composure / 20.0;
        let finishing = skills.technical.finishing / 20.0;
        let long_shots = skills.technical.long_shots / 20.0;
        let teamwork = skills.mental.teamwork / 20.0;

        // Must have clear shot for any shooting decision
        if !has_clear_shot {
            return false;
        }

        // Signature moves (PPMs): two hard-override traits that reshape the
        // whole decision tree. Only apply in realistic ranges so a 100m
        // "shoots from distance" shot still gets filtered out.
        let player = self.ctx.player;
        let prefers_shot = player.has_trait(PlayerTrait::ShootsFromDistance);
        let prefers_pass = player.has_trait(PlayerTrait::LooksForPassRatherThanAttemptShot);

        // Single scan: count opponents within 8 units (reused below)
        let opponents_within_8 = self.ctx.tick_context.grid
            .opponents(self.ctx.player.id, 8.0).count();

        // Check if heavily marked — prefer pass if 2+ opponents very close
        // (a pass-first trait makes players even less willing to shoot here)
        let heavy_marking_threshold = if prefers_pass { 1 } else { 2 };
        if opponents_within_8 >= heavy_marking_threshold && distance > VERY_CLOSE_RANGE_DISTANCE {
            return false;
        }

        // Very close range - almost always shoot (even pass-first players)
        if distance <= VERY_CLOSE_RANGE_DISTANCE {
            return true;
        }

        // Pass-first players need an extra-clean opportunity before shooting
        // anywhere outside the box.
        let finishing_close_threshold = if prefers_pass { 0.55 } else { 0.4 };
        let finishing_medium_threshold = if prefers_pass { 0.65 } else { 0.5 };

        // Close range - shoot if any finishing ability
        if distance <= SHOOT_OVER_PASS_CLOSE_THRESHOLD && finishing > finishing_close_threshold {
            return true;
        }

        // Check if teammates are in MUCH better positions first
        let opponent_goal_pos = self.ctx.player().opponent_goal_position();
        let better_positioned_teammate = self
            .ctx
            .players()
            .teammates()
            .nearby(100.0)
            .any(|t| {
                let t_dist = (t.position - opponent_goal_pos).magnitude();
                t_dist < distance * TEAMMATE_ADVANTAGE_RATIO
            });

        // High teamwork players defer to better-positioned teammates.
        // "Looks for pass" reinforces this; "Shoots from distance" ignores it.
        if better_positioned_teammate && !prefers_shot {
            let deference_threshold = if prefers_pass { 0.45 } else { 0.6 };
            if teamwork > deference_threshold {
                return false;
            }
        }

        // Medium range - shoot if decent skills
        if distance <= SHOOT_OVER_PASS_MEDIUM_THRESHOLD && finishing > finishing_medium_threshold {
            return true;
        }

        // Optimal distance with reasonable ability
        if distance <= OPTIMAL_SHOOTING_DISTANCE
            && (confidence + finishing) / 2.0 > 0.55
        {
            return true;
        }

        // Medium-long range with good long shot skills and no heavy pressure.
        // "Shoots from distance" players lower the long-shot bar significantly
        // and accept a bit more pressure — this is where the PPM most changes
        // match feel (Robben, Lampard, Steven Gerrard-style hits).
        if distance <= MEDIUM_RANGE_DISTANCE
            && (
                (prefers_shot && long_shots > 0.35 && finishing > 0.35 && opponents_within_8 <= 1)
                || (long_shots > 0.5 && finishing > 0.45 && opponents_within_8 == 0)
            )
        {
            return true;
        }

        // "Shoots from distance" opens the door for genuine long-range attempts
        // in the 80-100 unit bracket if the player has real ability.
        if prefers_shot
            && distance <= MAX_SHOOTING_DISTANCE
            && long_shots > 0.6
            && opponents_within_8 == 0
        {
            return true;
        }

        false
    }

    /// Check if in close range for finishing
    pub fn in_close_range(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        distance >= MIN_SHOOTING_DISTANCE && distance <= CLOSE_RANGE_DISTANCE
    }

    /// Check if in optimal shooting distance
    pub fn in_optimal_range(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        distance >= MIN_SHOOTING_DISTANCE && distance <= OPTIMAL_SHOOTING_DISTANCE
    }

    /// Get shooting confidence factor (0.0 - 1.0)
    pub fn shooting_confidence(&self) -> f32 {
        let skills = &self.ctx.player.skills;
        let finishing = skills.technical.finishing / 20.0;
        let composure = skills.mental.composure / 20.0;
        let technique = skills.technical.technique / 20.0;

        let distance_factor = self.distance_factor();
        let pressure_factor = self.pressure_factor();

        // Combine factors
        let skill_factor = finishing * 0.5 + composure * 0.3 + technique * 0.2;

        let base = (skill_factor * distance_factor * pressure_factor).clamp(0.0, 1.0);

        // Trait-flavoured final adjustments
        let player = self.ctx.player;
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let mut adjusted = base;
        if player.has_trait(PlayerTrait::PlacesShots) && distance <= OPTIMAL_SHOOTING_DISTANCE {
            adjusted += 0.05;
        }
        if player.has_trait(PlayerTrait::PowersShots) {
            adjusted += 0.03;
        }
        if player.has_trait(PlayerTrait::ShootsFromDistance) && distance > OPTIMAL_SHOOTING_DISTANCE {
            adjusted += 0.08;
        }
        adjusted.clamp(0.0, 1.0)
    }

    /// Get distance factor for shooting confidence (1.0 = optimal, 0.0 = too far/close)
    fn distance_factor(&self) -> f32 {
        let distance = self.ctx.ball().distance_to_opponent_goal();

        if distance < MIN_SHOOTING_DISTANCE {
            return 0.3; // Too close, awkward angle
        }

        if distance <= OPTIMAL_SHOOTING_DISTANCE {
            // Optimal range - linear increase to peak
            return (distance / OPTIMAL_SHOOTING_DISTANCE).min(1.0);
        }

        if distance <= MAX_SHOOTING_DISTANCE {
            // Beyond optimal - linear decrease
            let beyond_optimal = distance - OPTIMAL_SHOOTING_DISTANCE;
            let range = MAX_SHOOTING_DISTANCE - OPTIMAL_SHOOTING_DISTANCE;
            return 1.0 - (beyond_optimal / range);
        }

        0.0 // Too far
    }

    /// Get pressure factor for shooting confidence (1.0 = no pressure, 0.0 = extreme pressure)
    fn pressure_factor(&self) -> f32 {
        // Single scan at max distance, bucket by distance
        let mut close_opponents = 0;
        let mut medium_opponents = 0;
        for (_id, dist) in self.ctx.tick_context.grid.opponents(self.ctx.player.id, 10.0) {
            if dist <= 5.0 {
                close_opponents += 1;
            }
            medium_opponents += 1;
        }

        if close_opponents >= 2 {
            return 0.3;
        } else if close_opponents == 1 {
            return 0.6;
        } else if medium_opponents >= 2 {
            return 0.8;
        }

        1.0
    }
}
