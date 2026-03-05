use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

// Shooting distance constants for midfielders
const MAX_SHOOTING_DISTANCE: f32 = 80.0; // Midfielders rarely shoot from beyond ~40m
const STANDARD_SHOOTING_DISTANCE: f32 = 55.0; // Standard shooting range for midfielders
const PRESSURE_CHECK_DISTANCE: f32 = 10.0; // Distance to check for opponent pressure before shooting
const POINT_BLANK_DISTANCE: f32 = 20.0; // ~10m - must shoot, goalkeeper is right there
const MIN_SHOOTING_DISTANCE: f32 = 5.0;

#[derive(Default, Clone)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // Priority 0: Point-blank range - MUST shoot regardless of clear shot check
            // This prevents players from colliding with goalkeeper instead of shooting
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            if distance_to_goal <= POINT_BLANK_DISTANCE && distance_to_goal > MIN_SHOOTING_DISTANCE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            // Priority: Clear ball if congested anywhere (not just boundaries)
            // Allow emergency clearances even without stable possession
            // Cooldown: only attempt every 30 ticks to prevent claim-pass-reclaim loops
            if (self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested())
                && ctx.in_state_time % 30 < 2
            {
                // Try to find a good pass option first using the standard evaluator
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    let dist = (target_teammate.position - ctx.player.position).magnitude();
                    // Only pass if target is far enough away to escape congestion
                    if dist > 40.0 {
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(target_teammate.id)
                                    .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_BEST")
                                    .build(ctx),
                            )),
                        ));
                    }
                }

                // Fallback: find teammate at least 40 units away (outside congestion zone)
                if let Some(target_teammate) = ctx.players().teammates().nearby(100.0)
                    .filter(|t| (t.position - ctx.player.position).magnitude() > 40.0)
                    .next()
                {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_NEARBY")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Shooting evaluation for midfielders
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let long_shots = ctx.player.skills.technical.long_shots / 20.0;
            let finishing = ctx.player.skills.technical.finishing / 20.0;

            // Standard shooting - close enough with clear shot and good skill
            // Also check that player is not heavily marked
            if goal_dist <= STANDARD_SHOOTING_DISTANCE
                && ctx.player().has_clear_shot()
                && finishing > 0.65
                && ctx.players().opponents().nearby(8.0).count() < 2 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            // Distance shooting - long range with excellent skills and no pressure
            if goal_dist <= MAX_SHOOTING_DISTANCE
                && ctx.player().has_clear_shot()
                && long_shots > 0.65
                && finishing > 0.55
                && !ctx.players().opponents().exists(PRESSURE_CHECK_DISTANCE) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            // Minimum carry time before considering passes — let midfielders run with the ball
            let ownership_ticks = ctx.tick_context.ball.ownership_duration;

            // COUNTER-ATTACK: Quick transition but not instant — need a few ticks to assess
            if ownership_ticks > 8 && ctx.ball().has_stable_possession()
                && self.is_counter_attack_opportunity(ctx)
            {
                if let Some(forward_target) = self.find_counter_attack_pass(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(forward_target.id)
                                .with_reason("MID_RUNNING_COUNTER_ATTACK")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // ONE-TWO COMBINATION: After carrying briefly, check if passer has run ahead
            // into space — return the ball for a wall-pass / give-and-go
            if ownership_ticks >= 10 && ownership_ticks <= 30 && ctx.ball().has_stable_possession() {
                if let Some(return_target) = self.find_one_two_return(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(return_target.id)
                                .with_reason("MID_RUNNING_ONE_TWO_RETURN")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // DRAW AND RELEASE: If opponent is committing to tackle, draw them in
            // then pass to space they vacated — requires carrying to draw them
            if ownership_ticks > 30 && ctx.ball().has_stable_possession() {
                if let Some(release_target) = self.find_draw_and_release_pass(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(release_target.id)
                                .with_reason("MID_RUNNING_DRAW_AND_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // CROSSING: Wide midfielder in attacking third with teammates in the box
            if ownership_ticks > 20 && ctx.ball().has_stable_possession()
                && self.should_cross(ctx)
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Crossing,
                ));
            }

            // Enhanced passing decision — carry the ball before looking for a pass
            if ownership_ticks > 40 && ctx.ball().has_stable_possession()
                && self.should_pass(ctx)
            {
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_SHOULD_PASS")
                                .build(ctx),
                        )),
                    ));
                }
            }
        } else {
            // Without ball - check for opponent with ball first (highest priority)
            // CRITICAL: Tackle opponent with ball if close enough
            if let Some(opponent) = ctx.players().opponents().nearby(150.0).with_ball(ctx).next() {
                let opponent_distance = (opponent.position - ctx.player.position).magnitude();

                // If opponent with ball is close, tackle immediately
                if opponent_distance < 40.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }

                // If opponent with ball is nearby, press them aggressively
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // Teammate has the ball — don't chase it, support via movement instead
            if ctx.team().is_control_ball() {
                // Stay in Running — velocity() handles attacking support positioning
                return None;
            }

            // Emergency: if ball is nearby, slow/stopped, and unowned, go for it
            // But only if this player is the nearest teammate to prevent mass-chasing
            if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
                let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
                if ball_velocity < 3.0 {
                    let ball_pos = ctx.tick_context.positions.ball.position;
                    let my_dist = ctx.ball().distance();
                    let closer_teammate = ctx.players().teammates().all()
                        .any(|t| t.id != ctx.player.id && (t.position - ball_pos).magnitude() < my_dist - 5.0);

                    if !closer_teammate {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::TakeBall,
                        ));
                    }
                }
            }

            // Notification system: if ball system notified us to take the ball, act immediately
            if ctx.ball().should_take_ball_immediately() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            if ctx.ball().distance() < 30.0 && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // Guard unmarked attackers on our side when we can't press/intercept
            if ctx.ball().on_own_side() && ctx.ball().distance() > 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Guarding,
                ));
            }
        }

        // ANTI-OSCILLATION: If carrying ball too long without acting, force a decision
        // POSSESSION RETENTION: Allow longer holding when team is comfortable
        let anti_oscillation_threshold = if self.should_retain_possession(ctx) { 250 } else { 150 };
        if ctx.player.has_ball(ctx) && ctx.in_state_time > anti_oscillation_threshold {
            // Prefer passing first
            if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION")
                            .build(ctx),
                    )),
                ));
            }
            // Only shoot as fallback at point-blank range with clear shot
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            if distance_to_goal < 25.0 && ctx.player().has_clear_shot() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }
            // Last resort: pass to any nearby teammate ahead of the ball (toward opponent goal)
            let player_pos = ctx.player.position;
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - player_pos).normalize();
            if let Some(target_teammate) = ctx.players().teammates().nearby(200.0)
                .filter(|t| {
                    let to_teammate = (t.position - player_pos).normalize();
                    to_teammate.dot(&to_goal) > 0.0 // Teammate is ahead (toward opponent goal)
                })
                .next()
            {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION_FALLBACK")
                            .build(ctx),
                    )),
                ));
            }
            // Absolute last resort: pass to any nearby teammate (even backward)
            if let Some(target_teammate) = ctx.players().teammates().nearby(200.0).next() {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION_FALLBACK_ANY")
                            .build(ctx),
                    )),
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Simplified waypoint following
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();
            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 5.0,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                );
            }
        }

        if ctx.player.has_ball(ctx) {
            // POSSESSION RETENTION: When in control mode, move slower with more lateral sway
            // to keep ball and tire opponents instead of always charging forward
            if self.should_retain_possession(ctx) {
                Some(self.calculate_possession_retention_movement(ctx))
            } else {
                Some(self.calculate_simple_ball_movement(ctx))
            }
        } else {
            // Single smooth target per player — no binary is_control_ball() switch
            // which flickers and causes twitching / conflicting velocity directions
            let ball_pos = ctx.tick_context.positions.ball.position;
            let start_pos = ctx.player.start_position;
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;
            let ball_distance = ctx.ball().distance();

            let attacking_direction = match ctx.player.side {
                Some(crate::r#match::PlayerSide::Left) => 1.0,
                Some(crate::r#match::PlayerSide::Right) => -1.0,
                None => 0.0,
            };

            // Smooth ball proximity: 0.0 (very far) → 0.7 (right at ball)
            // Continuous function of distance — no flickering
            let proximity = (1.0 - ball_distance / 350.0).clamp(0.05, 0.7);

            let center_y = field_height / 2.0;
            let is_wide = (start_pos.y - center_y).abs() > field_height * 0.2;

            // X: shift from tactical start toward a support position ahead of ball
            let support_x = ball_pos.x + attacking_direction * 35.0;
            // Wide players stagger forward to break flat lines
            let width_stagger = if is_wide { attacking_direction * 15.0 } else { 0.0 };
            let target_x = start_pos.x + (support_x - start_pos.x) * proximity + width_stagger;

            // Y: anchor to each player's unique tactical Y, attracted toward ball Y
            // Wide players hold width more; central players track ball more
            let y_attraction = if is_wide { proximity * 0.15 } else { proximity * 0.35 };
            let target_y = start_pos.y + (ball_pos.y - start_pos.y) * y_attraction;

            // Per-player organic drift using stable match time (never resets)
            let match_time = ctx.context.total_match_time as f32;
            let player_seed = ctx.player.id as f32 * 2.39;
            let drift_x = (player_seed + match_time * 0.004).sin() * 10.0;
            let drift_y = (player_seed * 1.37 + match_time * 0.003).cos() * 8.0;

            let target = Vector3::new(
                (target_x + drift_x).clamp(30.0, field_width - 30.0),
                (target_y + drift_y).clamp(30.0, field_height - 30.0),
                0.0,
            );

            Some(
                SteeringBehavior::Arrive {
                    target,
                    slowing_distance: 25.0,
                }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity(),
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Midfielders cover the most ground during a match - box to box running
        // High intensity with velocity-based adjustment
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderRunningState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(ctx, 300.0)
    }

    /// Simplified ball carrying movement
    fn calculate_simple_ball_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;

        // Simple decision: move toward goal with slight variation
        let to_goal = (goal_pos - player_pos).normalize();

        // Smooth sinusoidal lateral sway instead of binary flip
        let phase = (ctx.in_state_time as f32) * std::f32::consts::TAU / 60.0;
        let sway = phase.sin() * 0.2;
        let lateral = Vector3::new(-to_goal.y * sway, to_goal.x * sway, 0.0);

        let target = player_pos + (to_goal + lateral).normalize() * 40.0;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }


    /// Enhanced passing decision that considers player skills and pressing intensity
    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Get player skills
        let vision = ctx.player.skills.mental.vision / 20.0;
        let passing = ctx.player.skills.technical.passing / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let teamwork = ctx.player.skills.mental.teamwork / 20.0;

        // Assess pressing situation
        let pressing_intensity = self.calculate_pressing_intensity(ctx);
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // 1. MUST PASS: Heavy pressing (multiple opponents very close)
        if pressing_intensity > 0.7 {
            // Even low-skilled players should pass under heavy pressure
            return passing > 0.3 || composure < 0.5;
        }

        // 2. FORCED PASS: Under moderate pressure with limited skills
        if pressing_intensity > 0.5 && (passing < 0.6 || composure < 0.6) {
            return true;
        }

        // 3. TACTICAL PASS: Skilled players looking for opportunities
        // Players with high vision and passing can spot good passes even without pressure
        if vision > 0.7 && passing > 0.7 {
            // Check if there's a better-positioned teammate
            if self.has_better_positioned_teammate(ctx, distance_to_goal) {
                return true;
            }
        }

        // 4. TEAM PLAY: High teamwork players distribute more
        if teamwork > 0.7 && decisions > 0.6 && pressing_intensity > 0.3 {
            // Midfielders with good teamwork pass to maintain possession and tempo
            return self.find_best_pass_option(ctx).is_some();
        }

        // 5. UNDER LIGHT PRESSURE: Decide based on skills and options
        if pressing_intensity > 0.3 {
            // Better decision-makers are more likely to pass when slightly pressed
            let pass_likelihood = (decisions * 0.4) + (vision * 0.3) + (passing * 0.3);
            return pass_likelihood > 0.6;
        }

        // 6. NO PRESSURE: Continue running unless very close to goal
        // Very skilled passers might still look for a pass if in midfield
        if distance_to_goal > 300.0 && vision > 0.8 && passing > 0.8 {
            return self.has_teammate_in_dangerous_position(ctx);
        }

        false
    }

    /// Calculate pressing intensity based on number and proximity of opponents
    fn calculate_pressing_intensity(&self, ctx: &StateProcessingContext) -> f32 {
        let very_close = ctx.players().opponents().nearby(15.0).count() as f32;
        let close = ctx.players().opponents().nearby(30.0).count() as f32;
        let medium = ctx.players().opponents().nearby(50.0).count() as f32;

        // Weight closer opponents more heavily
        let weighted_pressure = (very_close * 0.5) + (close * 0.3) + (medium * 0.1);

        // Normalize to 0-1 range (assuming max 5 opponents can reasonably press)
        (weighted_pressure / 2.0).min(1.0)
    }

    /// Check if there's a teammate in a better position
    fn has_better_positioned_teammate(&self, ctx: &StateProcessingContext, current_distance: f32) -> bool {
        ctx.players()
            .teammates()
            .nearby(300.0)
            .any(|teammate| {
                let teammate_distance = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                let is_closer = teammate_distance < current_distance * 0.8;
                let has_space = ctx.players().opponents().all()
                    .filter(|opp| (opp.position - teammate.position).magnitude() < 15.0)
                    .count() < 2;
                let has_clear_pass = ctx.player().has_clear_pass(teammate.id);

                is_closer && has_space && has_clear_pass
            })
    }

    /// Check if there's a teammate in a dangerous attacking position
    fn has_teammate_in_dangerous_position(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .teammates()
            .nearby(350.0)
            .any(|teammate| {
                // Prefer forwards and attacking midfielders
                let is_attacker = teammate.tactical_positions.is_forward() ||
                                 teammate.tactical_positions.is_midfielder();

                // Check if in attacking third
                let teammate_distance = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                let field_width = ctx.context.field_size.width as f32;
                let in_attacking_third = teammate_distance < field_width * 0.4;

                // Check if in free space
                let in_free_space = ctx.players().opponents().all()
                    .filter(|opp| (opp.position - teammate.position).magnitude() < 12.0)
                    .count() < 2;

                // Check if making a forward run
                let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
                let making_run = teammate_velocity.magnitude() > 1.5 && {
                    let to_goal = ctx.player().opponent_goal_position() - teammate.position;
                    teammate_velocity.normalize().dot(&to_goal.normalize()) > 0.5
                };

                let has_clear_pass = ctx.player().has_clear_pass(teammate.id);

                is_attacker && in_attacking_third && (in_free_space || making_run) && has_clear_pass
            })
    }

    /// ONE-TWO COMBINATION: Check if the player who just passed to us has run into
    /// a better forward position with space. If so, return the ball for a wall-pass.
    fn find_one_two_return<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let recent_passers = &ctx.tick_context.ball.recent_passers;
        // Get the most recent passer (last element in the ring buffer vec)
        let passer_id = *recent_passers.last()?;

        // Passer must be a teammate
        let passer = ctx.context.players.by_id(passer_id)?;
        if passer.team_id != ctx.player.team_id {
            return None;
        }

        // Find passer in nearby players
        let passer_lite = ctx.players().teammates().all()
            .find(|t| t.id == passer_id)?;

        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let passer_pos = passer_lite.position;

        // Passer must now be closer to opponent goal than us (they continued their run)
        let our_goal_dist = (goal_pos - player_pos).magnitude();
        let passer_goal_dist = (goal_pos - passer_pos).magnitude();
        if passer_goal_dist >= our_goal_dist * 0.9 {
            return None; // Passer didn't run ahead enough
        }

        // Passer must be in open space (fewer than 2 opponents within 12 units)
        let opponents_near_passer = ctx.players().opponents().all()
            .filter(|opp| (opp.position - passer_pos).magnitude() < 12.0)
            .count();
        if opponents_near_passer >= 2 {
            return None;
        }

        // Must have clear passing lane back to passer
        if !ctx.player().has_clear_pass(passer_id) {
            return None;
        }

        // Passer must be within reasonable passing distance
        let pass_distance = (passer_pos - player_pos).magnitude();
        if pass_distance > 200.0 || pass_distance < 10.0 {
            return None;
        }

        Some(passer_lite)
    }

    /// DRAW AND RELEASE: Detect an opponent committing to a tackle (approaching fast
    /// within 15-35 units). Find a teammate in the space the opponent is vacating.
    fn find_draw_and_release_pass<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;

        // Find the closest approaching opponent (within 15-35 units, closing in)
        let approaching_opponent = ctx.players().opponents().nearby(35.0)
            .filter(|opp| {
                let dist = (opp.position - player_pos).magnitude();
                if dist < 15.0 || dist > 35.0 { return false; }

                // Check if opponent is moving toward us
                let opp_velocity = ctx.tick_context.positions.players.velocity(opp.id);
                if opp_velocity.magnitude() < 1.0 { return false; }

                let to_us = (player_pos - opp.position).normalize();
                let opp_dir = opp_velocity.normalize();
                opp_dir.dot(&to_us) > 0.6 // Moving toward us
            })
            .min_by(|a, b| {
                let da = (a.position - player_pos).magnitude();
                let db = (b.position - player_pos).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })?;

        // The space the opponent is vacating is roughly behind them (opposite of their movement)
        let opp_velocity = ctx.tick_context.positions.players.velocity(approaching_opponent.id);
        let vacated_zone = approaching_opponent.position - opp_velocity.normalize() * 30.0;

        // Find a teammate near the vacated space (or in the channel the opponent left)
        let best_teammate = ctx.players().teammates().nearby(200.0)
            .filter(|t| {
                let t_dist_to_vacated = (t.position - vacated_zone).magnitude();
                // Teammate should be near the vacated space or generally in that direction
                t_dist_to_vacated < 60.0
                    && ctx.player().has_clear_pass(t.id)
                    && ctx.players().opponents().all()
                        .filter(|opp| (opp.position - t.position).magnitude() < 10.0)
                        .count() < 2
            })
            .min_by(|a, b| {
                let da = (a.position - vacated_zone).magnitude();
                let db = (b.position - vacated_zone).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })?;

        Some(best_teammate)
    }

    /// POSSESSION RETENTION: Determine if team should retain possession rather than
    /// attack directly. True when team is comfortable (not losing), in own/mid third,
    /// and not under heavy pressure.
    fn should_retain_possession(&self, ctx: &StateProcessingContext) -> bool {
        // Never retain if losing
        if ctx.team().is_loosing() {
            return false;
        }

        // Don't retain in attacking third - keep pressing forward
        let goal_dist = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;
        if goal_dist < field_width * 0.35 {
            return false;
        }

        // Don't retain under heavy pressure
        let pressing = self.calculate_pressing_intensity(ctx);
        if pressing > 0.5 {
            return false;
        }

        // Retain possession when team is in control
        ctx.team().is_control_ball()
    }

    /// Movement for possession retention mode: slower, more lateral, controlled tempo
    fn calculate_possession_retention_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;

        // Move laterally rather than directly toward goal
        // Wider sinusoidal sway with slower forward progress
        let to_goal = (goal_pos - player_pos).normalize();
        let phase = (ctx.in_state_time as f32) * std::f32::consts::TAU / 100.0; // Slower period
        let sway = phase.sin() * 0.5; // Wider lateral sway
        let lateral = Vector3::new(-to_goal.y * sway, to_goal.x * sway, 0.0);

        // Move toward a midfield position rather than directly at goal
        // Blend between lateral movement and slight forward progress
        let mid_y = if player_pos.y < field_height / 2.0 {
            field_height * 0.35
        } else {
            field_height * 0.65
        };
        let retention_target = Vector3::new(
            player_pos.x + to_goal.x * 15.0, // Slow forward drift
            mid_y,
            0.0,
        );

        let blended_target = player_pos + (retention_target - player_pos).normalize() * 20.0
            + lateral * 10.0;

        SteeringBehavior::Arrive {
            target: blended_target,
            slowing_distance: 30.0,
        }
        .calculate(ctx.player)
        .velocity * 0.6 // Slower overall speed in retention mode
            + ctx.player().separation_velocity()
    }

    /// COUNTER-ATTACK: Detect if a counter-attack opportunity exists.
    /// True when team just won possession, opponents are high, and space ahead is open.
    fn is_counter_attack_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let ownership_duration = ctx.tick_context.ball.ownership_duration;

        // Must have just won possession (< 15 ticks)
        if ownership_duration >= 15 {
            return false;
        }

        // Ball must be on own side or midfield (counter goes forward)
        if !ctx.ball().on_own_side() {
            // Allow early midfield counters too
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let field_width = ctx.context.field_size.width as f32;
            if goal_dist < field_width * 0.4 {
                return false; // Already in attacking third, no need for counter
            }
        }

        // Count opponents ahead of ball (between ball and opponent goal)
        let ball_pos = ctx.tick_context.positions.ball.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - ball_pos).normalize();

        let opponents_ahead = ctx.players().opponents().all()
            .filter(|opp| {
                let to_opp = opp.position - ball_pos;
                to_opp.normalize().dot(&to_goal) > 0.3 // Opponent is ahead of ball
            })
            .count();

        // Counter-attack opportunity if few opponents ahead
        opponents_ahead < 3
    }

    /// COUNTER-ATTACK: Find a forward pass target for quick transition.
    /// Prefers forwards making runs toward goal with space around them.
    fn find_counter_attack_pass<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        let mut best_target: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(300.0) {
            let to_teammate = teammate.position - player_pos;

            // Must be ahead of us (toward opponent goal)
            if to_teammate.normalize().dot(&to_goal) < 0.3 {
                continue;
            }

            // Must have space (no opponent within 10 units)
            let opponents_near = ctx.players().opponents().all()
                .filter(|opp| (opp.position - teammate.position).magnitude() < 10.0)
                .count();
            if opponents_near >= 2 {
                continue;
            }

            // Must have clear passing lane
            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            // Score: prefer forwards, closer to goal, making runs
            let is_forward = teammate.tactical_positions.is_forward();
            let goal_dist = (goal_pos - teammate.position).magnitude();
            let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
            let making_run = teammate_velocity.magnitude() > 1.0
                && teammate_velocity.normalize().dot(&to_goal) > 0.3;

            let mut score = 1000.0 - goal_dist; // Closer to goal = better
            if is_forward { score += 200.0; }
            if making_run { score += 150.0; }
            if opponents_near == 0 { score += 100.0; }

            if let Some((_, best_score)) = &best_target {
                if score > *best_score {
                    best_target = Some((teammate, score));
                }
            } else {
                best_target = Some((teammate, score));
            }
        }

        best_target.map(|(t, _)| t)
    }

    /// Check if player is stuck in a corner/boundary with multiple players around
    fn is_congested_near_boundary(&self, ctx: &StateProcessingContext) -> bool {
        // Check if near any boundary (within 20 units)
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let pos = ctx.player.position;

        let near_boundary = pos.x < 20.0
            || pos.x > field_width - 20.0
            || pos.y < 20.0
            || pos.y > field_height - 20.0;

        if !near_boundary {
            return false;
        }

        // Count all nearby players (teammates + opponents) within 15 units
        let nearby_teammates = ctx.players().teammates().nearby(15.0).count();
        let nearby_opponents = ctx.players().opponents().nearby(15.0).count();
        let total_nearby = nearby_teammates + nearby_opponents;

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }

    /// Check if wide midfielder should deliver a cross into the box
    fn should_cross(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;

        // Must be in a wide channel (top or bottom 20%)
        let is_wide = y < wide_margin || y > field_height - wide_margin;
        if !is_wide {
            return false;
        }

        // Must be in attacking third
        let goal_dist = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;
        if goal_dist > field_width * 0.35 {
            return false;
        }

        // Must have at least 1 teammate within 150 units of opponent goal
        let goal_pos = ctx.player().opponent_goal_position();
        let teammates_in_box = ctx.players().teammates().all()
            .filter(|t| (t.position - goal_pos).magnitude() < 150.0)
            .count();
        if teammates_in_box < 1 {
            return false;
        }

        // Crossing skill must be decent (> 8.0 on 0-20 scale)
        ctx.player.skills.technical.crossing > 8.0
    }
}