use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{GoalDetail, MatchContext, MatchField, MatchPlayer};
use log::debug;
use nalgebra::Vector3;
use rand::Rng;

/// Helper struct to encapsulate player passing skills and condition
struct PassSkills {
    passing: f32,
    technique: f32,
    vision: f32,
    composure: f32,
    decisions: f32,
    concentration: f32,
    flair: f32,
    long_shots: f32,
    crossing: f32,
    stamina: f32,
    match_readiness: f32,
    condition_factor: f32,
}

impl PassSkills {
    fn from_player(player: &MatchPlayer) -> Self {
        // Normalize skills to 0.0-1.0 range
        let passing = (player.skills.technical.passing / 20.0).clamp(0.4, 1.0);
        let technique = (player.skills.technical.technique / 20.0).clamp(0.4, 1.0);
        let vision = (player.skills.mental.vision / 20.0).clamp(0.3, 1.0);
        let composure = (player.skills.mental.composure / 20.0).clamp(0.3, 1.0);
        let decisions = (player.skills.mental.decisions / 20.0).clamp(0.3, 1.0);
        let concentration = (player.skills.mental.concentration / 20.0).clamp(0.3, 1.0);
        let flair = (player.skills.mental.flair / 20.0).clamp(0.0, 1.0);
        let long_shots = (player.skills.technical.long_shots / 20.0).clamp(0.3, 1.0);
        let crossing = (player.skills.technical.crossing / 20.0).clamp(0.3, 1.0);
        let stamina = (player.skills.physical.stamina / 20.0).clamp(0.3, 1.0);
        let match_readiness = (player.skills.physical.match_readiness / 20.0).clamp(0.3, 1.0);

        // Calculate condition factor (0.5 to 1.0 based on player condition)
        let condition_percentage = player.player_attributes.condition as f32 / 10000.0;
        let fitness_factor = (player.player_attributes.fitness as f32 / 10000.0).clamp(0.5, 1.0);
        let jadedness_penalty = (player.player_attributes.jadedness as f32 / 10000.0) * 0.3;

        let condition_factor = (condition_percentage * fitness_factor - jadedness_penalty).clamp(0.5, 1.0);

        Self {
            passing,
            technique,
            vision,
            composure,
            decisions,
            concentration,
            flair,
            long_shots,
            crossing,
            stamina,
            match_readiness,
            condition_factor,
        }
    }

    /// Calculate overall passing quality (affected by condition)
    fn overall_quality(&self) -> f32 {
        let base_quality = self.passing * 0.4 + self.technique * 0.3 + self.vision * 0.3;
        base_quality * self.condition_factor * self.match_readiness
    }

    /// Calculate decision-making quality for trajectory selection
    fn decision_quality(&self) -> f32 {
        (self.decisions * 0.4 + self.vision * 0.3 + self.concentration * 0.2 + self.composure * 0.1)
            * self.condition_factor
    }
}

/// Different trajectory styles for passes
/// Each type represents a different flight time and arc height to reach the same target
#[derive(Debug, Clone, Copy)]
enum TrajectoryType {
    /// Ground pass - minimal flight time, almost zero arc (fastest)
    Ground,
    /// Low driven pass - short flight time, low arc (fast and direct)
    LowDriven,
    /// Medium arc - moderate flight time, balanced trajectory
    MediumArc,
    /// High arc - longer flight time, high parabolic arc (for distance/obstacles)
    HighArc,
    /// Chip - very high arc over short distance (for beating defenders)
    Chip,
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
    pub fn dispatch(
        event: PlayerEvent,
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut crate::r#match::ResultMatchPositionData,
    ) -> Vec<Event> {
        let remaining_events = Vec::new();

        if context.logging_enabled {
            debug!("Player event: {:?}", event);
        }       

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
                // Record the pass event (only if tracking is enabled)
                if match_data.is_tracking_events() {
                    match_data.add_pass_event(
                        context.total_match_time,
                        pass_event_model.from_player_id,
                        pass_event_model.to_player_id,
                    );
                }
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
            PlayerEvent::ClearBall(velocity) => {
                Self::handle_clear_ball_event(velocity, field);
            }
            _ => {} // Ignore unsupported events
        }

