use crate::r#match::result::VectorExtensions;
use crate::r#match::{
    MatchPlayer, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateProcessingContext,
};
use crate::PlayerSkills;
use nalgebra::Vector3;
use rand::Rng;

const SEPARATION_RADIUS: f32 = 20.0;
const SEPARATION_STRENGTH: f32 = 20.0;

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

    pub fn on_own_side(&self) -> bool {
        let field_half_width = self.ctx.context.field_size.width / 2;

        self.ctx.player.side == Some(PlayerSide::Left)
            && self.ctx.player.position.x < field_half_width as f32
    }

    pub fn shooting_direction(&self) -> Vector3<f32> {
        let goal_position = self.opponent_goal_position();

        let goal_height = self.ctx.context.field_size.height as f32;

        let shooting_technique = self.skills(self.ctx.player.id).technical.technique;
        let shooting_accuracy = self.skills(self.ctx.player.id).technical.finishing;

        // Normalize the skill values to a range between 0 and 1
        let technique_factor = (shooting_technique as f32 - 1.0) / 19.0;
        let accuracy_factor = (shooting_accuracy as f32 - 1.0) / 19.0;

        // Calculate the maximum deviation in the y-direction based on the goal height and player skills
        let max_y_deviation = goal_height * 0.15 * (1.0 - technique_factor * accuracy_factor);

        let mut rng = rand::thread_rng();
        let y_offset = rng.gen_range(-max_y_deviation..max_y_deviation);

        let mut shooting_target = goal_position;
        
        shooting_target.y += y_offset;

        shooting_target
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
        let distance = self
            .ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, teammate_id);

        let pass_skill = self.ctx.player.skills.technical.passing / 20.0;

        let max_pass_distance = self.ctx.context.field_size.width as f32 * 0.6;
        let distance_factor = (distance / max_pass_distance).clamp(0.0, 1.0);

        let min_power = 0.5;
        let max_power = 2.5;

        let base_power = min_power + (max_power - min_power) * pass_skill * distance_factor;

        let random_factor = rand::thread_rng().gen_range(0.9..1.1);

        let pass_power = base_power * random_factor;

        pass_power
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
        let player_strength = self.ctx.player.skills.physical.strength;

        // Normalize the skill values to a range of 0.5 to 1.5
        let technique_factor = 0.7 + (shooting_technique - 1.0) / 19.0;
        let power_factor = 0.9 + (shooting_power - 1.0) / 19.0;
        let strength_factor = 0.2 + (player_strength - 1.0) / 19.0;

        // Calculate the shooting power based on the normalized skill values and goal distance
        let base_power = 10.0; // Adjust this value to control the overall shooting power
        let distance_factor = (self.ctx.context.field_size.width as f32 / 2.0 - goal_distance)
            / (self.ctx.context.field_size.width as f32 / 2.0);
        let shooting_power =
            base_power * technique_factor * power_factor * strength_factor * distance_factor;

        // Ensure the shooting power is within a reasonable range
        let min_power = 0.5;
        let max_power = 2.5;

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
        let field_length = self.ctx.context.field_size.width as f32;
        let field_width = self.ctx.context.field_size.width as f32;

        if self.ctx.player.side == Some(PlayerSide::Left) {
            Vector3::new(field_length, field_width / 2.0, 0.0)
        } else {
            Vector3::new(0.0, field_width / 2.0, 0.0)
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

        // Increased parameters
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_STRENGTH: f32 = 20.0;

        // Apply separation from teammates
        for other_player in teammates.nearby(SEPARATION_RADIUS) {
            let to_other = other_player.position - self.ctx.player.position;
            let distance = to_other.magnitude();

            if distance > 0.0 && distance < SEPARATION_RADIUS {
                // Using cubic falloff for stronger close-range separation
                let direction = -to_other.normalize();
                let strength = SEPARATION_STRENGTH * (1.0 - distance / SEPARATION_RADIUS).powf(3.0);
                separation += direction * strength;
            }
        }

        // Apply separation from opponents (slightly weaker effect)
        for other_player in opponents.nearby(SEPARATION_RADIUS * 0.8) {
            let to_other = other_player.position - self.ctx.player.position;
            let distance = to_other.magnitude();

            if distance > 0.0 && distance < SEPARATION_RADIUS * 0.8 {
                let direction = -to_other.normalize();
                let strength = SEPARATION_STRENGTH * 0.7 * (1.0 - distance / (SEPARATION_RADIUS * 0.8)).powf(2.0);
                separation += direction * strength;
            }
        }

        separation
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
