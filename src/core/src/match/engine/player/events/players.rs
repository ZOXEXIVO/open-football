use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{GoalDetail, MatchContext, MatchField, MatchPlayer, PlayerSide, ShotTarget};
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

/// How dirty was the foul — drives card probabilities.
/// `Normal`: shirt pull, mistimed challenge → occasional yellow.
/// `Reckless`: studs-up, late, from behind → high yellow, sometimes red.
/// `Violent`: denial of goalscoring opportunity or violent conduct → direct red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoulSeverity {
    Normal,
    Reckless,
    Violent,
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
    /// Goalkeeper got a touch on a real shot but couldn't catch it —
    /// the ball deflected away (parried wide, palmed over the bar,
    /// punched off the line). Emitted by the diving and catching states
    /// when they exit on "ball moving away" while `cached_shot_target`
    /// was set. The handler credits a save and increments `shots_faced`
    /// so the rating helper sees the GK's full workload.
    ParriedBall(u32),
    /// Foul committed by (fouler_id, severity). Dispatcher decides cards.
    CommitFoul(u32, FoulSeverity),
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
                Self::record_team_possession_if_switch(player_id, field, context);
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
                    let passer_id = pass_event_model.from_player_id;
                    Self::handle_pass_to_event(pass_event_model, field);
                    // Tag the ball with the passer for pass-accuracy
                    // accounting. Lives for a short window (150 ticks)
                    // and is cleared on opponent touch — see ball.rs
                    // `pending_pass_passer` docs.
                    field.ball.pending_pass_passer = Some(passer_id);
                    field.ball.pending_pass_set_tick = context.current_tick();
                }
            }
            PlayerEvent::ClaimBall(player_id) => {
                Self::record_team_possession_if_switch(player_id, field, context);
                Self::handle_claim_ball_event(player_id, field);
            }
            PlayerEvent::MoveBall(player_id, ball_velocity) => {
                Self::handle_move_ball_event(player_id, ball_velocity, field);
            }
            PlayerEvent::GainBall(player_id) => {
                Self::handle_gain_ball_event(player_id, field);
            }
            PlayerEvent::Shoot(shoot_event_model) => {
                // Capture field dimensions up-front so the log block
                // below (which runs under &Player borrow) doesn't try
                // to re-borrow field.size. Feature-gated so the capture
                // compiles away when logs are off.
                #[cfg(feature = "match-logs")]
                let field_w = field.size.width as f32;
                #[cfg(feature = "match-logs")]
                let field_h = field.size.height as f32;

                // Record shot at team level for cooldown. Log reason +
                // source context only when `match-logs` feature is
                // enabled (see `core/src/match_logs.rs`).
                if let Some(player) = field.get_player(shoot_event_model.from_player_id) {
                    let team_id = player.team_id;
                    let tick = context.current_tick();
                    #[cfg(feature = "match-logs")]
                    {
                        let pos = player.position;
                        let goal_dist = if let Some(side) = player.side {
                            let goal_x = match side {
                                crate::r#match::PlayerSide::Left => field_w,
                                crate::r#match::PlayerSide::Right => 0.0,
                            };
                            let goal_y = field_h / 2.0;
                            ((pos.x - goal_x).powi(2) + (pos.y - goal_y).powi(2)).sqrt()
                        } else {
                            0.0
                        };
                        let pos_tag = match player.tactical_position.current_position.position_group() {
                            PlayerFieldPositionGroup::Goalkeeper => "GK",
                            PlayerFieldPositionGroup::Defender => "DEF",
                            PlayerFieldPositionGroup::Midfielder => "MID",
                            PlayerFieldPositionGroup::Forward => "FWD",
                        };
                        crate::match_log_info!(
                            "SHOT team={} pos={} player={} state={} reason={} dist={:.1} tick={}",
                            team_id, pos_tag,
                            shoot_event_model.from_player_id,
                            player.state,
                            shoot_event_model.reason, goal_dist, tick
                        );
                    }
                    context.coach_for_team_mut(team_id).record_shot(tick);
                }
                if let Some(player) = field.get_player_mut(shoot_event_model.from_player_id) {
                    player.pending_shot_reason = None;
                }
                Self::handle_shoot_event(shoot_event_model, field);
            }
            PlayerEvent::CaughtBall(player_id) => {
                Self::handle_caught_ball_event(player_id, field);
            }
            PlayerEvent::ParriedBall(player_id) => {
                Self::handle_parried_ball_event(player_id, field);
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
            PlayerEvent::CommitFoul(fouler_id, severity) => {
                Self::handle_commit_foul_event(fouler_id, severity, field, context);
            }
            PlayerEvent::Offside(player_id, position) => {
                Self::handle_offside_event(player_id, position, field);
            }
            _ => {} // Ignore unsupported events
        }

        remaining_events
    }

    fn handle_goal_event(player_id: u32, is_auto_goal: bool, field: &mut MatchField, context: &mut MatchContext) {
        let scorer_team_id = field.get_player(player_id).map(|p| p.team_id);
        let player = field.get_player_mut(player_id).unwrap();

        player.statistics.add_goal(context.total_match_time, is_auto_goal);

        // Goal stands → credit on-target to the real scorer. Own goals
        // aren't counted as an on-target shot for the defender that
        // deflected the ball.
        if !is_auto_goal {
            player.memory.credit_shot_on_target();
        }

        // Credit the conceding goalkeeper's `shots_faced` so the rating
        // helper has the right denominator for save percentage. Auto-
        // goals (own-goals) don't count — those aren't shots the GK got
        // beaten by, they're defensive errors.
        if !is_auto_goal {
            if let Some(scoring_team) = scorer_team_id {
                let conceding_gk_id = field
                    .players
                    .iter()
                    .find(|p| {
                        p.team_id != scoring_team
                            && p.tactical_position
                                .current_position
                                .position_group()
                                == PlayerFieldPositionGroup::Goalkeeper
                    })
                    .map(|p| p.id);
                if let Some(gk_id) = conceding_gk_id {
                    if let Some(gk) = field.get_player_mut(gk_id) {
                        gk.statistics.shots_faced += 1;
                    }
                }
            }
        }

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal,
            time: context.total_match_time,
        });
        context.record_stoppage_time(30_000);

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
        Self::secure_ball_for(player_id, field);
        field.ball.clear_pass_history();
    }

    fn handle_ball_owner_change_event(player_id: u32, field: &mut MatchField) {
        Self::secure_ball_for(player_id, field);
    }

    fn handle_pass_to_event(event_model: PassingEventContext, field: &mut MatchField) {
        let mut rng = rand::rng();

        // Only increment attempts here. `passes_completed` is bumped when
        // the intended receiver actually claims the ball (see
        // `handle_claim_ball_event`). Previously both were bumped at
        // emit-time, so the metric was "passes emitted" — 99% accuracy
        // regardless of whether the ball reached the target. Real-football
        // pass accuracy sits around 85%; the inflated metric hid the
        // fact that our possession loop wasn't losing the ball often
        // enough via wayward passes.
        if let Some(passer) = field.get_player_mut(event_model.from_player_id) {
            passer.statistics.passes_attempted += 1;
        }

        // Extract player skills and condition
        let player = field.get_player(event_model.from_player_id).unwrap();
        let passer_position = player.position;
        let passer_side = player.side;
        let skills = PassSkills::from_player(player);

        // Calculate overall quality for accuracy - affected by condition
        let overall_quality = skills.overall_quality();

        // Calculate ideal target position — lead the pass toward receiver's movement
        let receiver_pos = event_model.pass_target;
        let receiver_velocity = field.get_player(event_model.to_player_id)
            .map(|p| p.velocity)
            .unwrap_or(Vector3::zeros());

        // Lead pass: target where the receiver will be when the ball
        // arrives. Ground-pass flight time is `distance * 0.015 * 1.15`
        // ÷ friction ≈ 70-95 ticks across short/medium passes. Elite
        // passers predict this correctly; poor passers under- or
        // over-estimate, so the lead itself is skill-dependent.
        //
        // Flight-time estimate: we aim `lead_ticks` ahead along the
        // receiver's current velocity, where `lead_ticks` = a fraction
        // of true flight time determined by vision + passing quality.
        let pass_distance_est = (receiver_pos - passer_position).magnitude();
        let flight_time_est = (pass_distance_est * 0.85).clamp(25.0, 95.0);
        // Vision = how well we anticipate the receiver's run.
        // Passing = technical precision on the pass itself.
        let anticipation = (skills.vision * 0.6 + skills.passing * 0.4).clamp(0.0, 1.0);
        // Skilled passers lead fully; poor passers lead most of the way.
        // Widened base from 0.40 → 0.60 after the pass-accuracy audit
        // showed average-anticipation passers were under-leading by
        // enough that receivers arrived at the ball 3-6u short — just
        // outside the tightest receiver claim windows, enough passes
        // failed to push team accuracy to 72% instead of the 85% target.
        let lead_fraction = 0.60 + anticipation * 0.35; // 0.60..0.95
        let lead_ticks = flight_time_est * lead_fraction;
        let ideal_target = receiver_pos + receiver_velocity * lead_ticks;

        // Always use passer's position as pass origin — ball position may lag behind
        let pass_origin = passer_position;
        let ideal_pass_vector = ideal_target - pass_origin;
        let horizontal_distance = Self::calculate_horizontal_distance(&ideal_pass_vector);

        // Skill-based targeting error. Steeper skill spread than the
        // previous linear formula — an elite passer (passing 18,
        // technique 18, concentration 18) hits within ~0.4u; an average
        // passer ~2.5u; a poor passer (all 6) ~7u.
        // Squared accuracy_factor sharpens the drop-off: the gap
        // between "world class" and "pro-level" accuracy is larger
        // than the gap between "pro-level" and "average".
        let accuracy_factor = (overall_quality * skills.concentration).clamp(0.0, 1.0);
        let precision = accuracy_factor * accuracy_factor;

        // Distance-based error: longer passes have more positional error.
        // Curve also steepened so 20u passes are near-perfect for
        // skilled players, while 200u passes lose significant accuracy.
        let distance_error_factor = (horizontal_distance / 250.0).clamp(0.1, 1.8);

        // Max error scales from 0.3u (elite) to 5.5u (poor), modulated
        // by distance. Narrowed from 9.0 → 5.5 — the old ceiling was
        // large enough that an average passer's random 6-8u error
        // combined with a 3-5u lead underestimation pushed the ball
        // consistently just outside the receiver claim radius. Real
        // football average passers deliver within ~1.5m, not ~4m.
        let max_position_error = (0.3 + (1.0 - precision) * 5.5) * distance_error_factor;

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

        // Miskick chance — heavily gated by technique. Elite technique
        // basically never miskicks; poor technique (≤6) fires wild ~8%
        // of the time (5th-power curve concentrates miskicks among
        // genuinely unskilled players). Previous cubic curve produced
        // ~5% even for average players, which muddied the skill signal.
        let miskick_chance = (1.0 - skills.technique).powi(5) * 0.25;
        if rng.random_range(0.0f32..1.0) < miskick_chance {
            target_error_x += rng.random_range(-8.0f32..8.0);
            target_error_y += rng.random_range(-8.0f32..8.0);
        }

        // Calculate actual target with error
        let mut actual_target = Vector3::new(
            ideal_target.x + target_error_x,
            ideal_target.y + target_error_y,
            0.0,
        );

        // SAFETY: keep pass target away from BOTH goals.
        // Own goal: obvious — no one passes into their own net.
        // Opposing goal: a pass aimed within a few yards of the opponent
        // goal line with even a small error scoots straight into the net
        // as an unowned ball → `check_goal` credits the passer → logged
        // as a "goal" that never involved a Shoot event. Logs showed
        // 10-15 of these per team per match, which was the primary
        // source of 20+ goal scorelines. Passes should always land in
        // playable space — clearances and shots have their own explicit
        // paths for getting the ball across a goal line.
        {
            use crate::r#match::PlayerSide;
            let field_width = field.size.width as f32;
            let goal_safety_margin = 20.0;
            match passer_side {
                Some(PlayerSide::Left) => {
                    // Own goal at x ≈ 0; opposing goal at x ≈ field_width.
                    if actual_target.x < goal_safety_margin {
                        actual_target.x = passer_position.x.max(goal_safety_margin);
                    }
                    if actual_target.x > field_width - goal_safety_margin {
                        actual_target.x = field_width - goal_safety_margin;
                    }
                }
                Some(PlayerSide::Right) => {
                    // Own goal at x ≈ field_width; opposing goal at x ≈ 0.
                    if actual_target.x > field_width - goal_safety_margin {
                        actual_target.x = passer_position.x.min(field_width - goal_safety_margin);
                    }
                    if actual_target.x < goal_safety_margin {
                        actual_target.x = goal_safety_margin;
                    }
                }
                _ => {}
            }
        }

        let actual_pass_vector = actual_target - pass_origin;
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

        // CRITICAL: Validate velocity to prevent cosmic-speed passes.
        // Field is 840u = 105m (1u = 0.125m), simulation at 100 ticks/s.
        // Old 7.0 u/tick = 87.5 m/s = 315 km/h — the same unit-conversion
        // error family as everything else, assuming "1u ≈ 0.5m". Real
        // football pass speeds: short ball 5-15 m/s, medium 15-25 m/s,
        // elite driven/long pass tops out ~35 m/s. Cap at 3.2 u/tick
        // (40 m/s) — elite piledriver territory, never broken by any
        // outfield pass in real play. Matches the MAX_SHOT_VELOCITY
        // calibration and restores the real shot/player speed ratio.
        const MAX_PASS_VELOCITY: f32 = 3.2;

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

        // Distance categories — calibrated for 840-unit field (1 unit ≈ 0.5m)
        let is_short = horizontal_distance <= 30.0;       // ~15m — quick one-touch
        let is_medium = horizontal_distance > 30.0 && horizontal_distance <= 60.0;  // 15-30m
        let is_long = horizontal_distance > 60.0 && horizontal_distance <= 120.0;   // 30-60m
        // > 120 units = very long (60m+)

        // Passes should be lofted based on BOTH obstacles AND distance.
        // In real football, passes over 25m often leave the ground even without obstacles.
        if obstacles_in_lane == 0 {
            // CLEAR LANE — trajectory based on distance
            if is_short {
                // Short passes — ground
                TrajectoryType::Ground
            } else if is_medium {
                // Medium passes — mostly ground, some driven
                if skill_influenced_random < 0.60 {
                    TrajectoryType::Ground
                } else {
                    TrajectoryType::LowDriven
                }
            } else if is_long {
                // Long passes — mix of driven and arced
                if skill_influenced_random < 0.25 {
                    TrajectoryType::LowDriven
                } else if skill_influenced_random < 0.70 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            } else {
                // Very long passes — always lofted
                if skill_influenced_random < 0.40 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
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

        if horizontal_distance <= 20.0 {
            // Short passes - mostly ground, slight lift allowed
            0.5 + (long_pass_ability * 0.3)
        } else if horizontal_distance <= 45.0 {
            // Medium passes - driven or low lofted
            1.2 + (long_pass_ability * 0.8)
        } else if horizontal_distance <= 80.0 {
            // Long passes - can be lofted over defenders
            2.5 + (long_pass_ability * 1.5)
        } else if horizontal_distance <= 150.0 {
            // Very long passes - proper arcs
            4.0 + (long_pass_ability * 2.0)
        } else if horizontal_distance <= 250.0 {
            // Ultra-long diagonal switches
            6.0 + (long_pass_ability * 3.0)
        } else {
            // Extreme distance - goalkeeper goal kicks, clearances
            8.0 + (long_pass_ability * 4.0)
        }
    }

    /// Records a possession gain on the claimant's team coach if this
    /// claim represents a team switch (previous owner was on the other
    /// team, or there was no previous owner). Used by `MatchCoach::can_shoot`
    /// to gate shots behind a build-up window.
    fn record_team_possession_if_switch(
        claimant_id: u32,
        field: &MatchField,
        context: &mut MatchContext,
    ) {
        let claimant_team = match field.players.iter().find(|p| p.id == claimant_id) {
            Some(p) => p.team_id,
            None => return,
        };
        let previous_team = field.ball.previous_owner
            .and_then(|pid| field.players.iter().find(|p| p.id == pid))
            .map(|p| p.team_id);
        let switched = previous_team.map_or(true, |pt| pt != claimant_team);
        if switched {
            let tick = context.current_tick();
            context.coach_for_team_mut(claimant_team).record_possession_gain(tick);
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

        // No current owner - normal claim.
        //
        // Pass-accuracy accounting: if the ball is within an active
        // pass window (`pending_pass_passer` set by the pass emit and
        // not yet cleared by an opponent touch), and this claimant is
        // a teammate of that passer, credit the pass as completed.
        // Using `pending_pass_passer` instead of `pass_target_player_id`
        // because the target flag gets cleared in many unrelated paths
        // (set-pieces, clearances, save handoffs) and was masking
        // legitimate same-team receptions. The dedicated passer flag
        // persists through the real pass window (~150 ticks).
        if let Some(passer_id) = field.ball.pending_pass_passer {
            let same_team = field.players.iter()
                .find(|p| p.id == player_id)
                .and_then(|claimant| {
                    field.players.iter()
                        .find(|p| p.id == passer_id)
                        .map(|passer| claimant.team_id == passer.team_id)
                })
                .unwrap_or(false);
            if same_team && passer_id != player_id {
                if let Some(passer) = field.get_player_mut(passer_id) {
                    passer.statistics.passes_completed += 1;
                }
                field.ball.pending_pass_passer = None;
            } else if !same_team {
                // Opponent won the pass — accuracy window ends.
                field.ball.pending_pass_passer = None;
            }
        }
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
        Self::secure_ball_for(player_id, field);
        field.ball.clear_pass_history();
        field.ball.flags.in_flight_state = 100;
    }

    // Snaps the ball to the winner's feet and zeros velocity — prevents
    // residual velocity from carrying it into the winner's own goal
    // after a tackle/interception/block.
    fn secure_ball_for(player_id: u32, field: &mut MatchField) {
        if let Some(player) = field.players.iter().find(|p| p.id == player_id) {
            field.ball.position = player.position;
            field.ball.position.z = 0.0;
        }
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.pass_target_player_id = None;
        field.ball.velocity = nalgebra::Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        field.ball.cached_shot_target = None;
    }

    fn handle_shoot_event(shoot_event_model: ShootingEventContext, field: &mut MatchField) {
        const GOAL_WIDTH: f32 = 29.0; // Half-width of goal in game units (matches engine GOAL_WIDTH)
        #[allow(dead_code)]
        const GOAL_HEIGHT: f32 = 8.0; // Height of crossbar
        // Shot velocity cap. Field is 840u = 105m, so 1u = 0.125m, and at
        // 100-tick/s simulation a cap of 5.6 u/tick meant 70 m/s (~252 km/h)
        // — about 2× the real-world top shot speed (~35 m/s = 2.8 u/tick).
        // Keepers covering 1.16 u/tick laterally had only 9 ticks of shot
        // flight from 50u out (~10u of coverage) against a 29u half-goal:
        // any shot aimed outside the central 15u bypassed the save check.
        // Calibrated to real-world peak shot speed — elite piledrivers now
        // arrive in ~18 ticks instead of ~9, matching the ~3.75× shot/player
        // speed ratio observed in real football (vs. the engine's prior ~10×).
        const MAX_SHOT_VELOCITY: f32 = 3.2;
        const MIN_SHOT_DISTANCE: f32 = 1.0; // Minimum distance to prevent NaN from normalization

        let mut rng = rand::rng();

        // Get player skills for power and accuracy calculations
        let player = field.get_player(shoot_event_model.from_player_id).unwrap();

        // Compressed scaling above skill 17 — elite strikers still
        // have an edge, but the Finishing-20 "generational talent" isn't
        // 5% better than a Finishing-17 top-flight striker at every
        // shot. Without this dampener a single elite finisher in a
        // real-data squad ran at ~1.7 goals/match (Vlahović line),
        // roughly 2× real-world top-scorer rates. Linear 0-17 → 0-0.85,
        // then compressed 17-20 → 0.85-0.94.
        let compress_high = |raw: f32| -> f32 {
            if raw > 17.0 {
                (0.85 + (raw - 17.0) * 0.03).min(0.94)
            } else {
                (raw / 20.0).clamp(0.1, 1.0)
            }
        };
        let finishing_skill = compress_high(player.skills.technical.finishing);
        let technique_skill = compress_high(player.skills.technical.technique);
        let long_shot_skill = compress_high(player.skills.technical.long_shots);
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

        // Calculate overall shooting accuracy (0.0 to 1.0).
        // Squared skill terms steepen the curve so mediocre finishers (skill 6-9)
        // are genuinely inaccurate even at close range. Without this, a fast
        // striker with Finishing 8 converts at ~elite rates simply by getting
        // into position — linear weighting flattens the skill gap too much.
        let base_accuracy = if horizontal_distance > 100.0 {
            // Long shots depend more on long_shot skill and technique
            (long_shot_skill.powi(2) * 0.5
                + technique_skill.powi(2) * 0.3
                + finishing_skill.powi(2) * 0.2)
                * composure_skill
                * 0.85
        } else if horizontal_distance > 50.0 {
            // Medium range - balanced
            (finishing_skill.powi(2) * 0.4
                + technique_skill.powi(2) * 0.3
                + long_shot_skill.powi(2) * 0.3)
                * composure_skill
                * 0.90
        } else {
            // Close range - finishing is key, but still imperfect
            (finishing_skill.powi(2) * 0.55
                + technique_skill.powi(2) * 0.2
                + composure_skill.powi(2) * 0.25)
                * 0.92
        };

        // Calculate target point within goal — skill-weighted.
        // Elite finishers deliberately aim for the corners where the GK
        // can't reach; poor finishers shoot central ("hit the keeper")
        // far more often. Real per-shot placement data (StatsBomb,
        // Opta) shows ~70% of elite-striker shots target the corners
        // and ~50% of replacement-level shots end up central-ish.
        //
        // `central_rate` scales inversely with finishing + decisions:
        //   finishing 1.0, decisions 1.0 → ~12% central shots
        //   finishing 0.3, decisions 0.3 → ~55% central shots
        let quality = (finishing_skill + decisions_skill) * 0.5;
        let central_rate = (0.60 - quality * 0.50).clamp(0.10, 0.55);
        let side_rate = (1.0 - central_rate) * 0.5;
        let target_preference = rng.random_range(0.0..1.0);
        // Corner placement depth — elite players find the post;
        // poor players land central even when "aiming" side.
        let corner_reach = GOAL_WIDTH * (0.55 + decisions_skill * 0.35);
        let ideal_y_target = if target_preference < side_rate {
            goal_center.y - corner_reach
        } else if target_preference < side_rate * 2.0 {
            goal_center.y + corner_reach
        } else {
            goal_center.y + rng.random_range(-GOAL_WIDTH * 0.3..GOAL_WIDTH * 0.3)
        };

        // Add shooting error based on skills and distance
        // Error increases with distance and decreases with skill
        let distance_error_factor = (horizontal_distance / 80.0).clamp(0.8, 3.0);

        // Calculate positional error (how far from intended target)
        // Distance penalty multiplier for base_accuracy — close range should be very accurate
        let distance_penalty = if horizontal_distance > 200.0 {
            0.10
        } else if horizontal_distance > 150.0 {
            0.16
        } else if horizontal_distance > 100.0 {
            0.26
        } else if horizontal_distance > 70.0 {
            0.42
        } else if horizontal_distance > 50.0 {
            0.58
        } else if horizontal_distance > 30.0 {
            0.74
        } else if horizontal_distance > 15.0 {
            0.85
        } else {
            0.92
        };
        let adjusted_accuracy = base_accuracy * distance_penalty;

        // Base error: elite close-range ±2-5 units, poor long-range ±20-40 units.
        // Goal half-width is 29u — a y-error spread larger than that
        // pushes most shots wide regardless of where they were aimed,
        // so prior 50× multiplier (close avg-player ±20, long ±35)
        // dominated the on-target outcome and held the rate at ~19%.
        // Cut to 30× and tightened mins so accuracy → on-target rate
        // tracks more linearly toward the real ~33%.
        let base_position_error = 30.0 * distance_error_factor * (1.0 - adjusted_accuracy);
        let min_error = if horizontal_distance < 30.0 { 2.0 } else if horizontal_distance < 60.0 { 4.0 } else { 8.0 };
        let max_y_error = base_position_error.clamp(min_error, 60.0);

        // Add random error to y-coordinate
        let y_error = rng.random_range(-max_y_error..max_y_error);
        let mut actual_y_target = ideal_y_target + y_error;

        // Wide miss chance: distance-dependent. Calibrated to real-
        // football shot data — Opta/Statsbomb have ~33% of shots on
        // target overall. After the prior cut (0.06/0.12/0.20/0.28 →
        // 0.04/0.08/0.14/0.20) on-target landed at ~19% — still well
        // below real. The y-error noise (line ~1248) already
        // randomises around the aimed point; the wide_miss_chance
        // is an additional "force the ball off-frame" path that was
        // adding ~25-30% of misses on top. Halving bases again and
        // pulling scaling 0.22 → 0.14 should move on-target toward
        // 30%.
        let wide_miss_base = if horizontal_distance < 30.0 {
            0.025
        } else if horizontal_distance < 60.0 {
            0.05
        } else if horizontal_distance < 100.0 {
            0.09
        } else {
            0.14
        };
        let wide_miss_chance = (1.0 - adjusted_accuracy) * 0.14 + wide_miss_base;
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

        // Over-the-bar miss chance: distance-dependent. Real football
        // data: close-range shots go over ~5%, long range ~20%. Earlier
        // 0.04/0.08/0.14/0.20 bases + 0.20 accuracy scaling were OK when
        // over-bar shots were silently counted on-target (the counting
        // bug just fixed). Now that over-bar shots correctly do nothing,
        // the same bases produce too many accuracy-less misses. Cut
        // bases ~40% and scaling 0.20 → 0.12 so the on-target rate can
        // recover toward the real ~33%.
        let over_bar_base = if horizontal_distance < 30.0 {
            0.025
        } else if horizontal_distance < 60.0 {
            0.05
        } else if horizontal_distance < 100.0 {
            0.09
        } else {
            0.13
        };
        let over_bar_chance = (1.0 - adjusted_accuracy) * 0.12 + over_bar_base;
        let shot_goes_over_bar = rng.random_range(0.0f32..1.0) < over_bar_chance;
        let z_velocity = if shot_goes_over_bar {
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

        // Record shot in player memory. A shot counts as "on target" only
        // if the frame of the goal is actually threatened — the y-aim lies
        // between the posts AND we didn't just roll "over the bar" above.
        // The over-bar branch sets z-velocity to 3.0-6.0 (guaranteed clear
        // of crossbar at 8.0), so those shots fly into the stands. Before
        // this check, `on_target` counted them because it only looked at y,
        // producing a ~49% conversion leak where on-target shots were
        // neither saved nor scored — they just sailed over, off-screen.
        let on_target = clamped_y_target >= goal_left_post
            && clamped_y_target <= goal_right_post
            && !shot_goes_over_bar;
        if let Some(shooter) = field.get_player_mut(shoot_event_model.from_player_id) {
            // Quick xG from distance + finishing skill. Full-context xG with
            // pressure/angle lives in ShotQualityEvaluator (player strategy
            // layer) — that's the right place to compute a richer value when
            // plumbing exposes the state context here.
            let xg = {
                let d = horizontal_distance;
                // Calibrated to real-world xG curves: ~0.55 at 6yd, ~0.08 at 20yd
                let distance_factor = if d <= 10.0 {
                    0.55
                } else if d <= 30.0 {
                    0.55 - (d - 10.0) / 20.0 * 0.30
                } else if d <= 60.0 {
                    0.25 - (d - 30.0) / 30.0 * 0.18
                } else if d <= 120.0 {
                    0.07 - (d - 60.0) / 60.0 * 0.05
                } else {
                    0.02
                };
                let finishing = (shooter.skills.technical.finishing / 20.0).clamp(0.0, 1.0);
                let skill_mult = 0.7 + finishing * 0.6; // 0.7 .. 1.3
                // Off-target shots are misses — don't credit xG
                let target_mult = if on_target { 1.0 } else { 0.15 };
                (distance_factor * skill_mult * target_mult).clamp(0.0, 0.90)
            };
            shooter.memory.record_shot(shoot_event_model.tick, on_target);
            shooter.memory.record_shot_xg(shoot_event_model.tick, xg);
        }

        field.ball.previous_owner = Some(shoot_event_model.from_player_id);
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;
        field.ball.velocity = final_velocity;

        // Shorter flight protection for shots — allows defenders/GK to claim sooner
        field.ball.flags.in_flight_state = 40;

        // Project where the ball will cross the goal line so the
        // defending keeper can commit to an intercept line rather than
        // chasing the ball's current position every tick. Uses
        // constant-velocity projection (close enough over the ~10-20
        // ticks of shot flight; drag and gravity effects are small
        // relative to the error the goalkeeper's own reaction
        // introduces). Without this cache the GK loses ground to the
        // 5.6 u/tick shot and never catches anything — the primary
        // reason saves/on-target sat below 1% in benchmarks.
        let shooter_side = field
            .get_player(shoot_event_model.from_player_id)
            .and_then(|p| p.side);
        if let Some(shooter_side) = shooter_side {
            let field_width = field.size.width as f32;
            let defending_side = match shooter_side {
                PlayerSide::Left => PlayerSide::Right,
                PlayerSide::Right => PlayerSide::Left,
            };
            let goal_line_x = match defending_side {
                PlayerSide::Left => 0.0,
                PlayerSide::Right => field_width,
            };
            let dx = goal_line_x - field.ball.position.x;
            let vx = final_velocity.x;
            // Only cache if the shot is actually heading toward that
            // goal — otherwise projection time is negative or infinite
            // and the cache would mislead the keeper.
            if (dx > 0.0 && vx > 0.1) || (dx < 0.0 && vx < -0.1) {
                let ticks_to_goal = (dx / vx).abs();
                let goal_line_y = field.ball.position.y + final_velocity.y * ticks_to_goal;
                // Arc approximation: z under gravity (~0.157 u/tick² from
                // update_velocity's 9.81 * 0.016 scaling).
                let goal_line_z = (field.ball.position.z
                    + final_velocity.z * ticks_to_goal
                    - 0.5 * 0.157 * ticks_to_goal * ticks_to_goal)
                    .max(0.0);
                field.ball.cached_shot_target = Some(ShotTarget {
                    goal_line_y,
                    goal_line_z,
                    defending_side,
                });
            } else {
                field.ball.cached_shot_target = None;
            }
        }
    }

    /// Credit a goalkeeper save for a parried shot — the GK touched the
    /// ball mid-flight but couldn't claim it cleanly. Emitted from the
    /// diving and catching states when they exit on "ball moving away"
    /// while `cached_shot_target` was set. The same `cached_shot_target`
    /// is then cleared so the eventual rest position (out of bounds,
    /// to a defender, or back into play) doesn't double-credit anyone.
    fn handle_parried_ball_event(player_id: u32, field: &mut MatchField) {
        // Only credit when the ball was a real shot — guards against
        // the diving state calling this when the GK gave up on a long
        // pass. The state-machine emitters are gated on the same flag,
        // so this is belt-and-braces.
        if field.ball.cached_shot_target.is_none() {
            return;
        }
        let shooter_id = field.ball.previous_owner;
        if let Some(gk) = field.get_player_mut(player_id) {
            gk.statistics.saves += 1;
            gk.statistics.shots_faced += 1;
        }
        // Credit on-target to the shooter — a parry IS the keeper
        // touching a shot that reached the goal frame. Without this,
        // saves > on-target shots, an impossible ratio.
        if let Some(sid) = shooter_id {
            if let Some(shooter) = field.get_player_mut(sid) {
                shooter.memory.credit_shot_on_target();
            }
        }
        field.ball.cached_shot_target = None;
    }

    fn handle_caught_ball_event(player_id: u32, field: &mut MatchField) {
        // Detect saves: ball was moving and came from an opponent
        let ball_was_moving = field.ball.velocity.norm_squared() > 0.25;
        let last_owner_team = field.ball.previous_owner
            .and_then(|prev_id| field.players.iter().find(|p| p.id == prev_id).map(|p| p.team_id));
        let gk_team = field.players.iter().find(|p| p.id == player_id).map(|p| p.team_id);

        // Save credit requires both: the ball was moving from an opponent
        // AND the catch resolves a real shot (cached_shot_target set).
        // Without the shot gate, every cross / through-ball / clearance
        // that ends in the keeper's hands counted as a save — pushing
        // saves/on-target above 100% (more "saves" than on-target shots).
        if ball_was_moving && last_owner_team.is_some() && last_owner_team != gk_team {
            let was_shot = field.ball.cached_shot_target.is_some();
            if was_shot {
                let shooter_id = field.ball.previous_owner;
                if let Some(player) = field.get_player_mut(player_id) {
                    player.statistics.saves += 1;
                    player.statistics.shots_faced += 1;
                }
                if let Some(sid) = shooter_id {
                    if let Some(shooter) = field.get_player_mut(sid) {
                        shooter.memory.credit_shot_on_target();
                    }
                }
            }
        }

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        // Ball must stop when caught — prevent it from continuing into the goal
        field.ball.velocity = Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        // GK holds ball in hands — long protection so opponents can't challenge.
        // Covers holding (25-60 ticks) + distribution time.
        field.ball.claim_cooldown = 200;
        field.ball.pass_target_player_id = None;
        // Shot is dead — clear the projected intercept so the keeper
        // doesn't keep chasing a ghost target next tick.
        field.ball.cached_shot_target = None;
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

    fn handle_commit_foul_event(
        fouler_id: u32,
        severity: FoulSeverity,
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        // Free-kick protection: the victim gets time without being challenged.
        if field.ball.current_owner.is_some() {
            field.ball.claim_cooldown = 150; // ~2.5 seconds of protection
            field.ball.flags.in_flight_state = 150;
            field.ball.contested_claim_count = 0;
        }

        let match_second = context.total_match_time;

        // Card decision — probability scales with severity and the fouler's
        // aggression/dirtiness. Composure reduces the chance a little
        // (cool-headed players get the benefit of the doubt).
        let (card_yellow_prob, card_red_prob) = {
            let player = match field.get_player_mut(fouler_id) {
                Some(p) => p,
                None => return,
            };

            // Count the foul itself whether or not it draws a card.
            player.fouls_committed = player.fouls_committed.saturating_add(1);
            player.statistics.add_foul(match_second);

            let aggression = player.skills.mental.aggression / 20.0;
            let composure = player.skills.mental.composure / 20.0;
            let teamwork = player.skills.mental.teamwork / 20.0;
            // Personality contributions — these used to be generated-only.
            // Dirtiness = how hard/cynical the challenges are; temperament
            // = how likely the player is to snap under provocation;
            // sportsmanship is a damper pulling the other way.
            let dirtiness = player.attributes.dirtiness / 20.0;
            let temperament = player.attributes.temperament / 20.0;
            let sportsmanship = player.attributes.sportsmanship / 20.0;

            // Persistent fouler escalation — 3+ fouls = next one much more likely booked.
            let persistent = if player.fouls_committed >= 3 { 0.15 } else { 0.0 };

            // High-aggression, low-composure, low-teamwork = "dirty" player.
            // Layer personality on top: dirtiness pushes cards up, sportsmanship
            // pulls them down, low temperament punishes you under pressure.
            let aggressor_factor = (aggression * 0.40
                - composure * 0.12
                - teamwork * 0.08
                + dirtiness * 0.18
                + (1.0 - temperament) * 0.10
                - sportsmanship * 0.10)
                .clamp(-0.25, 0.70);

            match severity {
                FoulSeverity::Normal => (
                    (0.12 + aggressor_factor * 0.20 + persistent).clamp(0.02, 0.55),
                    0.0_f32,
                ),
                FoulSeverity::Reckless => (
                    (0.55 + aggressor_factor * 0.25 + persistent).clamp(0.20, 0.92),
                    (0.08 + aggressor_factor * 0.15).clamp(0.01, 0.40),
                ),
                FoulSeverity::Violent => (0.0_f32, 1.0_f32),
            }
        };

        let mut rng = rand::rng();
        let roll_red = rng.random::<f32>();
        let roll_yellow = rng.random::<f32>();

        let direct_red = roll_red < card_red_prob;
        let got_yellow = !direct_red && roll_yellow < card_yellow_prob;

        if !direct_red && !got_yellow {
            return;
        }

        // Red cards disabled: the tackle/foul pipeline currently fires
        // far more often than real-world rates, which cascaded into
        // multiple sent-off players per match and left the viewer with
        // half-empty teams. Until foul frequency is properly calibrated,
        // no player gets sent off — direct red or second yellow both
        // degrade to a yellow caution. `is_sent_off` stays false so the
        // position recorder keeps the player in the viewer.
        let (second_yellow, ends_with_red) = {
            let player = match field.get_player_mut(fouler_id) {
                Some(p) => p,
                None => return,
            };
            player.yellow_cards = player.yellow_cards.saturating_add(1);
            player.statistics.add_yellow_card(match_second);
            context.record_stoppage_time(15_000);
            let _ = direct_red;
            (false, false)
        };

        if ends_with_red {
            // Transfer ball ownership back to a neutral state so the
            // opposing side can restart. Zero the fouler's velocity and
            // park them off the pitch so distance / collision checks stop
            // treating them as an active participant.
            if field.ball.current_owner == Some(fouler_id) {
                field.ball.previous_owner = field.ball.current_owner;
                field.ball.current_owner = None;
                field.ball.pass_target_player_id = None;
            }

            // Capture team id before we stash the player off-pitch so we
            // can reshape teammates afterwards.
            let team_id = field
                .get_player_mut(fouler_id)
                .map(|p| p.team_id);

            if let Some(player) = field.get_player_mut(fouler_id) {
                player.velocity = Vector3::zeros();
                // Stash them well beyond the sideline. Physics updates
                // still run but no one is close enough to interact.
                player.position = Vector3::new(-500.0, -500.0, 0.0);
            }

            // Compact the surviving team's shape — surviving players drop
            // deeper and narrower to cover the numerical disadvantage.
            if let Some(tid) = team_id {
                field.compact_after_dismissal(tid);
            }

            if second_yellow {
                debug!(
                    "Second yellow → red: player {} at {}s (severity {:?})",
                    fouler_id, match_second, severity
                );
            } else {
                debug!(
                    "Direct red: player {} at {}s (severity {:?})",
                    fouler_id, match_second, severity
                );
            }
        } else {
            debug!(
                "Yellow card: player {} at {}s (severity {:?})",
                fouler_id, match_second, severity
            );
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

    /// Identify the goalkeeper who is about to clear a shot, if any.
    /// Returns `Some(gk_id)` when the current ball owner is a GK *and*
    /// the ball is mid-flight from a real shot (`cached_shot_target` set).
    /// Used to credit punches/parries as saves — the existing path only
    /// credited catches via `handle_caught_ball_event`.
    fn gk_clearing_shot(field: &MatchField) -> Option<u32> {
        let clearer_id = field.ball.current_owner?;
        if field.ball.cached_shot_target.is_none() {
            return None;
        }
        // Iterate directly because `MatchField::get_player` takes `&mut
        // self`; we only need a read here and want to keep the borrow
        // immutable so the caller can re-borrow `field` mutably afterward.
        let clearer = field.players.iter().find(|p| p.id == clearer_id)?;
        if clearer
            .tactical_position
            .current_position
            .position_group()
            == PlayerFieldPositionGroup::Goalkeeper
        {
            Some(clearer_id)
        } else {
            None
        }
    }

    fn handle_clear_ball_event(velocity: Vector3<f32>, field: &mut MatchField) {
        // Punches / dive-parries from a shot: the GK touched a real
        // attempt-on-goal and steered it away. The catching path
        // (handle_caught_ball_event) already credits saves; this is the
        // companion that closes the long-standing gap where punched and
        // parried shots stayed at zero saves regardless of effort.
        let gk_save_id = Self::gk_clearing_shot(field);
        if let Some(gk_id) = gk_save_id {
            // Capture shooter BEFORE we mutate the field — the previous
            // owner is the player whose shot the GK is now clearing.
            let shooter_id = field.ball.previous_owner
                .filter(|&sid| sid != gk_id);
            if let Some(gk) = field.get_player_mut(gk_id) {
                gk.statistics.saves += 1;
                gk.statistics.shots_faced += 1;
            }
            if let Some(sid) = shooter_id {
                if let Some(shooter) = field.get_player_mut(sid) {
                    shooter.memory.credit_shot_on_target();
                }
            }
            field.ball.cached_shot_target = None;
        }

        // Clearance cap. Needs more headroom than a pass because a
        // clearance is typically lofted — horizontal AND vertical
        // components are both meaningful, so the total magnitude
        // (sqrt(hx² + hy² + vz²)) legitimately exceeds a flat pass.
        // In-engine gravity is strong (balls fall fast), so a proper
        // hoof needs ~5 u/tick each of horizontal and vertical to
        // travel 30-40m before landing. 7.0 total covers that with a
        // little slack. Still well below the global MAX_VELOCITY safety.
        const MAX_CLEAR_VELOCITY: f32 = 7.0;
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
