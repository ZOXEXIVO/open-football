use crate::r#match::result::VectorExtensions;
use crate::r#match::{
    MatchPlayer, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateProcessingContext,
};
use crate::PlayerSkills;
use nalgebra::Vector3;
use rand::Rng;

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

        if let Some(side) = self.ctx.player.side {
            return side == PlayerSide::Left
                && self.ctx.player.position.x < field_half_width as f32;
        }

        false
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

    pub fn pass_teammate_power(&self, teammate_id: u32) -> f64 {
        let distance = self
            .ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, teammate_id);

        let pass_skill = self.ctx.player.skills.technical.passing;

        // Calculate the base power based on distance and passing skill
        let base_power = distance / ((pass_skill as f32 + 30.0) * 20.0);

        // Introduce a random component to add variability
        let random_factor = rand::thread_rng().gen_range(0.8..1.2);

        // Calculate the final pass power
        let pass_power = base_power * random_factor;

        // Clamp the pass power between a minimum and maximum value
        let min_power = 1.0;
        let max_power = 2.0;

        pass_power.clamp(min_power, max_power) as f64
    }

    pub fn kick_teammate_power(&self, teammate_id: u32) -> f64 {
        let distance = self
            .ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, teammate_id);

        let kick_skill = self.ctx.player.skills.technical.free_kicks / 20.0;

        let raw_power = (distance / (kick_skill * 100.0)) as f64;

        let min_power = 0.1;
        let max_power = 1.0;
        let normalized_power = (raw_power - min_power) / (max_power - min_power);

        normalized_power.clamp(0.0, 1.0)
    }

    pub fn throw_teammate_power(&self, teammate_id: u32) -> f64 {
        let distance = self
            .ctx
            .tick_context
            .distances
            .get(self.ctx.player.id, teammate_id);

        let throw_skill = self.ctx.player.skills.technical.long_throws / 20.0;

        let raw_power = (distance / (throw_skill * 100.0)) as f64;

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
        let technique_factor = 0.5 + (shooting_technique - 1.0) / 19.0;
        let power_factor = 0.5 + (shooting_power - 1.0) / 19.0;
        let strength_factor = 0.5 + (player_strength - 1.0) / 19.0;

        // Calculate the shooting power based on the normalized skill values and goal distance
        let base_power = 10.0; // Adjust this value to control the overall shooting power
        let distance_factor = (self.ctx.context.field_size.width as f32 / 2.0 - goal_distance)
            / (self.ctx.context.field_size.width as f32 / 2.0);
        let shooting_power =
            base_power * technique_factor * power_factor * strength_factor * distance_factor;

        // Ensure the shooting power is within a reasonable range
        let min_power = 0.5;
        let max_power = 2.0;

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

    pub fn has_clear_shot(&self) -> bool {
        let player_position = self.ctx.player.position;
        let goal_position = self.ctx.ball().direction_to_opponent_goal();
        let direction_to_goal = (goal_position - player_position).normalize();

        // Check if the distance to the goal is within the player's shooting range
        let distance_to_goal = (goal_position - player_position).magnitude();
        let max_shooting_distance = calculate_max_shooting_distance(self.ctx.player);

        if distance_to_goal > max_shooting_distance {
            return false;
        }

        // Check if there are any opponents obstructing the shot
        let ray_cast_result = self.ctx.tick_context.space.cast_ray(
            player_position,
            direction_to_goal,
            distance_to_goal,
            false,
        );

        return ray_cast_result.is_none();

        fn calculate_max_shooting_distance(player: &MatchPlayer) -> f32 {
            let long_shots_skill = player.skills.technical.long_shots;
            let technique_skill = player.skills.technical.technique;
            let strength_skill = player.skills.physical.strength;

            // Calculate the base maximum shooting distance
            let base_distance = 350.0;

            // Calculate the additional distance based on long shots skill
            let long_shots_factor = long_shots_skill / 20.0;
            let long_shots_distance = base_distance * long_shots_factor * 0.5;

            // Calculate the additional distance based on technique skill
            let technique_factor = technique_skill / 20.0;
            let technique_distance = base_distance * technique_factor * 0.3;

            // Calculate the additional distance based on strength skill
            let strength_factor = strength_skill / 20.0;
            let strength_distance = base_distance * strength_factor * 0.2;

            // Calculate the total maximum shooting distance
            base_distance + long_shots_distance + technique_distance + strength_distance
        }
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
