use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{GoalDetail, MatchContext, MatchField, MatchPlayer};
use log::debug;
use nalgebra::Vector3;
use rand::Rng;

/// Helper struct to encapsulate player passing skills
struct PassSkills {
    passing: f32,
    technique: f32,
    vision: f32,
}

impl PassSkills {
    fn from_player(player: &MatchPlayer) -> Self {
        Self {
            passing: (player.skills.technical.passing / 20.0).clamp(0.4, 1.0),
            technique: (player.skills.technical.technique / 20.0).clamp(0.4, 1.0),
            vision: (player.skills.mental.vision / 20.0).clamp(0.3, 1.0),
        }
    }
}

/// Different trajectory styles for passes
#[derive(Debug, Clone, Copy)]
enum PassTrajectoryType {
    /// Ground pass - rolling along the surface
    Ground,
    /// Low driven pass - slight lift, fast and direct
    LowDriven,
    /// Parabolic loft - classic smooth arc, high peak
    ParabolicLoft,
    /// Chip/Scoop - quick rise with steep descent
    Chip,
    /// Simple loft - moderate height, less arc than parabolic
    SimpleLoft,
}

#[derive(Debug)]
pub enum PlayerEvent {
    Goal(u32, bool),
    Assist(u32),
    BallCollision(u32),
    TacklingBall(u32),
    BallOwnerChange(u32),
    PassTo(PassingEventContext),
    ClearBall(Vector3<f32>),
    RushOut(u32),
    Shoot(ShootingEventContext),
    MovePlayer(u32, Vector3<f32>),
    StayInGoal(u32),
    MoveBall(u32, Vector3<f32>),
    CommunicateMessage(u32, &'static str),
    OfferSupport(u32),
    ClaimBall(u32),
    GainBall(u32),
    CaughtBall(u32),
    CommitFoul,
    RequestHeading(u32, Vector3<f32>),
    RequestShot(u32, Vector3<f32>),
    RequestBallReceive(u32),
    TakeBall(u32),
}

pub struct PlayerEventDispatcher;

impl PlayerEventDispatcher {
    pub fn dispatch<'a>(
        event: PlayerEvent,
        field: &mut MatchField,
        context: &mut MatchContext,
    ) -> Vec<Event> {
        let remaining_events = Vec::new();

        debug!("Player event: {:?}", event);

        match event {
            PlayerEvent::Goal(player_id, is_auto_goal) => {
                Self::handle_goal_event(player_id, is_auto_goal, field, context);
            }
            PlayerEvent::Assist(player_id) => {
                Self::handle_assist_event(player_id, field, context);
            }
            PlayerEvent::BallCollision(player_id) => {
                Self::handle_ball_collision_event(player_id, field);
            }
            PlayerEvent::TacklingBall(player_id) => {
                Self::handle_tackling_ball_event(player_id, field);
            }
            PlayerEvent::BallOwnerChange(player_id) => {
                Self::handle_ball_owner_change_event(player_id, field);
            }
            PlayerEvent::PassTo(pass_event_model) => {
                Self::handle_pass_to_event(pass_event_model, field);
            }
            PlayerEvent::ClaimBall(player_id) => {
                Self::handle_claim_ball_event(player_id, field);
            }
            PlayerEvent::MoveBall(player_id, ball_velocity) => {
                Self::handle_move_ball_event(player_id, ball_velocity, field);
            }
            PlayerEvent::GainBall(player_id) => {
                Self::handle_gain_ball_event(player_id, field);
            }
            PlayerEvent::Shoot(shoot_event_model) => {
                Self::handle_shoot_event(shoot_event_model, field);
            }
            PlayerEvent::CaughtBall(player_id) => {
                Self::handle_caught_ball_event(player_id, field);
            }
            PlayerEvent::MovePlayer(player_id, position) => {
                Self::handle_move_player_event(player_id, position, field);
            }
            PlayerEvent::TakeBall(player_id) => {
                Self::handle_take_ball_event(player_id, field);
            }
            _ => {} // Ignore unsupported events
        }

        remaining_events
    }

