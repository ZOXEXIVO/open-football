use crate::r#match::result::VectorExtensions;
use crate::r#match::{
    MatchPlayer, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateProcessingContext,
};
use crate::{PlayerAttributes, PlayerSkills};
use nalgebra::Vector3;
use rand::RngExt;
use crate::r#match::player::strategies::players::{DefensiveOperationsImpl, MovementOperationsImpl, PassingOperationsImpl, PressureOperationsImpl, ShootingOperationsImpl, SkillOperationsImpl, ShotQualityEvaluator, MIN_XG_THRESHOLD};

pub struct PlayerOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> PlayerOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        PlayerOperationsImpl { ctx }
    }
}

impl<'p> PlayerOperationsImpl<'p> {
    pub fn get(&self, player_id: u32) -> MatchPlayerLite {
        MatchPlayerLite {
            id: player_id,
            position: self.ctx.tick_context.positions.players.position(player_id),
            tactical_positions: self
                .ctx
                .context
                .players
                .by_id(player_id)
                .expect(&format!("unknown player = {}", player_id))
                .tactical_position
                .current_position,
        }
    }

    pub fn skills(&self, player_id: u32) -> &PlayerSkills {
        let player = self.ctx.context.players.by_id(player_id).unwrap();
        &player.skills
    }

    pub fn attributes(&self, player_id: u32) -> &PlayerAttributes {
        let player = self.ctx.context.players.by_id(player_id).unwrap();
        &player.player_attributes
    }

    pub fn on_own_side(&self) -> bool {
        let field_half_width = self.ctx.context.field_size.width / 2;

        self.ctx.player.side == Some(PlayerSide::Left)
            && self.ctx.player.position.x < field_half_width as f32
    }

    pub fn shooting_direction(&self) -> Vector3<f32> {
        let goal_position = self.opponent_goal_position();
        let distance_to_goal = self.goal_distance();

        // Get player skills
        let finishing = self.skills(self.ctx.player.id).technical.finishing;
        let technique = self.skills(self.ctx.player.id).technical.technique;
        let composure = self.skills(self.ctx.player.id).mental.composure;
        let long_shots = self.skills(self.ctx.player.id).technical.long_shots;

        // Normalize skills (0.0 to 1.0)
        let finishing_factor = (finishing - 1.0) / 19.0;
        let technique_factor = (technique - 1.0) / 19.0;
        let composure_factor = (composure - 1.0) / 19.0;
        let long_shots_factor = (long_shots - 1.0) / 19.0;

        // Check pressure from defenders
        let nearby_defenders = self.ctx.players().opponents().nearby(10.0).count();
        let pressure_factor = 1.0 - (nearby_defenders as f32 * 0.15).min(0.5);

        // Distance factor (closer = more accurate)
        let distance_factor = if distance_to_goal < 150.0 {
            1.0 - (distance_to_goal / 300.0)
        } else {
            // For long shots, use long_shots skill
            0.5 * long_shots_factor
        };

        // Overall accuracy (0.0 to 1.0, higher = more accurate)
        let accuracy = (finishing_factor * 0.4
                      + technique_factor * 0.25
                      + composure_factor * 0.2
                      + distance_factor * 0.15)
                      * pressure_factor;

        // Goal dimensions (using goal post standard size)
        let goal_width = 73.0; // Standard goal width in decimeters

        let mut rng = rand::rng();

        // Determine shot type based on distance and skills
        let is_placement_shot = distance_to_goal < 150.0 && finishing > 12.0;

        let mut target = goal_position;

        if is_placement_shot {
            // Close range: Aim for corners (like real strikers)
            // Choose a corner based on angle and randomness
            let aim_preference = rng.random_range(0.0..1.0);

            // Determine horizontal target (Y-axis - width)
            let y_target = if aim_preference < 0.5 {
                // Aim for left post area
                -goal_width * 0.35
            } else {
                // Aim for right post area
                goal_width * 0.35
            };

            // Determine vertical target (Z-axis - height, if supported)
            // For now, shots are 2D so we focus on Y placement

            // Add accuracy-based deviation from intended corner
            let y_deviation = rng.random_range(-goal_width * 0.2..goal_width * 0.2) * (1.0 - accuracy);
            target.y += y_target + y_deviation;

        } else {
            // Long range: More central but with larger deviation
            // Players try to keep it on target rather than picking corners
            let y_base = rng.random_range(-goal_width * 0.15..goal_width * 0.15);

            // Larger deviation for long shots based on accuracy
            let y_deviation = rng.random_range(-goal_width * 0.35..goal_width * 0.35) * (1.0 - accuracy);
            target.y += y_base + y_deviation;
        }

        // Add slight depth deviation (X-axis) based on technique
        // Poor technique can cause shots to go over or fall short
        let x_deviation = rng.random_range(-5.0..5.0) * (1.0 - technique_factor);
        target.x += x_deviation;

        // Mental composure affects shot under pressure
        if nearby_defenders > 0 {
            let panic_factor = 1.0 - composure_factor;
            let panic_deviation_y = rng.random_range(-goal_width * 0.15..goal_width * 0.15) * panic_factor;
            let panic_deviation_x = rng.random_range(-8.0..8.0) * panic_factor;

            target.y += panic_deviation_y;
            target.x += panic_deviation_x;
        }

        target
    }

