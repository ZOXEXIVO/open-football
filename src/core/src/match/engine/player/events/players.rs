use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::statistics::MatchStatisticType;
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

        // Calculate horizontal velocity
        let horizontal_velocity = Self::calculate_horizontal_velocity(
            &ball_pass_vector,
            event_model.pass_force,
        );

        // Determine pass trajectory based on distance and skills
        let z_velocity = Self::calculate_pass_trajectory(
            horizontal_distance,
            &horizontal_velocity,
            &skills,
            &mut rng,
        );

        let max_z_velocity = Self::calculate_max_z_velocity(horizontal_distance, skills.vision);
        let final_z_velocity = z_velocity.min(max_z_velocity);

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

    fn calculate_very_long_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Very long cross-field pass - requires high vision
        let vision_bonus = skills.vision * 0.5;

        if pass_style_random < 0.10 * (1.0 - vision_bonus) {
            // Low driven long ball (very rare, only ~5% for high vision players)
            rng.random_range(0.5..1.2) * skills.technique
        } else if pass_style_random < 0.25 {
            // Medium height long ball (15% chance)
            let base_height = Self::calculate_lofted_trajectory(horizontal_distance, horizontal_velocity, 2.8);
            base_height * rng.random_range(0.8..1.1) * (0.7 + skills.technique * 0.3)
        } else if pass_style_random < 0.70 {
            // High lofted cross-field pass (45% chance)
            let base_height = Self::calculate_lofted_trajectory(horizontal_distance, horizontal_velocity, 1.8);
            base_height * rng.random_range(0.9..1.3) * (0.8 + skills.vision * 0.2)
        } else {
            // Very high switching pass (30% chance - spectacular passes)
            let base_height = Self::calculate_lofted_trajectory(horizontal_distance, horizontal_velocity, 1.4);
            base_height * rng.random_range(1.0..1.4) * (0.85 + skills.vision * 0.15)
        }
    }

    fn calculate_long_pass_trajectory(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Long pass - mix of driven and lofted passes
        if pass_style_random < 0.30 {
            // Driven ground pass (30% chance)
            0.0
        } else if pass_style_random < 0.50 {
            // Low driven pass with slight lift (20% chance)
            rng.random_range(0.05..0.3) * skills.passing
        } else if pass_style_random < 0.80 {
            // Normal lofted pass (30% chance)
            let base_height = Self::calculate_lofted_trajectory(horizontal_distance, horizontal_velocity, 4.0);
            base_height * rng.random_range(0.7..1.0) * (0.7 + skills.technique * 0.3)
        } else {
            // High lofted pass for switching play (20% chance)
            let base_height = Self::calculate_lofted_trajectory(horizontal_distance, horizontal_velocity, 2.8);
            base_height * rng.random_range(0.9..1.2) * (0.75 + skills.vision * 0.25)
        }
    }

    fn calculate_medium_pass_trajectory(
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Medium pass - mostly horizontal and low
        if pass_style_random < 0.50 {
            // Perfect ground pass (50% chance)
            0.0
        } else if pass_style_random < 0.78 {
            // Rolling ground pass with minimal height (28% chance)
            rng.random_range(0.0..0.12) * skills.technique
        } else if pass_style_random < 0.93 {
            // Low pass with small arc (15% chance)
            rng.random_range(0.2..0.5) * skills.passing
        } else {
            // Chip pass over defender (7% chance)
            rng.random_range(1.0..1.8) * skills.technique
        }
    }

    fn calculate_short_pass_trajectory(
        skills: &PassSkills,
        pass_style_random: f32,
        rng: &mut impl Rng,
    ) -> f32 {
        // Short pass - almost all ground passes
        if pass_style_random < 0.70 {
            // Pure horizontal ground pass (70% chance)
            0.0
        } else if pass_style_random < 0.92 {
            // Ground pass with tiny bounce (22% chance)
            rng.random_range(0.0..0.08) * skills.technique
        } else if pass_style_random < 0.97 {
            // Small lift pass (5% chance)
            rng.random_range(0.15..0.4) * skills.passing
        } else {
            // Delicate chip (3% chance)
            rng.random_range(0.8..1.3) * skills.technique
        }
    }

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
        if horizontal_distance > 100.0 {
            5.5 + (vision_skill * 2.5) // Up to 8.0 for high vision players
        } else if horizontal_distance > 60.0 {
            3.5 + (vision_skill * 0.8) // Up to 4.3
        } else {
            2.4 // Keep medium/short passes low
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

        // Calculate skill-based power multiplier (better players shoot harder)
        // Reduced multiplier range to keep speeds closer to original + 20%
        let power_skill_factor = (finishing_skill * 0.5) + (technique_skill * 0.3) + (long_shot_skill * 0.3);
        let power_multiplier = 0.97 + (power_skill_factor * 0.3); // Range: 0.95 to 1.25 (reduced from 0.7-1.3)

        // Calculate horizontal velocity with skill-based power
        let horizontal_direction = Vector3::new(ball_shot_vector.x, ball_shot_vector.y, 0.0).normalize();
        let base_horizontal_velocity = shoot_event_model.force as f32 * power_multiplier;

        // Add slight randomness to power (better players have more consistent power)
        let power_consistency = 0.98 + (technique_skill * 0.04); // 0.98 to 1.02 (reduced randomness)
        let power_random = rng.random_range(power_consistency - 0.02..power_consistency + 0.02);
        let horizontal_velocity = horizontal_direction * base_horizontal_velocity * power_random;

        // Calculate z-velocity based on shot style and player skills
        let shot_style: f32 = rng.random_range(0.0..1.0);
        let height_variation: f32 = rng.random_range(0.8..1.2);

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

        let z_velocity = (base_z_velocity * height_variation).min(3.0);

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