    fn handle_goal_event(player_id: u32, is_auto_goal: bool, field: &mut MatchField, context: &mut MatchContext) {
        let player = field.get_player_mut(player_id).unwrap();

        player.statistics.add_goal(context.time.time, is_auto_goal);

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal,
            time: context.time.time,
        });

        field.ball.previous_owner = None;
        field.ball.current_owner = None;
    }

    fn handle_assist_event(player_id: u32, field: &mut MatchField, context: &mut MatchContext) {
        let player = field.get_player_mut(player_id).unwrap();

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Assist,
            time: context.time.time,
            is_auto_goal: false
        });

        player.statistics.add_assist(context.time.time);
    }

    fn handle_ball_collision_event(player_id: u32, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();

        if player.skills.technical.first_touch > 10.0 {
            // Handle player gaining control of the ball after collision
        }
    }

    fn handle_tackling_ball_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
    }

    fn handle_ball_owner_change_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
    }

    fn handle_pass_to_event(event_model: PassingEventContext, field: &mut MatchField) {
        let mut rng = rand::rng();

        // Calculate pass trajectory parameters
        let ball_pass_vector = event_model.pass_target - field.ball.position;
        let horizontal_distance = Self::calculate_horizontal_distance(&ball_pass_vector);

        // Extract player skills
        let player = field.get_player_mut(event_model.from_player_id).unwrap();
        let skills = PassSkills::from_player(player);

        // Add directional inaccuracy based on passing skill (worse players = more error)
        let accuracy_factor = skills.passing * skills.technique;
        let angle_error = rng.random_range(-0.15..0.15) * (1.2 - accuracy_factor); // Up to ±8.6° for poor passers

        // Rotate the pass vector slightly for error
        let cos_angle = angle_error.cos();
        let sin_angle = angle_error.sin();
        let adjusted_vector = Vector3::new(
            ball_pass_vector.x * cos_angle - ball_pass_vector.y * sin_angle,
            ball_pass_vector.x * sin_angle + ball_pass_vector.y * cos_angle,
            0.0,
        );

        // Add power variation based on technique (inconsistent power application)
        let power_consistency = 0.95 + (skills.technique * 0.1); // 0.95 to 1.05
        let power_variation = rng.random_range(power_consistency - 0.05..power_consistency + 0.05);
        let adjusted_force = event_model.pass_force * power_variation;

        // Calculate horizontal velocity with randomness applied
        let horizontal_velocity = Self::calculate_horizontal_velocity(
            &adjusted_vector,
            adjusted_force,
        );

        // Determine pass trajectory based on distance and skills
        let z_velocity = Self::calculate_pass_trajectory(
            horizontal_distance,
            &horizontal_velocity,
            &skills,
            &mut rng,
        );

        // Add slight vertical variation (spin, wind, grass conditions)
        let vertical_variation = rng.random_range(0.92..1.08);
        let adjusted_z_velocity = z_velocity * vertical_variation;

        let max_z_velocity = Self::calculate_max_z_velocity(horizontal_distance, skills.vision);
        let final_z_velocity = adjusted_z_velocity.min(max_z_velocity);

        // Apply ball physics
        field.ball.velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            final_z_velocity,
        );

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.flags.in_flight_state = 10;
    }

    fn calculate_horizontal_distance(ball_pass_vector: &Vector3<f32>) -> f32 {
        (ball_pass_vector.x * ball_pass_vector.x + ball_pass_vector.y * ball_pass_vector.y).sqrt()
    }

    fn calculate_horizontal_velocity(
        ball_pass_vector: &Vector3<f32>,
        pass_force: f32,
    ) -> Vector3<f32> {
        const PASS_FORCE_MULTIPLIER: f32 = 4.0;
        let horizontal_direction = Vector3::new(ball_pass_vector.x, ball_pass_vector.y, 0.0).normalize();
        horizontal_direction * (pass_force * PASS_FORCE_MULTIPLIER)
    }

    fn calculate_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        let pass_style_random: f32 = rng.random_range(0.0..1.0);

        match horizontal_distance {
            d if d > 300.0 => Self::calculate_extreme_long_pass_trajectory(
                horizontal_distance,
                horizontal_velocity,
                skills,
                pass_style_random,
                rng,
            ),
            d if d > 200.0 => Self::calculate_ultra_long_pass_trajectory(
                horizontal_distance,
                horizontal_velocity,
                skills,
                pass_style_random,
                rng,
            ),
            d if d > 100.0 => Self::calculate_very_long_pass_trajectory(
                horizontal_distance,
                horizontal_velocity,
                skills,
                pass_style_random,
                rng,
            ),
            d if d > 60.0 => Self::calculate_long_pass_trajectory(
                horizontal_distance,
                horizontal_velocity,
                skills,
                pass_style_random,
                rng,
            ),
            d if d > 25.0 => Self::calculate_medium_pass_trajectory(
                horizontal_distance,
                skills,
                pass_style_random,
                rng,
            ),
            _ => Self::calculate_short_pass_trajectory(
                skills,
                pass_style_random,
                rng,
            ),
        }
    }

    fn calculate_extreme_long_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Extreme distance pass (300m+) - goalkeeper kicks, clearances
        // MUST be very high - almost always aerial

        if pass_style_random < 0.03 {
            // Ultra-low missile (3% chance - extremely rare)
            Self::calculate_low_driven_trajectory(horizontal_distance, skills, rng) * 3.0
        } else if pass_style_random < 0.20 {
            // High simple loft (17% chance)
            Self::calculate_simple_loft_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.8
        } else if pass_style_random < 0.60 {
            // High parabolic clearance (40% chance - most common)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.5
        } else {
            // Extreme parabolic (40% chance - maximum height/distance)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.8
        }
    }

    fn calculate_ultra_long_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Ultra-long pass (200-300m) - goal kicks, long diagonals
        // Must be high to reach

        if pass_style_random < 0.05 {
            // Driven missile (5% chance - exceptional technique required)
            Self::calculate_low_driven_trajectory(horizontal_distance, skills, rng) * 2.5
        } else if pass_style_random < 0.25 {
            // Simple high loft (20% chance)
            Self::calculate_simple_loft_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.5
        } else if pass_style_random < 0.65 {
            // Parabolic arc - standard for distance (40% chance)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.15
        } else {
            // Very high parabolic (35% chance - maximum elevation)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.45
        }
    }

    fn calculate_very_long_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Very long cross-field pass (100-200m) - needs significant height
        let vision_bonus = skills.vision * 0.5;

        if pass_style_random < 0.08 * (1.0 - vision_bonus) {
            // Low driven long ball (very rare, ~4% for high vision)
            Self::calculate_low_driven_trajectory(horizontal_distance, skills, rng) * 1.5
        } else if pass_style_random < 0.30 {
            // Simple loft - moderate arc (22% chance)
            Self::calculate_simple_loft_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.2
        } else if pass_style_random < 0.70 {
            // Parabolic loft - smooth high arc (40% chance)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 0.95
        } else {
            // High parabolic - spectacular switching pass (30% chance)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.25
        }
    }

    fn calculate_long_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Long pass - varied mix of driven, lofted, and parabolic passes
        if pass_style_random < 0.25 {
            // Driven ground pass (25% chance)
            Self::calculate_ground_trajectory(skills, rng)
        } else if pass_style_random < 0.45 {
            // Low driven pass (20% chance)
            Self::calculate_low_driven_trajectory(horizontal_distance, skills, rng)
        } else if pass_style_random < 0.65 {
            // Simple loft (20% chance)
            Self::calculate_simple_loft_trajectory(horizontal_distance, horizontal_velocity, skills, rng)
        } else if pass_style_random < 0.85 {
            // Parabolic loft - classic high arc (20% chance)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 0.7
        } else {
            // High parabolic for switching play (15% chance)
            Self::calculate_parabolic_trajectory(horizontal_distance, horizontal_velocity, skills, rng) * 1.1
        }
    }

    fn calculate_medium_pass_trajectory(
        horizontal_distance: f32,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Medium pass - mix of ground and low passes
        if pass_style_random < 0.55 {
            // Pure ground pass (55% chance)
            Self::calculate_ground_trajectory(skills, rng)
        } else if pass_style_random < 0.80 {
            // Low driven pass (25% chance)
            Self::calculate_low_driven_trajectory(horizontal_distance, skills, rng) * 0.7
        } else if pass_style_random < 0.95 {
            // Simple loft (15% chance)
            rng.random_range(0.3..0.7) * skills.passing
        } else {
            // Chip over defender (5% chance)
            Self::calculate_chip_trajectory(horizontal_distance, skills, rng) * 0.8
        }
    }

    fn calculate_short_pass_trajectory(
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Short pass - almost all ground passes with occasional chips
        if pass_style_random < 0.75 {
            // Pure ground pass (75% chance)
            Self::calculate_ground_trajectory(skills, rng)
        } else if pass_style_random < 0.95 {
            // Low driven with tiny lift (20% chance)
            rng.random_range(0.05..0.15) * skills.technique
        } else {
            // Delicate chip over defender (5% chance)
            let horizontal_distance = 15.0; // Approximate short distance
            Self::calculate_chip_trajectory(horizontal_distance, skills, rng) * 0.6
        }
    }

    /// Calculate z-velocity for ground passes (minimal to no lift)
    fn calculate_ground_trajectory(skills: &PassSkills, rng: &mut impl Rng) -> f32 {
        // Pure ground pass with very minimal variance
        rng.random_range(0.0..0.05) * skills.technique
    }

    /// Calculate z-velocity for low driven passes (slight lift, fast)
    fn calculate_low_driven_trajectory(
        horizontal_distance: f32,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        // Low trajectory with minimal lift - stays close to ground
        let base_height = (horizontal_distance / 100.0).clamp(0.2, 1.5);
        base_height * rng.random_range(0.8..1.2) * (0.6 + skills.technique * 0.4)
    }

    /// Calculate z-velocity for classic parabolic lofted passes (smooth high arc)
    fn calculate_parabolic_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        const GRAVITY: f32 = 9.81;

        // Classic physics-based parabolic arc
        let time_to_target = horizontal_distance / horizontal_velocity.norm();

        // Calculate initial z-velocity for a smooth parabolic arc
        // The peak height occurs at half the flight time
        let base_z_velocity = 0.5 * GRAVITY * time_to_target;

        // Add skill-based variation for arc height
        let arc_multiplier = rng.random_range(0.85..1.25) * (0.8 + skills.vision * 0.2);

        base_z_velocity * arc_multiplier
    }

    /// Calculate z-velocity for chip/scoop passes (quick rise, steep descent)
    fn calculate_chip_trajectory(
        horizontal_distance: f32,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        // Chip has a steeper initial velocity for quick rise
        // Height is less dependent on distance, more on technique
        let base_height = (horizontal_distance / 30.0).clamp(1.5, 4.5);

        // Chips require good technique - higher multiplier for skilled players
        let technique_factor = 0.7 + skills.technique * 0.6;

        base_height * rng.random_range(1.0..1.4) * technique_factor
    }

    /// Calculate z-velocity for simple loft passes (moderate arc, less than parabolic)
    fn calculate_simple_loft_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        const GRAVITY: f32 = 9.81;

        // Similar to parabolic but with lower arc
        let time_to_target = horizontal_distance / horizontal_velocity.norm();
        let base_z_velocity = 0.35 * GRAVITY * time_to_target;

        // Less variation than parabolic, more consistent
        let arc_multiplier = rng.random_range(0.9..1.15) * (0.75 + skills.passing * 0.25);

        base_z_velocity * arc_multiplier
    }

    /// Helper function for legacy compatibility - delegates to parabolic
    fn calculate_lofted_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        time_divisor: f32,
    ) -> f32 {
        const GRAVITY: f32 = 9.81;
        let time_to_target = horizontal_distance / horizontal_velocity.norm();
        0.5 * GRAVITY * time_to_target / time_divisor
    }

    fn calculate_max_z_velocity(horizontal_distance: f32, vision_skill: f32) -> f32 {
        if horizontal_distance > 300.0 {
            // Extreme distance - goalkeeper goal kicks, desperate clearances
            12.0 + (vision_skill * 3.0) // Up to 15.0 m/s for extreme passes
        } else if horizontal_distance > 200.0 {
            // Ultra-long diagonal switches
            9.0 + (vision_skill * 2.5) // Up to 11.5 m/s
        } else if horizontal_distance > 100.0 {
            // Very long cross-field passes
            5.5 + (vision_skill * 2.5) // Up to 8.0 m/s for high vision players
        } else if horizontal_distance > 60.0 {
            // Long passes
            3.5 + (vision_skill * 0.8) // Up to 4.3 m/s
        } else {
            // Medium/short passes - keep low
            2.4
        }
    }

    fn handle_claim_ball_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);

        field.ball.flags.in_flight_state = 30;
    }

    fn handle_move_ball_event(player_id: u32, ball_velocity: Vector3<f32>, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);

        field.ball.velocity = ball_velocity;
    }

    fn handle_gain_ball_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);

        field.ball.flags.in_flight_state = 100;
    }

    fn handle_shoot_event(shoot_event_model: ShootingEventContext, field: &mut MatchField) {
        let mut rng = rand::rng();

        let ball_shot_vector = shoot_event_model.target - field.ball.position;
        let horizontal_distance = (ball_shot_vector.x * ball_shot_vector.x + ball_shot_vector.y * ball_shot_vector.y).sqrt();

        // Get player skills for power and accuracy calculations
        let player = field.get_player_mut(shoot_event_model.from_player_id).unwrap();

        let finishing_skill = (player.skills.technical.finishing / 20.0).clamp(0.5, 1.0);
        let technique_skill = (player.skills.technical.technique / 20.0).clamp(0.5, 1.0);
        let long_shot_skill = (player.skills.technical.long_shots / 20.0).clamp(0.5, 1.0);
        let composure_skill = (player.skills.mental.composure / 20.0).clamp(0.4, 1.0);

        // Add directional inaccuracy (shooting error)
        // Better finishers have less angular error
        let accuracy_factor = finishing_skill * composure_skill;
        let max_angle_error = if horizontal_distance > 50.0 {
            0.25 * (1.5 - accuracy_factor) // Long shots: up to ±14.3° for poor shooters
        } else {
            0.15 * (1.3 - accuracy_factor) // Close shots: up to ±8.6° for poor shooters
        };

        let angle_error = rng.random_range(-max_angle_error..max_angle_error);

        // Rotate shot vector for directional error
        let cos_angle = angle_error.cos();
        let sin_angle = angle_error.sin();
        let adjusted_shot_vector = Vector3::new(
            ball_shot_vector.x * cos_angle - ball_shot_vector.y * sin_angle,
            ball_shot_vector.x * sin_angle + ball_shot_vector.y * cos_angle,
            0.0,
        );

        // Calculate skill-based power multiplier (better players shoot harder)
        let power_skill_factor = (finishing_skill * 0.5) + (technique_skill * 0.3) + (long_shot_skill * 0.3);
        let power_multiplier = 0.97 + (power_skill_factor * 0.3); // Range: 0.95 to 1.25

        // Calculate horizontal velocity with skill-based power
        let horizontal_direction = Vector3::new(adjusted_shot_vector.x, adjusted_shot_vector.y, 0.0).normalize();
        let base_horizontal_velocity = shoot_event_model.force as f32 * power_multiplier;

        // Add power randomness (better players have more consistent power)
        let power_consistency = 0.96 + (technique_skill * 0.08); // 0.96 to 1.04
        let power_random = rng.random_range(power_consistency - 0.04..power_consistency + 0.04);
        let horizontal_velocity = horizontal_direction * base_horizontal_velocity * power_random;

        // Calculate z-velocity based on shot style and player skills
        let shot_style: f32 = rng.random_range(0.0..1.0);
        let height_variation: f32 = rng.random_range(0.85..1.15);

        let base_z_velocity = if horizontal_distance > 100.0 {
            // Long-range shot - varied heights (technique matters more)
            if shot_style < 0.4 {
                rng.random_range(0.7..1.3) * technique_skill // Low driven (40%)
            } else if shot_style < 0.8 {
                rng.random_range(1.3..2.2) * technique_skill // Normal (40%)
            } else {
                rng.random_range(2.2..3.0) * long_shot_skill // Rising shot (20%)
            }
        } else if horizontal_distance > 50.0 {
            // Medium-range shot - mostly low (finishing matters more)
            if shot_style < 0.6 {
                rng.random_range(0.5..1.0) * finishing_skill // Very low (60%)
            } else if shot_style < 0.9 {
                rng.random_range(1.0..1.7) * technique_skill // Medium (30%)
            } else {
                rng.random_range(1.7..2.3) * technique_skill // High (10%)
            }
        } else {
            // Close-range shot - very low and varied (finishing is key)
            if shot_style < 0.7 {
                rng.random_range(0.2..0.7) * finishing_skill // Ground shot (70%)
            } else if shot_style < 0.95 {
                rng.random_range(0.7..1.3) * finishing_skill // Rising (25%)
            } else {
                rng.random_range(1.3..2.0) * technique_skill // Chip (5%)
            }
        };

        // Add spin/environmental variation to height
        let vertical_spin_variation = rng.random_range(0.94..1.06);
        let z_velocity = (base_z_velocity * height_variation * vertical_spin_variation).min(3.0);

        field.ball.previous_owner = Some(shoot_event_model.from_player_id);
        field.ball.current_owner = None;
        field.ball.velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            z_velocity
        );

        field.ball.flags.in_flight_state = 100;
    }

    fn handle_caught_ball_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        
    }

    fn handle_move_player_event(player_id: u32, position: Vector3<f32>, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();
        player.position = position;
    }

    fn handle_take_ball_event(player_id: u32, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();
        player.run_for_ball();
    }
}