        remaining_events
    }

    fn handle_goal_event(player_id: u32, is_auto_goal: bool, field: &mut MatchField, context: &mut MatchContext) {
        let player = field.get_player_mut(player_id).unwrap();

        player.statistics.add_goal(context.total_match_time, is_auto_goal);

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal,
            time: context.total_match_time,
        });

        field.ball.previous_owner = None;
        field.ball.current_owner = None;
    }

    fn handle_assist_event(player_id: u32, field: &mut MatchField, context: &mut MatchContext) {
        let player = field.get_player_mut(player_id).unwrap();

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Assist,
            time: context.total_match_time,
            is_auto_goal: false
        });

        player.statistics.add_assist(context.total_match_time);
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

        // Extract player skills and condition
        let player = field.get_player(event_model.from_player_id).unwrap();
        let passer_position = player.position;
        let skills = PassSkills::from_player(player);

        // Calculate overall quality for accuracy - affected by condition
        let overall_quality = skills.overall_quality();

        // Calculate ideal target position
        let ideal_target = event_model.pass_target;

        // Use passer's position as starting point if ball is very close (within 5m)
        // This handles cases where ball ownership just changed
        let ball_position = field.ball.position;
        let ideal_pass_vector = ideal_target - ball_position;
        let horizontal_distance = Self::calculate_horizontal_distance(&ideal_pass_vector);

        // Apply skill-based targeting error
        // Better players are more accurate with their intended target
        let accuracy_factor = overall_quality * skills.concentration;

        // Distance-based error: longer passes have more positional error
        // Realistic values: professional players accurate to ~0.5-2m depending on distance
        let distance_error_factor = (horizontal_distance / 200.0).min(1.5);
        let max_position_error = 1.2 * (1.0 - accuracy_factor) * distance_error_factor;

        // Add random targeting error
        let target_error_x = rng.random_range(-max_position_error..max_position_error);
        let target_error_y = rng.random_range(-max_position_error..max_position_error);

        // Calculate actual target with error
        let actual_target = Vector3::new(
            ideal_target.x + target_error_x,
            ideal_target.y + target_error_y,
            0.0,
        );

        let actual_pass_vector = actual_target - ball_position;
        let actual_horizontal_distance = Self::calculate_horizontal_distance(&actual_pass_vector);

        // Calculate pass force with power variation
        // Reduced variation to make passes more consistent
        let power_consistency = 0.9 + (skills.technique * skills.stamina * 0.07);
        let power_variation_range = (1.0 - overall_quality) * 0.08;
        let power_variation = rng.random_range(
            power_consistency - power_variation_range..power_consistency + power_variation_range
        );
        let adjusted_force = event_model.pass_force * power_variation;

        // Calculate horizontal velocity to reach target
        let horizontal_velocity = Self::calculate_horizontal_velocity(
            &actual_pass_vector,
            adjusted_force,
        );

        // Determine trajectory type based on context, not just distance
        let passer = field.get_player_mut(event_model.from_player_id).unwrap();
        let passer_team_id = passer.team_id;

        let trajectory_type = Self::select_trajectory_type_contextual(
            actual_horizontal_distance,
            &skills,
            &mut rng,
            &passer_position,
            &actual_target,
            passer_team_id,
            &field.players,
        );

        // Calculate z-velocity to reach target with chosen trajectory type
        let z_velocity = Self::calculate_trajectory_to_target(
            actual_horizontal_distance,
            &horizontal_velocity,
            trajectory_type,
            &skills,
            &mut rng,
        );

        let max_z_velocity = Self::calculate_max_z_velocity(actual_horizontal_distance, &skills);
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
        const PASS_FORCE_MULTIPLIER: f32 = 4.5;
        let horizontal_direction = Vector3::new(ball_pass_vector.x, ball_pass_vector.y, 0.0).normalize();
        horizontal_direction * (pass_force * PASS_FORCE_MULTIPLIER)
    }

    /// Select trajectory type based on game context (obstacles, pressure, tactical situation)
    /// In real football, trajectory depends on what's between you and the target!
    fn select_trajectory_type_contextual(
        horizontal_distance: f32,
        skills: &PassSkills,
        rng: &mut impl Rng,
        from_position: &Vector3<f32>,
        to_position: &Vector3<f32>,
        passer_team_id: u32,
        players: &[MatchPlayer],
    ) -> TrajectoryType {
        // Check for obstacles in the passing lane
        let obstacles_in_lane = Self::count_obstacles_in_passing_lane(
            from_position,
            to_position,
            passer_team_id,
            players,
        );

        // Calculate decision quality - determines how well player chooses trajectory
        let decision_quality = skills.decision_quality();
        let vision_quality = skills.vision;

        // Better decision makers make more appropriate choices
        let skill_influenced_random = {
            let pure_random = rng.random_range(0.0..1.0);
            let randomness_factor = 1.0 - (decision_quality * 0.6);
            let skill_bias = decision_quality * 0.5;
            (pure_random * randomness_factor + skill_bias).clamp(0.0, 1.0)
        };

        // Context factors
        let has_obstacles = obstacles_in_lane > 0;
        let many_obstacles = obstacles_in_lane >= 2;
        let clear_lane = obstacles_in_lane == 0;

        // Distance categories (adjusted for more realistic football passing)
        let is_short = horizontal_distance <= 25.0;
        let is_medium = horizontal_distance > 25.0 && horizontal_distance <= 55.0;
        let is_long = horizontal_distance > 55.0 && horizontal_distance <= 90.0;
        let is_very_long = horizontal_distance > 90.0;

        // SHORT PASSES (0-25m) - context matters most
        if is_short {
            if clear_lane {
                // Clear lane - almost always ground pass
                if skill_influenced_random < 0.95 {
                    TrajectoryType::Ground
                } else if skills.flair * skills.technique > 0.75 {
                    TrajectoryType::Chip // Rare skillful chip
                } else {
                    TrajectoryType::Ground
                }
            } else if has_obstacles {
                // Obstacles nearby - need to lift it slightly or chip
                if skill_influenced_random < 0.6 {
                    TrajectoryType::Ground // Try to thread through
                } else if vision_quality > 0.7 && skill_influenced_random < 0.85 {
                    TrajectoryType::Chip // Smart chip over defender
                } else {
                    TrajectoryType::LowDriven // Lift it slightly
                }
            } else {
                TrajectoryType::Ground
            }
        }
        // MEDIUM PASSES (25-55m) - balance between ground and aerial
        else if is_medium {
            if clear_lane {
                // Clear lane - strongly prefer ground/driven passes
                if skill_influenced_random < 0.75 {
                    TrajectoryType::Ground
                } else if skill_influenced_random < 0.95 {
                    TrajectoryType::LowDriven
                } else {
                    TrajectoryType::MediumArc  // Occasional variation
                }
            } else if many_obstacles {
                // Multiple obstacles - need to go over them
                if skill_influenced_random < 0.4 {
                    TrajectoryType::LowDriven
                } else if skill_influenced_random < 0.75 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            } else if has_obstacles {
                // One obstacle - can drive it or loft it
                if skill_influenced_random < 0.5 {
                    TrajectoryType::LowDriven
                } else if skill_influenced_random < 0.8 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::Ground // Try to thread through
                }
            } else {
                // Default - mostly ground/driven
                if skill_influenced_random < 0.7 {
                    TrajectoryType::Ground
                } else {
                    TrajectoryType::LowDriven
                }
            }
        }
        // LONG PASSES (55-90m) - prefer ground/driven when possible
        else if is_long {
            if clear_lane {
                // Clear lane - heavily favor ground/driven passes
                if skills.technique > 0.7 {
                    // Good technique - can execute long driven passes
                    if skill_influenced_random < 0.60 {
                        TrajectoryType::LowDriven
                    } else if skill_influenced_random < 0.80 {
                        TrajectoryType::MediumArc
                    } else {
                        TrajectoryType::HighArc
                    }
                } else {
                    // Average technique - still prefer driven but mix more
                    if skill_influenced_random < 0.45 {
                        TrajectoryType::LowDriven
                    } else if skill_influenced_random < 0.70 {
                        TrajectoryType::MediumArc
                    } else {
                        TrajectoryType::HighArc
                    }
                }
            } else if many_obstacles {
                // Many obstacles - must go aerial
                if skill_influenced_random < 0.30 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            } else {
                // One obstacle - prefer driven but can go aerial
                if skill_influenced_random < 0.40 {
                    TrajectoryType::LowDriven
                } else if skill_influenced_random < 0.70 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            }
        }
        // VERY LONG PASSES (90m+) - usually aerial, but elite players can drive it
        else if is_very_long {
            let long_pass_ability = skills.long_shots * skills.vision * skills.crossing;

            if long_pass_ability > 0.75 && clear_lane {
                // Elite long passer with clear lane - can try powerful driven pass
                if skill_influenced_random < 0.40 {
                    TrajectoryType::LowDriven // Powerful driven pass
                } else if skill_influenced_random < 0.70 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            } else if skill_influenced_random < 0.35 {
                TrajectoryType::MediumArc
            } else {
                TrajectoryType::HighArc
            }
        }
        // EXTREME DISTANCES (fallback)
        else {
            if skill_influenced_random < 0.4 {
                TrajectoryType::MediumArc
            } else {
                TrajectoryType::HighArc
            }
        }
    }

    /// Count how many opponent players are in the passing lane (obstacles)
    fn count_obstacles_in_passing_lane(
        from_position: &Vector3<f32>,
        to_position: &Vector3<f32>,
        passer_team_id: u32,
        players: &[MatchPlayer],
    ) -> usize {
        const LANE_WIDTH: f32 = 8.0; // Width of the passing lane corridor (wider for realistic obstacle detection)

        let pass_direction = (*to_position - *from_position).normalize();
        let pass_distance = (*to_position - *from_position).magnitude();

        players
            .iter()
            .filter(|player| {
                // Only count opponents
                if player.team_id == passer_team_id {
                    return false;
                }

                // Vector from passer to player
                let to_player = player.position - *from_position;

                // Project player onto pass line to find closest point
                let projection_length = to_player.dot(&pass_direction);

                // Player must be between passer and target
                if projection_length < 0.0 || projection_length > pass_distance {
                    return false;
                }

                // Calculate perpendicular distance to pass line
                let projection_point = *from_position + pass_direction * projection_length;
                let perpendicular_distance = (player.position - projection_point).magnitude();

                // Player is an obstacle if within lane width
                perpendicular_distance < LANE_WIDTH
            })
            .count()
    }
    
    /// Calculate z-velocity to reach target with chosen trajectory type
    /// Ground passes stay on the ground, aerial passes use physics
    fn calculate_trajectory_to_target(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        trajectory_type: TrajectoryType,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        const GRAVITY: f32 = 9.81;

        let horizontal_speed = horizontal_velocity.norm();
        if horizontal_speed < 0.1 {
            return 0.0; // Avoid division by zero
        }

        // Add very small random variation to all trajectories for realism
        let tiny_random = rng.random_range(0.98..1.02);

        match trajectory_type {
            // Ground pass - truly on the ground (rolling)
            TrajectoryType::Ground => {
                // Almost no lift - just enough to handle slight bumps
                // This keeps the ball rolling along the ground
                let base_lift = 0.02 * skills.technique;
                let random_variation = rng.random_range(0.0..0.1);
                base_lift * random_variation * tiny_random // 0.0 to ~0.002 m/s (truly ground)
            }

            // Low driven - stays very close to ground, minimal arc
            TrajectoryType::LowDriven => {
                // Slight lift for speed, but stays low (max height ~0.5-1m)
                let distance_factor = (horizontal_distance / 100.0).clamp(0.3, 1.0);
                let skill_factor = skills.technique * skills.condition_factor;

                let base_z = 0.5 + (distance_factor * 0.8); // 0.5 to 1.3 m/s
                let variation = rng.random_range(0.85..1.15);

                base_z * skill_factor * variation * tiny_random
            }

            // Medium arc - moderate parabolic trajectory (height ~2-4m)
            TrajectoryType::MediumArc => {
                let base_flight_time = horizontal_distance / horizontal_speed;
                let flight_time = base_flight_time * 0.5; // Moderate arc

                let ideal_z = 0.5 * GRAVITY * flight_time;

                // Skill affects consistency
                let execution_quality = skills.overall_quality();
                let error_range = (1.0 - execution_quality) * 0.12;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                ideal_z * error * tiny_random
            }

            // High arc - high parabolic trajectory (height ~4-8m)
            TrajectoryType::HighArc => {
                let base_flight_time = horizontal_distance / horizontal_speed;
                let flight_time = base_flight_time * 1.3; // High arc

                let ideal_z = 0.5 * GRAVITY * flight_time;

                // Requires good long passing ability
                let execution_quality = (skills.overall_quality() + skills.long_shots + skills.crossing) / 3.0;
                let error_range = (1.0 - execution_quality) * 0.18;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                ideal_z * error * tiny_random
            }

            // Chip - very high arc over short distance (height ~3-6m)
            TrajectoryType::Chip => {
                // Chips are based on technique, not distance
                let chip_ability = (skills.technique * 0.5 + skills.flair * 0.3 + skills.passing * 0.2)
                    * skills.condition_factor;

                // Base height for chip regardless of distance
                let base_chip_height = 2.5 + (chip_ability * 2.0); // 2.5 to 4.5 m/s

                // Execution error for this difficult skill
                let error_range = (1.0 - chip_ability) * 0.25;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                base_chip_height * error * tiny_random
            }
        }
    }

    fn calculate_max_z_velocity(horizontal_distance: f32, skills: &PassSkills) -> f32 {
        // Combine vision and long_shots for long pass capability
        let long_pass_ability = (skills.vision * 0.6 + skills.long_shots * 0.4) * skills.condition_factor;

        // Much stricter limits - most passes should stay low
        if horizontal_distance <= 20.0 {
            // Short passes - almost no lift allowed (ground passes)
            0.3 // Maximum 0.3 m/s vertical
        } else if horizontal_distance <= 45.0 {
            // Medium passes - keep very low
            1.0 + (long_pass_ability * 0.3) // 1.0 to 1.3 m/s
        } else if horizontal_distance <= 80.0 {
            // Long passes - moderate lift allowed
            2.5 + (long_pass_ability * 1.5) // 2.5 to 4.0 m/s
        } else if horizontal_distance <= 150.0 {
            // Very long passes - significant lift needed
            4.5 + (long_pass_ability * 2.5) // 4.5 to 7.0 m/s
        } else if horizontal_distance <= 250.0 {
            // Ultra-long diagonal switches
            7.0 + (long_pass_ability * 3.0) // 7.0 to 10.0 m/s
        } else {
            // Extreme distance - goalkeeper goal kicks, clearances
            10.0 + (long_pass_ability * 4.0) // 10.0 to 14.0 m/s
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

    fn handle_clear_ball_event(velocity: Vector3<f32>, field: &mut MatchField) {
        // Apply the clearing velocity to the ball
        field.ball.velocity = velocity;

        // Clear ownership - ball is now loose
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;

        // Set in-flight state to prevent immediate tackling
        field.ball.flags.in_flight_state = 10;
    }
}
