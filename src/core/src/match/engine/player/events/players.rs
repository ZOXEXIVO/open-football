use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{GoalDetail, MatchContext, MatchField, MatchPlayer, PlayerSide};
use crate::PlayerFieldPositionGroup;
use log::debug;
use nalgebra::Vector3;
use rand::{Rng, RngExt};

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
        // Low floors allow bad players (skill < 7) to be genuinely inaccurate
        let passing = (player.skills.technical.passing / 20.0).clamp(0.1, 1.0);
        let technique = (player.skills.technical.technique / 20.0).clamp(0.1, 1.0);
        let vision = (player.skills.mental.vision / 20.0).clamp(0.1, 1.0);
        let composure = (player.skills.mental.composure / 20.0).clamp(0.1, 1.0);
        let decisions = (player.skills.mental.decisions / 20.0).clamp(0.1, 1.0);
        let concentration = (player.skills.mental.concentration / 20.0).clamp(0.1, 1.0);
        let flair = (player.skills.mental.flair / 20.0).clamp(0.0, 1.0);
        let long_shots = (player.skills.technical.long_shots / 20.0).clamp(0.1, 1.0);
        let crossing = (player.skills.technical.crossing / 20.0).clamp(0.1, 1.0);
        let stamina = (player.skills.physical.stamina / 20.0).clamp(0.15, 1.0);
        let match_readiness = (player.skills.physical.match_readiness / 20.0).clamp(0.15, 1.0);

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
        let base_quality = self.passing * 0.5 + self.technique * 0.3 + self.vision * 0.2;
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

