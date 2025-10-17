use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::statistics::MatchStatisticType;
use crate::r#match::{GoalDetail, MatchContext, MatchField};
use log::debug;
use nalgebra::Vector3;

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
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let ball_pass_vector = event_model.pass_target - field.ball.position;
        let horizontal_distance = (ball_pass_vector.x * ball_pass_vector.x + ball_pass_vector.y * ball_pass_vector.y).sqrt();

        let pass_force = event_model.pass_force;
        let pass_force_multiplier = 4.0;

        // Calculate horizontal velocity
        let horizontal_direction = Vector3::new(ball_pass_vector.x, ball_pass_vector.y, 0.0).normalize();
        let horizontal_velocity = horizontal_direction * (pass_force * pass_force_multiplier);

        // Get player for skill-based variation
        let player = field.get_player_mut(event_model.from_player_id).unwrap();
        let technique_variation = (player.skills.technical.technique / 20.0).clamp(0.5, 1.5);

        // Much more random height variation - each pass is different
        let random_height_multiplier: f32 = rng.gen_range(0.3..1.8);
        let pass_style_random: f32 = rng.gen_range(0.0..1.0);

        // Calculate z-velocity with much more variety
        let base_z_velocity = if horizontal_distance > 50.0 {
            // Long pass - varied trajectories
            let gravity = 9.81;
            let time_to_target = horizontal_distance / horizontal_velocity.norm();
            let calculated_height = 0.5 * gravity * time_to_target / 3.0; // Divided by 3

            // Different pass styles for long passes
            if pass_style_random < 0.3 {
                // Low driven pass (30% chance)
                calculated_height * 0.4
            } else if pass_style_random < 0.7 {
                // Normal lofted pass (40% chance)
                calculated_height
            } else {
                // High lofted pass (30% chance)
                calculated_height * 1.5
            }
        } else if horizontal_distance > 20.0 {
            // Medium pass - lots of variation
            if pass_style_random < 0.5 {
                // Ground pass (50% chance)
                rng.gen_range(0.0..0.5) // Divided by 3
            } else if pass_style_random < 0.8 {
                // Low pass (30% chance)
                rng.gen_range(0.5..1.3) // Divided by 3
            } else {
                // Chip pass (20% chance)
                rng.gen_range(1.3..2.3) // Divided by 3
            }
        } else {
            // Short pass - mostly ground with occasional variety
            if pass_style_random < 0.75 {
                // Ground pass (75% chance)
                rng.gen_range(0.0..0.17) // Divided by 3
            } else if pass_style_random < 0.92 {
                // Small lift (17% chance)
                rng.gen_range(0.17..0.83) // Divided by 3
            } else {
                // Chip (8% chance)
                rng.gen_range(0.83..1.67) // Divided by 3
            }
        };

        let z_velocity = (base_z_velocity * random_height_multiplier * technique_variation).min(4.0);

        field.ball.velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            z_velocity
        );

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;

        field.ball.flags.in_flight_state = 10;
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
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let ball_shot_vector = shoot_event_model.target - field.ball.position;
        let horizontal_distance = (ball_shot_vector.x * ball_shot_vector.x + ball_shot_vector.y * ball_shot_vector.y).sqrt();

        // Calculate horizontal velocity
        let horizontal_direction = Vector3::new(ball_shot_vector.x, ball_shot_vector.y, 0.0).normalize();
        let horizontal_velocity = horizontal_direction * shoot_event_model.force as f32;

        // Get player for skill-based variation
        let player = field.get_player_mut(shoot_event_model.from_player_id).unwrap();
        let finishing_skill = (player.skills.technical.finishing / 20.0).clamp(0.6, 1.4);

        // Much more variety in shot heights
        let shot_style: f32 = rng.gen_range(0.0..1.0);
        let height_variation: f32 = rng.gen_range(0.5..1.5);

        // Calculate z-velocity - lower shots with variety
        let base_z_velocity = if horizontal_distance > 100.0 {
            // Long-range shot - varied heights
            if shot_style < 0.4 {
                rng.gen_range(0.7..1.3) // Low driven (40%)
            } else if shot_style < 0.8 {
                rng.gen_range(1.3..2.2) // Normal (40%)
            } else {
                rng.gen_range(2.2..3.0) // Rising shot (20%)
            }
        } else if horizontal_distance > 50.0 {
            // Medium-range shot - mostly low
            if shot_style < 0.6 {
                rng.gen_range(0.5..1.0) // Very low (60%)
            } else if shot_style < 0.9 {
                rng.gen_range(1.0..1.7) // Medium (30%)
            } else {
                rng.gen_range(1.7..2.3) // High (10%)
            }
        } else {
            // Close-range shot - very low and varied
            if shot_style < 0.7 {
                rng.gen_range(0.2..0.7) // Ground shot (70%)
            } else if shot_style < 0.95 {
                rng.gen_range(0.7..1.3) // Rising (25%)
            } else {
                rng.gen_range(1.3..2.0) // Chip (5%)
            }
        };

        let z_velocity = (base_z_velocity * height_variation * finishing_skill).min(3.0);

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