    pub fn opponent_goal_position(&self) -> Vector3<f32> {
        match self.ctx.player.side {
            Some(PlayerSide::Left) => self.ctx.context.goal_positions.right,
            Some(PlayerSide::Right) => self.ctx.context.goal_positions.left,
            _ => Vector3::new(0.0, 0.0, 0.0),
        }
    }

    pub fn distance_from_start_position(&self) -> f32 {
        self.ctx
            .player
            .start_position
            .distance_to(&self.ctx.player.position)
    }

    pub fn position_to_distance(&self) -> PlayerDistanceFromStartPosition {
        MatchPlayerLogic::distance_to_start_position(self.ctx.player)
    }

    pub fn is_tired(&self) -> bool {
        self.ctx.player.player_attributes.condition_percentage() > 50
    }

    pub fn pass_teammate_power(&self, teammate_id: u32) -> f32 {
        let distance = self.ctx.tick_context.distances.get(self.ctx.player.id, teammate_id);

        // Use multiple skills to determine pass power
        let pass_skill = self.ctx.player.skills.technical.passing / 20.0;
        let technique_skill = self.ctx.player.skills.technical.technique / 20.0;
        let strength_skill = self.ctx.player.skills.physical.strength / 20.0;

        // Calculate skill-weighted factor
        let skill_factor = (pass_skill * 0.6) + (technique_skill * 0.2) + (strength_skill * 0.2);

        // More skilled players can hit passes at more appropriate power levels
        let max_pass_distance = self.ctx.context.field_size.width as f32 * 0.8;

        // Use distance-scaled power with proper minimum for very short passes
        // Very short passes (< 10m) should use proportionally less power
        let distance_factor = if distance < 10.0 {
            // For very short passes, scale linearly from 0.05 to 0.125
            (0.05 + (distance / 10.0) * 0.075).clamp(0.05, 0.125)
        } else {
            // For longer passes, use the normal scaling with lower minimum
            (distance / max_pass_distance).clamp(0.125, 1.0)
        };

        // Calculate base power with adjusted ranges for better control
        let min_power = 0.3;
        let max_power = 2.5;
        let base_power = min_power + (max_power - min_power) * skill_factor * distance_factor;

        // Add slight randomization
        let random_factor = rand::rng().random_range(0.9..1.1);

        // Players with better skills have less randomization
        let final_random_factor = 1.0 + (random_factor - 1.0) * (1.0 - skill_factor * 0.5);

        base_power * final_random_factor
    }

    pub fn kick_teammate_power(&self, teammate_id: u32) -> f32 {
        let distance = self
            .ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, teammate_id);

        let kick_skill = self.ctx.player.skills.technical.free_kicks / 20.0;

        let raw_power = distance / (kick_skill * 100.0);

        let min_power = 0.1;
        let max_power = 1.0;
        let normalized_power = (raw_power - min_power) / (max_power - min_power);

