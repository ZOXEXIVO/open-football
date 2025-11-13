use crate::r#match::{MatchPlayerLite, StateProcessingContext};

/// Operations for shooting decision-making
pub struct ShootingOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

// Realistic shooting distances (field is typically 840 units)
const MAX_SHOOTING_DISTANCE: f32 = 250.0; // ~60m - absolute max for long shots
const MIN_SHOOTING_DISTANCE: f32 = 1.0;
const VERY_CLOSE_RANGE_DISTANCE: f32 = 50.0; // ~20m - anyone can shoot
const CLOSE_RANGE_DISTANCE: f32 = 70.0; // ~30m - close range shots
const OPTIMAL_SHOOTING_DISTANCE: f32 = 110.0; // ~40m - ideal shooting distance
const MEDIUM_RANGE_DISTANCE: f32 = 150.0; // ~45m - medium range shots

// Shooting decision thresholds
const SHOOT_OVER_PASS_CLOSE_THRESHOLD: f32 = 60.0; // Always prefer shooting if closer than this
const SHOOT_OVER_PASS_MEDIUM_THRESHOLD: f32 = 70.0; // Shoot over pass for decent finishers
const EXCELLENT_OPPORTUNITY_CLOSE_RANGE: f32 = 130.0; // Distance for close-range excellent opportunity

// Teammate advantage thresholds (multipliers)
const TEAMMATE_ADVANTAGE_RATIO: f32 = 0.4; // Teammate must be this much closer to prevent shot

impl<'p> ShootingOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        ShootingOperationsImpl { ctx }
    }

    /// Check if player is in shooting range (skill-aware)
    pub fn in_shooting_range(&self) -> bool {
        let distance_to_goal = self.ctx.ball().distance_to_opponent_goal();
        let shooting_skill = self.ctx.player.skills.technical.finishing / 20.0;
        let long_shot_skill = self.ctx.player.skills.technical.long_shots / 20.0;

        // Very close range - even poor finishers should shoot!
        if distance_to_goal <= VERY_CLOSE_RANGE_DISTANCE {
            return true;
        }

        // Close range shots (most common) - almost anyone can shoot from close range
        if distance_to_goal >= MIN_SHOOTING_DISTANCE && distance_to_goal <= CLOSE_RANGE_DISTANCE {
            return true;
        }

        // Medium range shots - requires decent finishing
        if distance_to_goal <= OPTIMAL_SHOOTING_DISTANCE && shooting_skill > 0.5 {
            return true;
        }

        // Medium-long range shots - moderate skill requirement (new tier)
        if distance_to_goal <= MEDIUM_RANGE_DISTANCE
            && long_shot_skill > 0.5
            && shooting_skill > 0.45
        {
            return true;
        }

        // Long range shots - skilled players (reduced from 0.75/0.65 to 0.6/0.5)
        if distance_to_goal <= MAX_SHOOTING_DISTANCE
            && long_shot_skill > 0.6
            && shooting_skill > 0.5
        {
            return true;
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
        let confidence = self.ctx.player.skills.mental.composure / 20.0;
        let finishing = self.ctx.player.skills.technical.finishing / 20.0;
        let long_shots = self.ctx.player.skills.technical.long_shots / 20.0;

        // Close range - almost always shoot if clear
        if distance <= SHOOT_OVER_PASS_CLOSE_THRESHOLD && has_clear_shot {
            return finishing > 0.5 || distance <= VERY_CLOSE_RANGE_DISTANCE;
        }

        // Medium range - shoot if decent skills
        if distance <= SHOOT_OVER_PASS_MEDIUM_THRESHOLD && has_clear_shot && finishing > 0.55 {
            return true;
        }

        // Optimal distance with good overall ability
        if distance <= OPTIMAL_SHOOTING_DISTANCE
            && has_clear_shot
            && (confidence + finishing) / 2.0 > 0.6
        {
            return true;
        }

        // Medium-long range with good long shot skills
        if distance <= MEDIUM_RANGE_DISTANCE
            && has_clear_shot
            && long_shots > 0.5
            && finishing > 0.45
            && !self.ctx.players().opponents().exists(10.0)
        {
            return true;
        }

        // Check if teammates are in MUCH better positions
        let better_positioned_teammate = self
            .ctx
            .players()
            .teammates()
            .nearby(100.0)
            .any(|t| {
                let t_dist = (t.position - self.ctx.player().opponent_goal_position()).magnitude();
                t_dist < distance * TEAMMATE_ADVANTAGE_RATIO
            });

        // Shoot if no better teammate and clear shot within medium range
        !better_positioned_teammate && has_clear_shot && distance <= MEDIUM_RANGE_DISTANCE
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
        let finishing = self.ctx.player.skills.technical.finishing / 20.0;
        let composure = self.ctx.player.skills.mental.composure / 20.0;
        let technique = self.ctx.player.skills.technical.technique / 20.0;

        let distance_factor = self.distance_factor();
        let pressure_factor = self.pressure_factor();

        // Combine factors
        let skill_factor = (finishing * 0.5 + composure * 0.3 + technique * 0.2);

        (skill_factor * distance_factor * pressure_factor).clamp(0.0, 1.0)
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
        let close_opponents = self.ctx.players().opponents().nearby(5.0).count();
        let medium_opponents = self.ctx.players().opponents().nearby(10.0).count();

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