#[derive(Debug, Clone)]
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
    Offside(u32, Vector3<f32>),  // (offside_player_id, position_for_free_kick)
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
            match event {
                PlayerEvent::TakeBall(_) | PlayerEvent::ClaimBall(_) => {},
                _ => debug!("Player event: {:?}, tick = {}", event, context.time.time)
            }
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
                // Check offside before executing the pass
                let is_gk = field.players.iter()
                    .find(|p| p.id == pass_event_model.from_player_id)
                    .map(|p| p.tactical_position.current_position.position_group() == PlayerFieldPositionGroup::Goalkeeper)
                    .unwrap_or(false);

                if !is_gk && Self::is_receiver_offside(
                    pass_event_model.to_player_id,
                    pass_event_model.from_player_id,
                    field,
                ) {
                    let receiver_pos = field.players.iter()
                        .find(|p| p.id == pass_event_model.to_player_id)
                        .map(|p| p.position)
                        .unwrap_or(field.ball.position);

                    if context.logging_enabled {
                        debug!("Offside detected: player {} at position {:?}", pass_event_model.to_player_id, receiver_pos);
                    }

                    Self::handle_offside_event(pass_event_model.to_player_id, receiver_pos, field);
                } else {
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
            PlayerEvent::RequestBallReceive(player_id) => {
                Self::handle_request_ball_receive(player_id, field);
            }
            PlayerEvent::CommitFoul => {
                Self::handle_commit_foul_event(field);
            }
            PlayerEvent::Offside(player_id, position) => {
                Self::handle_offside_event(player_id, position, field);
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
        field.ball.pass_target_player_id = None;

        field.reset_players_positions();
        field.ball.reset();
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
        if let Some(player) = field.get_player_mut(player_id) {
            player.statistics.tackles += 1;
        }

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.clear_pass_history();
    }

    fn handle_ball_owner_change_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
    }

    fn handle_pass_to_event(event_model: PassingEventContext, field: &mut MatchField) {
        let mut rng = rand::rng();

        // Increment pass counters on the passer
        if let Some(passer) = field.get_player_mut(event_model.from_player_id) {
            passer.statistics.passes_attempted += 1;
            passer.statistics.passes_completed += 1;
        }

        // Extract player skills and condition
        let player = field.get_player(event_model.from_player_id).unwrap();
        let passer_position = player.position;
        let passer_side = player.side;
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
        let distance_error_factor = (horizontal_distance / 200.0).min(1.5);
        let max_position_error = 5.0 * (1.0 - accuracy_factor) * distance_error_factor;

        // Add random targeting error
        let mut target_error_x = if max_position_error > f32::EPSILON {
            rng.random_range(-max_position_error..max_position_error)
        } else {
            0.0
        };
        let mut target_error_y = if max_position_error > f32::EPSILON {
            rng.random_range(-max_position_error..max_position_error)
        } else {
            0.0
        };

        // Miskick chance for very low-technique players — ball goes off target
        let miskick_chance = (1.0 - skills.technique).powi(3) * 0.15;
        if rng.random_range(0.0f32..1.0) < miskick_chance {
            target_error_x += rng.random_range(-5.0f32..5.0);
            target_error_y += rng.random_range(-5.0f32..5.0);
        }

        // Calculate actual target with error
        let mut actual_target = Vector3::new(
            ideal_target.x + target_error_x,
            ideal_target.y + target_error_y,
            0.0,
        );

        // SAFETY: Prevent miskicked passes from going into passer's own goal
        // Even the worst passer wouldn't kick the ball directly into their own net
        {
            use crate::r#match::PlayerSide;
            let field_width = field.size.width as f32;
            let goal_safety_margin = 20.0;
            match passer_side {
                Some(PlayerSide::Left) => {
                    // Own goal at x ≈ 0 — keep target away from goal line
                    if actual_target.x < goal_safety_margin {
                        actual_target.x = passer_position.x.max(goal_safety_margin);
                    }
                }
                Some(PlayerSide::Right) => {
                    // Own goal at x ≈ field_width — keep target away from goal line
                    if actual_target.x > field_width - goal_safety_margin {
                        actual_target.x = passer_position.x.min(field_width - goal_safety_margin);
                    }
                }
                _ => {}
            }
        }

        let actual_pass_vector = actual_target - ball_position;
        let actual_horizontal_distance = Self::calculate_horizontal_distance(&actual_pass_vector);

        // Calculate pass force with power variation
        // Bad players hit passes with inconsistent power
        let power_consistency = 1.0 + (skills.technique * skills.stamina * 0.1);
        let power_variation_range = (1.0 - overall_quality) * 0.35;
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
        let passer_is_goalkeeper = passer.tactical_position.current_position.is_goalkeeper();

        let trajectory_type = Self::select_trajectory_type_contextual(
            actual_horizontal_distance,
            &skills,
            &mut rng,
            &passer_position,
            &actual_target,
            passer_team_id,
            &field.players,
        );

        // Goalkeeper long kicks must always be high arcs (goal kicks from penalty area)
        let trajectory_type = if passer_is_goalkeeper && actual_horizontal_distance > 60.0 {
            TrajectoryType::HighArc
        } else {
            trajectory_type
        };

        // Calculate z-velocity to reach target with chosen trajectory type
        let z_velocity = Self::calculate_trajectory_to_target(
            actual_horizontal_distance,
            &horizontal_velocity,
            trajectory_type,
            &skills,
            &mut rng,
        );

        let base_max_z = Self::calculate_max_z_velocity(actual_horizontal_distance, &skills);
        // Goalkeeper long kicks get a higher z-cap — goal kicks should fly high
        let max_z_velocity = if passer_is_goalkeeper && actual_horizontal_distance > 60.0 {
            base_max_z * 1.5
        } else {
            base_max_z
        };
        let final_z_velocity = z_velocity.min(max_z_velocity);

        // Calculate final velocity
        let mut final_velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            final_z_velocity,
        );

        // CRITICAL: Validate velocity to prevent cosmic-speed passes
        const MAX_PASS_VELOCITY: f32 = 7.0; // Cap for longest passes including lofted balls

        // Check for NaN or infinity
        if final_velocity.x.is_nan() || final_velocity.y.is_nan() || final_velocity.z.is_nan()
            || final_velocity.x.is_infinite() || final_velocity.y.is_infinite() || final_velocity.z.is_infinite()
        {
            // Fallback to a safe default velocity
            let safe_direction = actual_pass_vector.normalize();
            final_velocity = Vector3::new(
                safe_direction.x * 1.5,
                safe_direction.y * 1.5,
                0.3
            );
        }

        // Clamp velocity magnitude to maximum
        let velocity_magnitude = final_velocity.norm();
        if velocity_magnitude > MAX_PASS_VELOCITY {
            final_velocity = final_velocity * (MAX_PASS_VELOCITY / velocity_magnitude);
        }

        // Apply ball physics
        field.ball.velocity = final_velocity;

        // Record the passer in recent passers history before clearing ownership
        field.ball.record_passer(event_model.from_player_id);

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = Some(event_model.to_player_id);

        // Increase in_flight_state based on pass distance to prevent immediate reclaim
        // Ball velocity is low (~1-3 units/tick) so it needs significant protection
        // to travel most of the distance before opponents can claim
        // Short passes (< 30m): 40 ticks — covers ~45% of distance
        // Medium passes (30-80m): 60 ticks — covers ~60% of distance
        // Long passes (> 80m): 80 ticks — covers ~70% of distance
        let flight_protection = if actual_horizontal_distance < 30.0 {
            40
        } else if actual_horizontal_distance < 80.0 {
            60
        } else {
            80
        };
        field.ball.flags.in_flight_state = flight_protection;
    }

    fn calculate_horizontal_distance(ball_pass_vector: &Vector3<f32>) -> f32 {
        (ball_pass_vector.x * ball_pass_vector.x + ball_pass_vector.y * ball_pass_vector.y).sqrt()
    }

    fn calculate_horizontal_velocity(
        ball_pass_vector: &Vector3<f32>,
        pass_force: f32,
    ) -> Vector3<f32> {
        let horizontal_direction = Vector3::new(ball_pass_vector.x, ball_pass_vector.y, 0.0).normalize();
        let distance = (ball_pass_vector.x * ball_pass_vector.x + ball_pass_vector.y * ball_pass_vector.y).sqrt();

        // Calculate velocity needed to reach target accounting for friction and air drag
        // With ground friction factor 0.985/tick, total roll distance = v0 / 0.015
        // So v0 = distance * 0.015 for ground passes
        // Lofted passes experience air drag (proportional to v²) which bleeds much more speed,
        // plus 5% horizontal loss on each bounce — so longer passes need more overshoot
        const GROUND_FRICTION: f32 = 0.015;

        // Distance-dependent overshoot: short passes need little extra,
        // long passes need significantly more to compensate for air drag and bounce losses
        let overshoot = if distance < 50.0 {
            1.15 // Short: ground friction only
        } else if distance < 100.0 {
            1.25 // Medium: slight air drag on lofted balls
        } else if distance < 200.0 {
            1.45 // Long: significant air drag compensation
        } else {
            1.65 // Very long: heavy air drag + multiple bounces
        };

        let needed_velocity = distance * GROUND_FRICTION * overshoot;

        // pass_force (0.3-2.0) modulates: skilled players weight the pass better
        // Normalize to 0.90-1.1 range so it fine-tunes rather than drives the physics
        let skill_modifier = 0.90 + (pass_force.clamp(0.3, 2.0) - 0.3) * 0.12;

        horizontal_direction * (needed_velocity * skill_modifier)
    }

    /// Select trajectory type based on obstacles in the passing lane
    /// Simple rule: obstacles present → cross (lofted), no obstacles → ground pass
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
            let skill_bias = decision_quality * 0.3;
            (pure_random * randomness_factor + skill_bias).clamp(0.0, 1.0)
        };

        // Distance categories
        let is_short = horizontal_distance <= 100.0;
        let is_medium = horizontal_distance > 100.0 && horizontal_distance <= 200.0;
        let is_long = horizontal_distance > 200.0 && horizontal_distance <= 300.0;
        // Note: distances > 300.0 (very long) are handled in the else branches

        // MAIN LOGIC: If obstacles present, use lofted passes (crosses)
        // If no obstacles, use ground passes
        if obstacles_in_lane == 0 {
            // CLEAR LANE - Ground passes or low-driven passes
            if is_short {
                // Short passes - almost always ground
                if skill_influenced_random < 0.95 {
                    TrajectoryType::Ground
                } else if skills.flair * skills.technique > 0.75 {
                    TrajectoryType::Chip // Rare skillful chip
                } else {
                    TrajectoryType::Ground
                }
            } else if is_medium {
                // Medium passes - mostly ground, some driven
                if skill_influenced_random < 0.75 {
                    TrajectoryType::Ground      // 75% ground
                } else {
                    TrajectoryType::LowDriven   // 25% driven
                }
            } else if is_long {
                // Long passes need arc to cover distance even in clear lanes
                if skills.technique > 0.7 {
                    // Good technique - can pick trajectory precisely
                    if skill_influenced_random < 0.20 {
                        TrajectoryType::LowDriven  // 20% driven (skillful option)
                    } else if skill_influenced_random < 0.70 {
                        TrajectoryType::MediumArc  // 50% medium arc
                    } else {
                        TrajectoryType::HighArc    // 30% high arc
                    }
                } else {
                    // Average technique - default to higher arcs for safety
                    if skill_influenced_random < 0.10 {
                        TrajectoryType::LowDriven  // 10% driven
                    } else if skill_influenced_random < 0.55 {
                        TrajectoryType::MediumArc  // 45% medium arc
                    } else {
                        TrajectoryType::HighArc    // 45% high arc
                    }
                }
            } else {
                // Very long passes - almost always lofted
                let long_pass_ability = skills.long_shots * skills.vision * skills.crossing;
                if long_pass_ability > 0.7 {
                    // Elite long passer - precise lofted balls
                    if skill_influenced_random < 0.15 {
                        TrajectoryType::LowDriven  // 15% driven (exceptional skill)
                    } else if skill_influenced_random < 0.50 {
                        TrajectoryType::MediumArc  // 35% medium arc
                    } else {
                        TrajectoryType::HighArc    // 50% high arc
                    }
                } else {
                    // Average passer - high arcs to cover distance
                    if skill_influenced_random < 0.30 {
                        TrajectoryType::MediumArc  // 30% medium arc
                    } else {
                        TrajectoryType::HighArc    // 70% high arc
                    }
                }
            }
        } else {
            // OBSTACLES PRESENT - Use lofted passes (crosses)
            let many_obstacles = obstacles_in_lane >= 2;
            let has_good_crossing = skills.crossing > 0.7;

            if is_short {
                // Short pass with obstacles - chip or lift (NEVER low)
                if vision_quality > 0.7 && skill_influenced_random < 0.65 {
                    TrajectoryType::Chip // Smart chip over defender (65%)
                } else {
                    TrajectoryType::MediumArc // Medium loft (35%)
                }
            } else if is_medium {
                // Medium pass with obstacles - cross with arc (NEVER low)
                if many_obstacles {
                    // Multiple obstacles - higher arc needed
                    if skill_influenced_random < 0.70 {
                        TrajectoryType::HighArc     // 70% high cross
                    } else {
                        TrajectoryType::MediumArc   // 30% medium cross
                    }
                } else {
                    // One obstacle - medium/high arc to clear it
                    if skill_influenced_random < 0.70 {
                        TrajectoryType::MediumArc   // 70% medium cross
                    } else {
                        TrajectoryType::HighArc     // 30% high cross
                    }
                }
            } else if is_long {
                // Long pass with obstacles - definitely need arc
                if many_obstacles || has_good_crossing {
                    // Multiple obstacles or good crosser - high arc
                    if skill_influenced_random < 0.75 {
                        TrajectoryType::HighArc     // 75% high cross
                    } else {
                        TrajectoryType::MediumArc   // 25% medium cross
                    }
                } else {
                    // One obstacle - medium/high arc mix
                    if skill_influenced_random < 0.60 {
                        TrajectoryType::MediumArc   // 60% medium cross
                    } else {
                        TrajectoryType::HighArc     // 40% high cross
                    }
                }
            } else {
                // Very long pass with obstacles - high cross
                let long_pass_ability = skills.long_shots * skills.vision * skills.crossing;
                if long_pass_ability > 0.7 {
                    // Elite crosser - controlled high arc
                    if skill_influenced_random < 0.80 {
                        TrajectoryType::HighArc     // 80% high cross
                    } else {
                        TrajectoryType::MediumArc   // 20% medium cross
                    }
                } else {
                    // Average crosser - mostly high arc
                    if skill_influenced_random < 0.70 {
                        TrajectoryType::HighArc     // 70% high cross
                    } else {
                        TrajectoryType::MediumArc   // 30% medium cross
                    }
                }
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
        const LANE_WIDTH: f32 = 12.0; // Width of the passing lane corridor (accounts for player reach and movement)

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

            // Low driven - stays very close to ground, minimal arc (like real driven passes)
            TrajectoryType::LowDriven => {
                // Very slight lift - driven passes should barely leave the ground
                // Maximum height should be ~0.3-0.8m for realistic driven passes
                let distance_factor = (horizontal_distance / 150.0).clamp(0.2, 0.8);
                let skill_factor = skills.technique * skills.condition_factor;

                let base_z = 0.2 + (distance_factor * 0.5); // 0.2 to 0.7 m/s (much lower)
                let variation = rng.random_range(0.9..1.1);

                base_z * skill_factor * variation * tiny_random
            }

            // Medium arc - moderate parabolic trajectory (height ~1.5-3m, reduced)
            TrajectoryType::MediumArc => {
                let base_flight_time = horizontal_distance / horizontal_speed;
                let flight_time = base_flight_time * 0.5; // Reduced from 0.7 - lower arc

                let ideal_z = 0.65 * GRAVITY * flight_time; // Reduced from 0.8

                // Skill affects consistency
                let execution_quality = skills.overall_quality();
                let error_range = (1.0 - execution_quality) * 0.12;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                ideal_z * error * tiny_random
            }

            // High arc - high parabolic trajectory (height ~4-8m)
            TrajectoryType::HighArc => {
                let base_flight_time = horizontal_distance / horizontal_speed;
                let flight_time = base_flight_time * 1.5; // High arc

                let ideal_z = 0.8 * GRAVITY * flight_time;

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

        // Very strict limits - driven passes should dominate, not high arcs
        if horizontal_distance <= 20.0 {
            // Short passes - almost no lift allowed (ground passes)
            0.25 // Maximum 0.25 m/s vertical
        } else if horizontal_distance <= 45.0 {
            // Medium passes - keep very low (mostly ground/driven)
            0.6 + (long_pass_ability * 0.2) // 0.6 to 0.8 m/s
        } else if horizontal_distance <= 80.0 {
            // Long passes - still prefer low arcs
            1.2 + (long_pass_ability * 0.8) // 1.2 to 2.0 m/s (much lower)
        } else if horizontal_distance <= 150.0 {
            // Very long passes - moderate lift (reduced significantly)
            2.5 + (long_pass_ability * 1.5) // 2.5 to 4.0 m/s (was 4.5-7.0)
        } else if horizontal_distance <= 250.0 {
            // Ultra-long diagonal switches - still prefer lower when possible
            4.5 + (long_pass_ability * 2.5) // 4.5 to 7.0 m/s (was 7.0-10.0)
        } else {
            // Extreme distance - goalkeeper goal kicks, clearances
            7.0 + (long_pass_ability * 3.5) // 7.0 to 10.5 m/s (was 10.0-14.0)
        }
    }

    fn handle_claim_ball_event(player_id: u32, field: &mut MatchField) {
        // CLAIM COOLDOWN: Prevent rapid ping-pong between players
        // If the ball was just claimed by someone else, reject this claim
        const CLAIM_COOLDOWN_TICKS: u32 = 15; // ~250ms at 60fps - time before ball can change hands

        // IN-FLIGHT CHECK: If ball was just passed, only allow the intended receiver to claim
        // This prevents the passer from reclaiming via a stale ClaimBall event that was
        // generated before the PassTo event cleared ownership in the same tick
        if field.ball.flags.in_flight_state > 0 {
            if let Some(target_id) = field.ball.pass_target_player_id {
                if player_id != target_id {
                    return; // Reject claim from non-target during in-flight
                }
            } else {
                // Ball is in flight with no target (e.g., shot) - reject all claims
                return;
            }
        }

        // If there's a cooldown active and this player doesn't already own the ball
        if field.ball.claim_cooldown > 0 {
            if let Some(current_owner) = field.ball.current_owner {
                if current_owner != player_id {
                    // Ball was just claimed by someone else - reject this claim
                    return;
                }
            }
        }

        // If there's already an owner and they're different from the claimer
        // Only allow the claim if enough time has passed (ownership_duration check)
        if let Some(current_owner) = field.ball.current_owner {
            if current_owner == player_id {
                // Already owns the ball (e.g. try_pass_target_claim already set ownership)
                // Don't reset previous_owner — it tracks who passed to us
                return;
            }

            // Different player trying to claim - this is a tackle/interception
            // Reject if current owner hasn't held ball long enough (prevents ping-pong)
            let min_duration = if field.ball.contested_claim_count > 3 { 60 } else { 25 };
            if field.ball.ownership_duration < min_duration {
                return;
            }

            // Allow claim with escalated cooldown
            field.ball.previous_owner = Some(current_owner);
            field.ball.current_owner = Some(player_id);
            field.ball.pass_target_player_id = None;
            field.ball.ownership_duration = 0;
            field.ball.contested_claim_count += 1;
            let cooldown = if field.ball.contested_claim_count > 6 {
                90
            } else if field.ball.contested_claim_count > 3 {
                45
            } else {
                CLAIM_COOLDOWN_TICKS
            };
            field.ball.claim_cooldown = cooldown;
            field.ball.flags.in_flight_state = cooldown as usize;
            return;
        }

        // No current owner - normal claim
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.pass_target_player_id = None;
        field.ball.ownership_duration = 0;
        field.ball.claim_cooldown = CLAIM_COOLDOWN_TICKS;
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
        field.ball.pass_target_player_id = None;
        field.ball.clear_pass_history();

        field.ball.flags.in_flight_state = 100;
    }

    fn handle_shoot_event(shoot_event_model: ShootingEventContext, field: &mut MatchField) {
        const GOAL_WIDTH: f32 = 45.0; // Half-width of goal in game units (matches engine GOAL_WIDTH)
        #[allow(dead_code)]
        const GOAL_HEIGHT: f32 = 8.0; // Height of crossbar
        const MAX_SHOT_VELOCITY: f32 = 8.0; // Maximum realistic shot velocity per tick
        const MIN_SHOT_DISTANCE: f32 = 1.0; // Minimum distance to prevent NaN from normalization

        let mut rng = rand::rng();

        // Get player skills for power and accuracy calculations
        let player = field.get_player(shoot_event_model.from_player_id).unwrap();

        // Low floors let bad players (skill < 7) be genuinely inaccurate
        let finishing_skill = (player.skills.technical.finishing / 20.0).clamp(0.1, 1.0);
        let technique_skill = (player.skills.technical.technique / 20.0).clamp(0.1, 1.0);
        let long_shot_skill = (player.skills.technical.long_shots / 20.0).clamp(0.1, 1.0);
        let composure_skill = (player.skills.mental.composure / 20.0).clamp(0.1, 1.0);
        let decisions_skill = (player.skills.mental.decisions / 20.0).clamp(0.1, 1.0);

        // Determine which goal we're shooting at
        let goal_center = shoot_event_model.target;

        // Calculate goal bounds
        let goal_left_post = goal_center.y - GOAL_WIDTH;
        let goal_right_post = goal_center.y + GOAL_WIDTH;

        // Calculate distance to goal
        let ball_to_goal_vector = goal_center - field.ball.position;
        let horizontal_distance = (ball_to_goal_vector.x * ball_to_goal_vector.x +
                                   ball_to_goal_vector.y * ball_to_goal_vector.y).sqrt();

        // Safety check: if ball is already at/very near the goal, just give it a gentle push
        if horizontal_distance < MIN_SHOT_DISTANCE {
            let direction = if ball_to_goal_vector.x.abs() > 0.01 {
                Vector3::new(ball_to_goal_vector.x.signum(), 0.0, 0.0)
            } else {
                Vector3::new(1.0, 0.0, 0.0) // Default direction
            };
            field.ball.previous_owner = Some(shoot_event_model.from_player_id);
            field.ball.current_owner = None;
            field.ball.velocity = direction * 2.0; // Gentle push
            field.ball.flags.in_flight_state = 20;
            return;
        }

        // Calculate overall shooting accuracy (0.0 to 1.0)
        let base_accuracy = if horizontal_distance > 100.0 {
            // Long shots depend more on long_shot skill and technique
            (long_shot_skill * 0.5 + technique_skill * 0.3 + finishing_skill * 0.2) * composure_skill
        } else if horizontal_distance > 50.0 {
            // Medium range - balanced
            (finishing_skill * 0.4 + technique_skill * 0.3 + long_shot_skill * 0.3) * composure_skill
        } else {
            // Close range - finishing is key
            finishing_skill * 0.6 + technique_skill * 0.2 + composure_skill * 0.2
        };

        // Calculate target point within goal (aim for corners/areas based on skill)
        // Better players aim for harder-to-save spots (corners)
        let target_preference = rng.random_range(0.0..1.0);
        let ideal_y_target = if target_preference < 0.3 {
            // Aim for left side of goal (30%)
            goal_center.y - (GOAL_WIDTH * 0.6) * decisions_skill
        } else if target_preference < 0.6 {
            // Aim for right side of goal (30%)
            goal_center.y + (GOAL_WIDTH * 0.6) * decisions_skill
        } else {
            // Aim more central (40%) - safer but easier to save
            goal_center.y + rng.random_range(-GOAL_WIDTH * 0.3..GOAL_WIDTH * 0.3)
        };

        // Add shooting error based on skills and distance
        // Error increases with distance and decreases with skill
        let distance_error_factor = (horizontal_distance / 80.0).clamp(0.8, 3.0);

        // Calculate positional error (how far from intended target)
        // Distance penalty multiplier for base_accuracy — close range should be very accurate
        let distance_penalty = if horizontal_distance > 200.0 {
            0.12
        } else if horizontal_distance > 150.0 {
            0.20
        } else if horizontal_distance > 100.0 {
            0.32
        } else if horizontal_distance > 70.0 {
            0.50
        } else if horizontal_distance > 50.0 {
            0.68
        } else if horizontal_distance > 30.0 {
            0.85
        } else if horizontal_distance > 15.0 {
            0.93
        } else {
            0.98
        };
        let adjusted_accuracy = base_accuracy * distance_penalty;

        // Base error: elite close-range ±2-5 units, poor long-range ±30-60 units
        let base_position_error = 45.0 * distance_error_factor * (1.0 - adjusted_accuracy);
        let min_error = if horizontal_distance < 30.0 { 2.0 } else if horizontal_distance < 60.0 { 4.0 } else { 8.0 };
        let max_y_error = base_position_error.clamp(min_error, 90.0);

        // Add random error to y-coordinate
        let y_error = rng.random_range(-max_y_error..max_y_error);
        let mut actual_y_target = ideal_y_target + y_error;

        // Wide miss chance: distance-dependent — close range is much more accurate
        // Real football: ~50% of all shots miss the frame, but close range (<15m) is ~25%
        let wide_miss_base = if horizontal_distance < 30.0 {
            0.04 // Very close — skilled players rarely miss the frame
        } else if horizontal_distance < 60.0 {
            0.10
        } else if horizontal_distance < 100.0 {
            0.22
        } else {
            0.35 // Long range — high base miss rate
        };
        let wide_miss_chance = (1.0 - adjusted_accuracy) * 0.4 + wide_miss_base;
        if rng.random_range(0.0f32..1.0) < wide_miss_chance {
            // Shot goes wide — force y outside goal posts
            let extra_wide = rng.random_range(GOAL_WIDTH * 0.2..GOAL_WIDTH * 1.5);
            if rng.random_range(0.0f32..1.0) < 0.5 {
                actual_y_target = goal_right_post + extra_wide; // Wide right
            } else {
                actual_y_target = goal_left_post - extra_wide; // Wide left
            }
        }

        // Miskick chance for very low-technique players — shot goes way off target
        let miskick_chance = (1.0 - technique_skill).powi(3) * 0.3;
        if rng.random_range(0.0f32..1.0) < miskick_chance {
            actual_y_target += rng.random_range(-GOAL_WIDTH * 1.5..GOAL_WIDTH * 1.5);
        }

        // Clamp to reasonable bounds — allow shots to miss by up to 3x goal width
        let max_miss_distance = GOAL_WIDTH * 3.0;
        let clamped_y_target = actual_y_target.clamp(
            goal_left_post - max_miss_distance,
            goal_right_post + max_miss_distance
        );

        // Calculate final shot direction
        let actual_target = Vector3::new(goal_center.x, clamped_y_target, 0.0);
        let shot_vector = actual_target - field.ball.position;

        // Calculate skill-based power multiplier (better players shoot harder)
        let power_skill_factor = (finishing_skill * 0.5) + (technique_skill * 0.3) + (long_shot_skill * 0.2);
        let power_multiplier = 0.95 + (power_skill_factor * 0.35); // Range: 0.95 to 1.30

        // Calculate horizontal velocity with skill-based power
        let horizontal_direction = Vector3::new(shot_vector.x, shot_vector.y, 0.0).normalize();
        let base_horizontal_velocity = shoot_event_model.force as f32 * power_multiplier * 1.6;

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
        let vertical_spin_variation = rng.random_range(0.90..1.10);

        // Over-the-bar miss chance: distance-dependent
        // Close range: players keep shots low. Long range: more ballooning
        let over_bar_base = if horizontal_distance < 30.0 {
            0.03 // Close range — very rarely sky it
        } else if horizontal_distance < 60.0 {
            0.08
        } else if horizontal_distance < 100.0 {
            0.14
        } else {
            0.22 // Long range — more likely to balloon over
        };
        let over_bar_chance = (1.0 - adjusted_accuracy) * 0.35 + over_bar_base;
        let z_velocity = if rng.random_range(0.0f32..1.0) < over_bar_chance {
            // Shot goes over the bar — set z high enough to clear crossbar (GOAL_HEIGHT = 8.0)
            // Ball needs to reach height > 8.0 during flight, so z_velocity must be significant
            rng.random_range(3.0..6.0) // Guaranteed to fly high over the bar
        } else {
            (base_z_velocity * height_variation * vertical_spin_variation).min(5.0)
        };

        // Calculate final velocity
        let mut final_velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            z_velocity
        );

        // CRITICAL: Validate and clamp velocity to prevent cosmic-speed shots
        // Check for NaN or infinity
        if final_velocity.x.is_nan() || final_velocity.y.is_nan() || final_velocity.z.is_nan()
            || final_velocity.x.is_infinite() || final_velocity.y.is_infinite() || final_velocity.z.is_infinite()
        {
            // Fallback to a safe default velocity toward the goal
            let safe_direction = (goal_center - field.ball.position).normalize();
            final_velocity = Vector3::new(
                safe_direction.x * 5.0,
                safe_direction.y * 5.0,
                1.0
            );
        }

        // Clamp velocity magnitude to maximum realistic shot speed
        let velocity_magnitude = final_velocity.norm();
        if velocity_magnitude > MAX_SHOT_VELOCITY {
            final_velocity = final_velocity * (MAX_SHOT_VELOCITY / velocity_magnitude);
        }

        // Record shot in player memory
        let on_target = clamped_y_target >= goal_left_post && clamped_y_target <= goal_right_post;
        if let Some(shooter) = field.get_player_mut(shoot_event_model.from_player_id) {
            shooter.memory.record_shot(shoot_event_model.tick, on_target);
        }

        field.ball.previous_owner = Some(shoot_event_model.from_player_id);
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;
        field.ball.velocity = final_velocity;

        // Shorter flight protection for shots — allows defenders/GK to claim sooner
        field.ball.flags.in_flight_state = 40;
    }

    fn handle_caught_ball_event(player_id: u32, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        // Ball must stop when caught — prevent it from continuing into the goal
        field.ball.velocity = Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        field.ball.claim_cooldown = 30;
        field.ball.pass_target_player_id = None;
    }

    fn handle_move_player_event(player_id: u32, position: Vector3<f32>, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();
        player.position = position;
    }

    fn handle_take_ball_event(player_id: u32, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();
        player.run_for_ball();
    }

    fn handle_request_ball_receive(player_id: u32, field: &mut MatchField) {
        // Only allow if ball is close and either unowned or this player is the target
        let is_target = field.ball.pass_target_player_id == Some(player_id);
        let is_unowned = field.ball.current_owner.is_none();

        if !is_target && !is_unowned {
            return;
        }

        // Copy ball position to avoid borrow conflict
        let ball_pos = field.ball.position;

        let player = match field.get_player(player_id) {
            Some(p) => p,
            None => return,
        };

        let dx = player.position.x - ball_pos.x;
        let dy = player.position.y - ball_pos.y;
        let distance = (dx * dx + dy * dy).sqrt();

        if distance < 3.5 && ball_pos.z <= 2.8 {
            field.ball.previous_owner = field.ball.current_owner;
            field.ball.current_owner = Some(player_id);
            field.ball.pass_target_player_id = None;
            field.ball.ownership_duration = 0;
            field.ball.claim_cooldown = 15;
            field.ball.flags.in_flight_state = 0;
        }
    }

    fn handle_commit_foul_event(field: &mut MatchField) {
        // When a foul is committed, the current ball owner (victim) gets protected possession
        // This simulates a free kick - the victim gets time to act without being challenged
        if field.ball.current_owner.is_some() {
            field.ball.claim_cooldown = 150; // ~2.5 seconds of protection (free kick setup)
            field.ball.flags.in_flight_state = 150; // Prevent ClaimBall events from tackling states
            field.ball.contested_claim_count = 0; // Reset contested counter
        }
    }

    /// Check if the receiver is in an offside position at the moment of the pass.
    /// FIFA rules: a player is offside if they are
    ///   1) in the opponent's half,
    ///   2) ahead of the ball, and
    ///   3) beyond the second-to-last opponent (including goalkeeper).
    fn is_receiver_offside(
        receiver_id: u32,
        passer_id: u32,
        field: &MatchField,
    ) -> bool {
        let receiver = match field.players.iter().find(|p| p.id == receiver_id) {
            Some(p) => p,
            None => return false,
        };

        // Verify passer exists
        if !field.players.iter().any(|p| p.id == passer_id) {
            return false;
        }

        let receiver_side = match receiver.side {
            Some(s) => s,
            None => return false,
        };

        let half_width = field.size.half_width as f32;
        let ball_x = field.ball.position.x;
        let receiver_x = receiver.position.x;

        // Tolerance to avoid marginal false positives
        const TOLERANCE: f32 = 1.0;

        match receiver_side {
            PlayerSide::Left => {
                // Left side attacks right: opponent goal at x = field_width
                // Must be in opponent's half (past halfway)
                if receiver_x < half_width {
                    return false;
                }
                // Must be ahead of the ball (closer to opponent goal)
                if receiver_x <= ball_x + TOLERANCE {
                    return false;
                }
                // Collect all opponents (Right side players)
                // Right side's own goal is at x = field_width
                // Sort DESCENDING so [0] = closest to their goal (GK), [1] = second-to-last
                let mut opponent_xs: Vec<f32> = field.players.iter()
                    .filter(|p| p.side == Some(PlayerSide::Right))
                    .map(|p| p.position.x)
                    .collect();
                opponent_xs.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

                if opponent_xs.len() < 2 {
                    return false;
                }
                let second_last_x = opponent_xs[1];

                // Offside if receiver is beyond (greater x) the second-to-last opponent
                receiver_x > second_last_x + TOLERANCE
            }
            PlayerSide::Right => {
                // Right side attacks left: opponent goal at x = 0
                // Must be in opponent's half (before halfway)
                if receiver_x > half_width {
                    return false;
                }
                // Must be ahead of the ball (closer to opponent goal, i.e. smaller x)
                if receiver_x >= ball_x - TOLERANCE {
                    return false;
                }
                // Collect all opponents (Left side players)
                // Left side's own goal is at x = 0
                // Sort ASCENDING so [0] = closest to their goal (GK), [1] = second-to-last
                let mut opponent_xs: Vec<f32> = field.players.iter()
                    .filter(|p| p.side == Some(PlayerSide::Left))
                    .map(|p| p.position.x)
                    .collect();
                opponent_xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                if opponent_xs.len() < 2 {
                    return false;
                }
                let second_last_x = opponent_xs[1];

                // Offside if receiver is beyond (smaller x) the second-to-last opponent
                receiver_x < second_last_x - TOLERANCE
            }
        }
    }

    /// Handle an offside event: stop the ball, award a free kick to the nearest opponent.
    fn handle_offside_event(offside_player_id: u32, position: Vector3<f32>, field: &mut MatchField) {
        // Increment offside stat on the player
        if let Some(player) = field.players.iter_mut().find(|p| p.id == offside_player_id) {
            player.statistics.offsides += 1;
        }

        // Determine the offside player's side to find opponents
        let offside_side = field.players.iter()
            .find(|p| p.id == offside_player_id)
            .and_then(|p| p.side);

        // Find nearest opponent to the offside position to award free kick
        let nearest_opponent_id = field.players.iter()
            .filter(|p| p.side != offside_side && p.side.is_some())
            .min_by(|a, b| {
                let dist_a = (a.position - position).norm();
                let dist_b = (b.position - position).norm();
                dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id);

        // Stop ball at offside position
        field.ball.position = position;
        field.ball.velocity = Vector3::new(0.0, 0.0, 0.0);

        // Award possession to nearest opponent (free kick)
        if let Some(opponent_id) = nearest_opponent_id {
            field.ball.previous_owner = field.ball.current_owner;
            field.ball.current_owner = Some(opponent_id);
            field.ball.ownership_duration = 0;
        }

        // Protected possession (same pattern as foul free kick)
        field.ball.claim_cooldown = 60;
        field.ball.flags.in_flight_state = 60;
        field.ball.contested_claim_count = 0;
        field.ball.pass_target_player_id = None;
        field.ball.clear_pass_history();
    }

    fn handle_clear_ball_event(velocity: Vector3<f32>, field: &mut MatchField) {
        // Cap clearance velocity to prevent unrealistic ball speed
        // Clearances are powerful kicks - higher cap than passes (7.2)
        const MAX_CLEAR_VELOCITY: f32 = 14.0;
        let speed = velocity.norm();
        let mut capped_velocity = if speed > MAX_CLEAR_VELOCITY {
            velocity * (MAX_CLEAR_VELOCITY / speed)
        } else {
            velocity
        };

        // SAFETY: Prevent clearances from going toward own goal
        // A clearance should always go AWAY from own goal, never toward it
        {
            use crate::r#match::PlayerSide;
            if let Some(clearer_id) = field.ball.current_owner {
                if let Some(clearer) = field.get_player(clearer_id) {
                    match clearer.side {
                        Some(PlayerSide::Left) => {
                            // Own goal at x ≈ 0 — clearance must go forward (positive x)
                            if capped_velocity.x < 0.0 {
                                capped_velocity.x = capped_velocity.x.abs();
                            }
                        }
                        Some(PlayerSide::Right) => {
                            // Own goal at x ≈ field_width — clearance must go backward (negative x)
                            if capped_velocity.x > 0.0 {
                                capped_velocity.x = -capped_velocity.x.abs();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Apply the clearing velocity to the ball
        field.ball.velocity = capped_velocity;

        // Clear ownership - ball is now loose
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;
        field.ball.clear_pass_history();

        // Set in-flight state to prevent immediate reclaim after clearance
        field.ball.flags.in_flight_state = 40;
    }
}
