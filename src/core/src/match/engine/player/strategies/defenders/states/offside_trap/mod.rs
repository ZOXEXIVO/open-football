use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{ConditionContext, MatchPlayer, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;
use std::sync::LazyLock;
use rand::Rng;

static DEFENDER_OFFSIDE_TRAP_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_offside_trap_data.json")));

const OFFSIDE_TRAP_DISTANCE: f32 = 5.0; // Distance to move forward to set the trap
const OFFSIDE_TRAP_SPEED_MULTIPLIER: f32 = 1.2; // Speed multiplier when executing the trap

#[derive(Default)]
pub struct DefenderOffsideTrapState {}

impl StateProcessingHandler for DefenderOffsideTrapState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the team is defending a lead and there is limited time remaining
        let defending_lead = ctx.team().is_loosing() && ctx.context.time.is_running_out();

        // 2. Check if the opponent's playing style and formation are suitable for an offside trap
        let opponent_style_suitable = self.is_opponent_style_suitable(ctx);

        // 3. Evaluate the defensive line's cohesion and communication
        let defensive_line_cohesion = self.evaluate_defensive_line_cohesion(ctx);

        // 4. Consider the individual defender's attributes
        let defender_attributes_suitable = self.are_defender_attributes_suitable(ctx);

        if defending_lead && opponent_style_suitable && defensive_line_cohesion && defender_attributes_suitable {
            // Execute the offside trap
            let trap_success = self.attempt_offside_trap(ctx);

            if trap_success {
                // Offside trap is successful
                Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ))
            } else {
                // Offside trap failed; opponent may be through on goal
                Some(StateChangeResult::with_defender_state(
                    DefenderState::TrackingBack,
                ))
            }
        } else {
            // Conditions for setting up an offside trap are not met
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move forward smoothly to execute the offside trap
        let target_position = self.calculate_offside_trap_target_position(ctx);
        let current_position = ctx.player.position;
        let direction = (target_position - current_position).normalize();

        // Calculate the distance to the target position
        let distance_to_target = (target_position - current_position).magnitude();

        // Define a threshold distance for smooth deceleration
        let deceleration_threshold = 100.0;

        // Calculate the base speed based on the player's pace attribute
        let base_speed = ctx.player.skills.physical.pace;

        // Adjust the speed based on player attributes
        let agility_factor = ctx.player.skills.physical.agility as f32 / 100.0;
        let acceleration_factor = ctx.player.skills.physical.acceleration as f32 / 100.0;
        let stamina_factor = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;

        let adjusted_speed = base_speed * agility_factor * acceleration_factor * stamina_factor;

        // Calculate the target speed based on the distance to the target position
        let target_speed = if distance_to_target <= deceleration_threshold {
            // Smoothly decelerate as the player approaches the target position
            adjusted_speed * OFFSIDE_TRAP_SPEED_MULTIPLIER * (distance_to_target / deceleration_threshold)
        } else {
            // Move at the adjusted offside trap speed
            adjusted_speed * OFFSIDE_TRAP_SPEED_MULTIPLIER
        };

        // Define an acceleration factor and a blending factor
        let acceleration_factor = 0.1;
        let blending_factor = 0.8;

        // Calculate the target velocity
        let target_velocity = direction * target_speed;

        // Update the player's velocity based on the acceleration factor and blending factor
        let current_velocity = ctx.player.velocity;
        let new_velocity = current_velocity + (target_velocity - current_velocity) * acceleration_factor;
        let blended_velocity = current_velocity * (1.0 - blending_factor) + new_velocity * blending_factor;

        Some(blended_velocity)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl DefenderOffsideTrapState {
    fn is_opponent_style_suitable(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the opponent's playing style is suitable for an offside trap
        let opponent_team_id = if ctx.player.team_id == ctx.context.score.home_team.team_id {
            ctx.context.score.away_team.team_id
        } else {
            ctx.context.score.home_team.team_id
        };

        false

        // TODO
        // // Retrieve the opponent team's tactics (assuming you have a method to get the tactics by team ID)
        // let opponent_tactics = ctx.get_team_tactics(opponent_team_id);
        //
        // // Check if the opponent's tactics involve playing long balls or fast forwards
        // // Adjust the conditions based on your specific game mechanics and tactics representation
        // opponent_tactics.involves_long_balls() || opponent_tactics.has_fast_forwards()
    }

    fn evaluate_defensive_line_cohesion(&self, ctx: &StateProcessingContext) -> bool {
        // Evaluate the defensive line's cohesion and communication
        let defenders: Vec<&MatchPlayer> = ctx.players().teammates().defenders().filter_map(|defender| {
            ctx.context.players.by_id(defender.id)
        }).collect();

        // Calculate the average experience and communication attributes of the defenders
        let total_experience = defenders.iter().map(|p| p.player_attributes.potential_ability as u32).sum::<u32>();
        let total_communication = defenders.iter().map(|p| p.skills.mental.teamwork as u32).sum::<u32>();
        let avg_experience = total_experience as f32 / defenders.len() as f32;
        let avg_communication = total_communication as f32 / defenders.len() as f32;

        // Check if the average experience and communication exceed certain thresholds
        // Adjust the thresholds based on your specific game balance
        avg_experience >= 70.0 && avg_communication >= 75.0
    }

    fn are_defender_attributes_suitable(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the individual defender's attributes are suitable for executing an offside trap
        let positioning = ctx.player.skills.mental.positioning;
        let anticipation = ctx.player.skills.mental.anticipation;
        let speed = ctx.player.skills.physical.pace;

        // Check if the defender's attributes exceed certain thresholds
        // Adjust the thresholds based on your specific game balance
        positioning >= 15.0 && anticipation >= 15.0 && speed >= 70.0
    }

    fn attempt_offside_trap(&self, ctx: &StateProcessingContext) -> bool {
        // Get the positions of opponents and the defensive line
        let defensive_line_position = self.calculate_defensive_line_position(ctx);
        let opponent_positions: Vec<f32> = ctx
            .players()
            .opponents()
            .all()
            .map(|p| p.position.x)
            .collect();

        // Calculate the success probability based on teamwork and concentration
        let teamwork = ctx.player.skills.mental.teamwork as f32 / 20.0;
        let concentration = ctx.player.skills.mental.concentration as f32 / 20.0;
        let mut rng = rand::thread_rng();
        let success_probability = (teamwork + concentration) / 2.0;

        // Determine the offside trap outcome
        let offside_trap_successful = rng.gen::<f32>() < success_probability;

        if offside_trap_successful {
            // Check if any opponent is caught offside
            let caught_offside = opponent_positions.iter().any(|&x| {
                if ctx.player.side == Some(PlayerSide::Left) {
                    x > defensive_line_position
                } else {
                    x < defensive_line_position
                }
            });

            caught_offside
        } else {
            false
        }
    }

    fn calculate_defensive_line_position(&self, ctx: &StateProcessingContext) -> f32 {
        let defenders: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .defenders()
            .collect();

        let sum_x: f32 = defenders.iter().map(|p| p.position.x).sum();
        let avg_x = sum_x / defenders.len() as f32;

        // Adjust the defensive line position based on the team's tactics
        // You can modify this calculation based on your specific game mechanics
        let adjustment = 5.0; // Adjust this value as needed
        if ctx.player.side == Some(PlayerSide::Left) {
            avg_x + adjustment
        } else {
            avg_x - adjustment
        }
    }

    fn calculate_offside_trap_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let team_direction = if ctx.player.side.unwrap() == PlayerSide::Left {
            Vector3::new(1.0, 0.0, 0.0) // Left team moves forward in positive x-direction
        } else {
            Vector3::new(-1.0, 0.0, 0.0) // Right team moves forward in negative x-direction
        };

        // Calculate the defensive line position
        let defensive_line_position = self.calculate_defensive_line_position(ctx);

        // Calculate the target position for the offside trap
        let target_position = if ctx.player.side.unwrap() == PlayerSide::Left {
            Vector3::new(defensive_line_position + OFFSIDE_TRAP_DISTANCE, player_position.y, 0.0)
        } else {
            Vector3::new(defensive_line_position - OFFSIDE_TRAP_DISTANCE, player_position.y, 0.0)
        };

        // Calculate the direction from the player's current position to the target position
        let direction = (target_position - player_position).normalize();

        // Blend the team direction with the calculated direction
        let blended_direction = (team_direction + direction).normalize();

        blended_direction
    }

    fn calculate_offside_trap_target_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let defensive_line_position = self.calculate_defensive_line_position(ctx);

        // Calculate the target position for the offside trap
        if ctx.player.side.unwrap() == PlayerSide::Left {
            Vector3::new(defensive_line_position + OFFSIDE_TRAP_DISTANCE, player_position.y, 0.0)
        } else {
            Vector3::new(defensive_line_position - OFFSIDE_TRAP_DISTANCE, player_position.y, 0.0)
        }
    }
}
