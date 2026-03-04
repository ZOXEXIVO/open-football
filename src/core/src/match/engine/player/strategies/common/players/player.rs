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

        let skills = &self.ctx.player.skills;

        // Normalize skills (0.0 to 1.0)
        let finishing_f = (skills.technical.finishing - 1.0) / 19.0;
        let technique_f = (skills.technical.technique - 1.0) / 19.0;
        let first_touch_f = (skills.technical.first_touch - 1.0) / 19.0;
        let long_shots_f = (skills.technical.long_shots - 1.0) / 19.0;
        let composure_f = (skills.mental.composure - 1.0) / 19.0;

        // Core shot accuracy: finishing and technique are dominant
        // Blend finishing vs long_shots based on distance
        let max_field_distance = self.ctx.context.field_size.width as f32;
        let distance_blend = (distance_to_goal / (max_field_distance * 0.3)).clamp(0.0, 1.0);
        let shot_skill = finishing_f * (1.0 - distance_blend) + long_shots_f * distance_blend;

        let base_accuracy = shot_skill * 0.45
            + technique_f * 0.25
            + first_touch_f * 0.15
            + composure_f * 0.15;

        // Distance modifier: closer = more accurate (multiplicative, not part of accuracy blend)
        let distance_modifier = if distance_to_goal < 100.0 {
            1.0
        } else if distance_to_goal < 200.0 {
            1.0 - (distance_to_goal - 100.0) / 400.0 // 1.0 → 0.75
        } else {
            0.6 + long_shots_f * 0.2 // Long shots: 0.6-0.8 based on skill
        };

        // Pressure modifier
        let nearby_defenders = self.ctx.players().opponents().nearby(10.0).count();
        let pressure_modifier = 1.0 - (nearby_defenders as f32 * 0.12).min(0.4);

        // Condition modifier: slight accuracy loss when exhausted
        let condition = self.ctx.player.player_attributes.condition as f32 / 10000.0;
        let condition_modifier = 0.93 + condition * 0.07;

        // Final accuracy (0.0 to ~1.0)
        let accuracy = (base_accuracy * distance_modifier * pressure_modifier * condition_modifier)
            .clamp(0.0, 1.0);

        // Inaccuracy factor: squared so high-skill players are much tighter
        let inaccuracy = (1.0 - accuracy) * (1.0 - accuracy);

        let goal_width = 73.0;
        let mut rng = rand::rng();

        // Placement shot: skilled finishers pick corners from close range
        let is_placement_shot = distance_to_goal < 150.0 && finishing_f > 0.55;

        let mut target = goal_position;

        if is_placement_shot {
            // Close range: aim for a corner
            let y_target = if rng.random_range(0.0..1.0) < 0.5 {
                -goal_width * 0.35
            } else {
                goal_width * 0.35
            };

            // Better finishing = tighter grouping around intended corner
            let y_deviation = rng.random_range(-goal_width * 0.2..goal_width * 0.2) * inaccuracy;
            target.y += y_target + y_deviation;
        } else {
            // Long range / low skill: aim more central with wider spread
            let y_base = rng.random_range(-goal_width * 0.1..goal_width * 0.1);
            let y_deviation = rng.random_range(-goal_width * 0.4..goal_width * 0.4) * inaccuracy;
            target.y += y_base + y_deviation;
        }

        // Technique affects clean contact — poor technique sprays the ball
        let x_deviation = rng.random_range(-5.0..5.0) * inaccuracy;
        target.x += x_deviation;

        // Composure under pressure: defenders nearby cause panic deviation
        if nearby_defenders > 0 {
            let panic = (1.0 - composure_f) * (1.0 - composure_f);
            let panic_y = rng.random_range(-goal_width * 0.12..goal_width * 0.12) * panic;
            let panic_x = rng.random_range(-6.0..6.0) * panic;
            target.y += panic_y;
            target.x += panic_x;
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

        let skills = &self.ctx.player.skills;

        // Technical: passing for weight, technique for clean contact
        let passing = skills.technical.passing / 20.0;
        let technique = skills.technical.technique / 20.0;
        // Physical: strength for raw power capability
        let strength = skills.physical.strength / 20.0;
        // Mental: vision for pass weight judgement, composure for consistency
        let vision = skills.mental.vision / 20.0;
        let composure = skills.mental.composure / 20.0;

        let skill_factor = passing * 0.35 + technique * 0.2 + strength * 0.15
            + vision * 0.15 + composure * 0.15;

        // Condition: slight power loss when exhausted (0-10000 scale)
        // Ranges from 0.92 (exhausted) to 1.0 (fresh)
        let condition = self.ctx.player.player_attributes.condition as f32 / 10000.0;
        let condition_factor = 0.92 + condition * 0.08;

        // Distance scaling: pass power proportional to distance needed
        let max_pass_distance = self.ctx.context.field_size.width as f32 * 0.8;
        let distance_factor = if distance < 10.0 {
            (0.05 + (distance / 10.0) * 0.075).clamp(0.05, 0.125)
        } else {
            (distance / max_pass_distance).clamp(0.125, 1.0)
        };

        let min_power = 0.3;
        let max_power = 2.0;
        let base_power = min_power + (max_power - min_power) * skill_factor * distance_factor;

        base_power * condition_factor
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

        let skills = &self.ctx.player.skills;

        // Technical skills
        let technique = skills.technical.technique / 20.0;
        let long_shots = skills.technical.long_shots / 20.0;
        let finishing = skills.technical.finishing / 20.0;
        // Physical
        let strength = skills.physical.strength / 20.0;
        // Mental: composure under pressure
        let composure = skills.mental.composure / 20.0;

        // Blend finishing (close) vs long_shots (far) based on distance
        let max_field_distance = self.ctx.context.field_size.width as f32;
        let distance_blend = (goal_distance / (max_field_distance * 0.3)).clamp(0.0, 1.0);
        let shot_skill = finishing * (1.0 - distance_blend) + long_shots * distance_blend;

        // Skill multiplier with floor so even low-skill players generate some power
        let skill_multiplier = 0.2 + 0.8 * (
            shot_skill * 0.3 + technique * 0.25 + strength * 0.25 + composure * 0.2
        );

        // Distance factor: longer shots need more power (1.0 close, up to 1.6 far)
        let distance_ratio = (goal_distance / max_field_distance).clamp(0.0, 1.0);
        let distance_factor = 1.0 + distance_ratio * 0.6;

        // Condition: slight power loss when exhausted (0.90 exhausted to 1.0 fresh)
        let condition = self.ctx.player.player_attributes.condition as f32 / 10000.0;
        let condition_factor = 0.90 + condition * 0.10;

        let base_power = 2.0;
        let shooting_power = base_power * skill_multiplier * distance_factor * condition_factor;

        shooting_power.clamp(1.2, 3.5) as f64
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

        let distance_to_player = self.distance_to_player(player_id);

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
        let goal_position = self.opponent_goal_position();
        let direction_to_goal = (goal_position - player_position).normalize();

        let distance_to_goal = self.goal_distance();

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
        // Check global post-goal cooldown (kickoff protection)
        if !self.ctx.context.can_shoot_after_goal() {
            return false;
        }

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