        normalized_power.clamp(0.0, 1.0)
    }

    pub fn throw_teammate_power(&self, teammate_id: u32) -> f32 {
        let distance = self
            .ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, teammate_id);

        let throw_skill = self.ctx.player.skills.technical.long_throws / 20.0;

        let raw_power = distance / (throw_skill * 100.0);

        let min_power = 0.1;
        let max_power = 1.0;
        let normalized_power = (raw_power - min_power) / (max_power - min_power);

        normalized_power.clamp(0.0, 1.0)
    }

    pub fn shoot_goal_power(&self) -> f64 {
        let goal_distance = self.goal_distance();

        // Calculate the base shooting power based on the player's relevant skills
        let shooting_technique = self.ctx.player.skills.technical.technique;
        let shooting_power = self.ctx.player.skills.technical.long_shots;
        let finishing_skill = self.ctx.player.skills.technical.finishing;
        let player_strength = self.ctx.player.skills.physical.strength;

        // Normalize the skill values to a range between 0.5 and 1.5
        let technique_factor = 0.5 + (shooting_technique / 20.0);
        let power_factor = 0.5 + (shooting_power / 20.0);
        let finishing_factor = 0.5 + (finishing_skill / 20.0);
        let strength_factor = 0.3 + (player_strength / 20.0) * 0.7;

        // Calculate distance factor that increases power for longer distances
        // Close shots: ~1.0, Long shots: ~1.6
        let max_field_distance = self.ctx.context.field_size.width as f32;
        let distance_ratio = (goal_distance / max_field_distance).clamp(0.0, 1.0);
        let distance_factor = 1.0 + distance_ratio * 0.6;

        // Calculate the shooting power - moderate increase from original
        let base_power = 3.5;
        let skill_multiplier = (technique_factor * 0.3)
            + (power_factor * 0.35)
            + (finishing_factor * 0.2)
            + (strength_factor * 0.15);

        let shooting_power = base_power * skill_multiplier * distance_factor;

        // Ensure the shooting power is within a reasonable range
        let min_power = 2.0;
        let max_power = 5.5;

        shooting_power.clamp(min_power, max_power) as f64
    }

    pub fn distance_to_player(&self, player_id: u32) -> f32 {
        self.ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, player_id)
    }

    pub fn goal_angle(&self) -> f32 {
        // Calculate the angle between the player's facing direction and the goal direction
        let player_direction = self.ctx.player.velocity.normalize();
        let goal_direction = (self.goal_position() - self.ctx.player.position).normalize();
        player_direction.angle(&goal_direction)
    }

    pub fn goal_distance(&self) -> f32 {
        let player_position = self.ctx.player.position;
        let goal_position = self.goal_position();
        (player_position - goal_position).magnitude()
    }

    pub fn goal_position(&self) -> Vector3<f32> {
        let field_width = self.ctx.context.field_size.width as f32;
        let field_height = self.ctx.context.field_size.height as f32;

        if self.ctx.player.side == Some(PlayerSide::Left) {
            Vector3::new(field_width, field_height / 2.0, 0.0)
        } else {
            Vector3::new(0.0, field_height / 2.0, 0.0)
        }
    }

    pub fn has_clear_pass(&self, player_id: u32) -> bool {
        let player_position = self.ctx.player.position;
        let target_player_position = self.ctx.tick_context.positions.players.position(player_id);
        let direction_to_player = (target_player_position - player_position).normalize();

        // Check if the distance to the target player is within a reasonable pass range
        let distance_to_player = self.ctx.player().distance_to_player(player_id);

        // Check if there are any opponents obstructing the pass
        let ray_cast_result = self.ctx.tick_context.space.cast_ray(
            player_position,
            direction_to_player,
            distance_to_player,
            false,
        );

        ray_cast_result.is_none()
    }

    pub fn has_clear_shot(&self) -> bool {
        let player_position = self.ctx.player.position;
        let goal_position = self.ctx.player().opponent_goal_position();
        let direction_to_goal = (goal_position - player_position).normalize();

        // Check if the distance to the goal is within the player's shooting range
        let distance_to_goal = self.ctx.player().goal_distance();

        // Check if there are any opponents obstructing the shot
        let ray_cast_result = self.ctx.tick_context.space.cast_ray(
            player_position,
            direction_to_goal,
            distance_to_goal,
            false,
        );

        ray_cast_result.is_none()
    }

    pub fn separation_velocity(&self) -> Vector3<f32> {
        let players = self.ctx.players();
        let teammates = players.teammates();
        let opponents = players.opponents();

        let mut separation = Vector3::zeros();

        // Balanced parameters to prevent oscillation while maintaining separation
        const SEPARATION_RADIUS: f32 = 20.0;
        const SEPARATION_STRENGTH: f32 = 15.0; // Reduced to prevent separation canceling pressing forces
        const MIN_SEPARATION_DISTANCE: f32 = 3.0; // Reduced threshold for emergency separation

        // Apply separation from teammates
        for other_player in teammates.nearby(SEPARATION_RADIUS) {
            let to_other = other_player.position - self.ctx.player.position;
            let distance = to_other.magnitude();

            if distance > 0.0 && distance < SEPARATION_RADIUS {
                // Using cubic falloff for smoother separation (reduced from quartic)
                let direction = -to_other.normalize();
                let strength = SEPARATION_STRENGTH * (1.0f32 - distance / SEPARATION_RADIUS).powf(3.0);
                separation += direction * strength;

                // Gentle emergency separation when very close (reduced multiplier to prevent oscillation)
                if distance < MIN_SEPARATION_DISTANCE {
                    let emergency_multiplier = (MIN_SEPARATION_DISTANCE / distance).min(1.5); // Reduced from 3.0x to 1.5x
                    separation += direction * SEPARATION_STRENGTH * emergency_multiplier * 0.5; // Half strength
                }
            }
        }

        // Apply separation from opponents (slightly stronger effect)
        for other_player in opponents.nearby(SEPARATION_RADIUS * 0.8) {
            let to_other = other_player.position - self.ctx.player.position;
            let distance = to_other.magnitude();

            if distance > 0.0 && distance < SEPARATION_RADIUS * 0.8 {
                let direction = -to_other.normalize();
                let strength = SEPARATION_STRENGTH * 0.8 * (1.0f32 - distance / (SEPARATION_RADIUS * 0.8)).powf(3.0);
                separation += direction * strength;

                // Gentle emergency separation when very close (reduced to prevent oscillation)
                if distance < MIN_SEPARATION_DISTANCE {
                    let emergency_multiplier = (MIN_SEPARATION_DISTANCE / distance).min(1.5); // Reduced from 2.5x to 1.5x
                    separation += direction * SEPARATION_STRENGTH * 0.4 * emergency_multiplier; // Reduced strength
                }
            }
        }

        // Add minimal random jitter to separation for natural movement (reduced to prevent twitching)
        if separation.magnitude() > 0.1 {
            let jitter = Vector3::new(
                (rand::random::<f32>() - 0.5) * 0.3, // Reduced from 0.8 to 0.3
                (rand::random::<f32>() - 0.5) * 0.3, // Reduced from 0.8 to 0.3
                0.0,
            );
            separation += jitter;
        }

        // Clamp separation force to reasonable limits to prevent excessive velocities
        // Separation should add to steering, not dominate it
        const MAX_SEPARATION_FORCE: f32 = 15.0;
        let separation_magnitude = separation.magnitude();
        if separation_magnitude > MAX_SEPARATION_FORCE {
            separation = separation * MAX_SEPARATION_FORCE / separation_magnitude;
        }

        separation
    }

    /// Get pressure operations for assessing game pressure
    pub fn pressure(&self) -> PressureOperationsImpl<'p> {
        PressureOperationsImpl::new(self.ctx)
    }

    /// Get shooting operations for shooting decisions
    pub fn shooting(&self) -> ShootingOperationsImpl<'p> {
        ShootingOperationsImpl::new(self.ctx)
    }

    /// Get passing operations for passing decisions
    pub fn passing(&self) -> PassingOperationsImpl<'p> {
        PassingOperationsImpl::new(self.ctx)
    }

    /// Get defensive operations for defensive positioning
    pub fn defensive(&self) -> DefensiveOperationsImpl<'p> {
        DefensiveOperationsImpl::new(self.ctx)
    }

    /// Get movement operations for space-finding and positioning
    pub fn movement(&self) -> MovementOperationsImpl<'p> {
        MovementOperationsImpl::new(self.ctx)
    }

    /// Get skill operations for skill-based calculations
    pub fn skill(&self) -> SkillOperationsImpl<'p> {
        SkillOperationsImpl::new(self.ctx)
    }

    /// Check if the player should attempt a shot based on cooldown and xG
    pub fn should_attempt_shot(&self) -> bool {
        let current_tick = self.ctx.current_tick();

        // Check shot cooldown
        if !self.ctx.memory().can_shoot(current_tick) {
            return false;
        }

        // Evaluate xG
        let xg = ShotQualityEvaluator::evaluate(self.ctx);

        // Adjust threshold based on confidence and intentions
        let confidence = self.ctx.memory().confidence;
        let has_shoot_intention = self.ctx.memory().has_intention(
            &crate::r#match::player::memory::IntentionKind::LookingToShoot,
        );

        let mut threshold = MIN_XG_THRESHOLD;

        // Lower threshold if player is confident
        if confidence > 0.7 {
            threshold *= 0.7;
        }

        // Lower threshold if player intends to shoot
        if has_shoot_intention {
            threshold *= 0.8;
        }

        xg >= threshold
    }
}

pub struct MatchPlayerLogic;

impl MatchPlayerLogic {
    pub fn distance_to_start_position(player: &MatchPlayer) -> PlayerDistanceFromStartPosition {
        let start_position_distance = player.position.distance_to(&player.start_position);

        if start_position_distance < 100.0 {
            PlayerDistanceFromStartPosition::Small
        } else if start_position_distance < 250.0 {
            PlayerDistanceFromStartPosition::Medium
        } else {
            PlayerDistanceFromStartPosition::Big
        }
    }
}
