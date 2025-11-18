use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

// Shooting distance constants for midfielders
const MAX_SHOOTING_DISTANCE: f32 = 100.0; // Midfielders rarely shoot from beyond ~50m
const STANDARD_SHOOTING_DISTANCE: f32 = 70.0; // Standard shooting range for midfielders
const PRESSURE_CHECK_DISTANCE: f32 = 10.0; // Distance to check for opponent pressure before shooting

#[derive(Default)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // Priority: Clear ball if congested anywhere (not just boundaries)
            // Allow emergency clearances even without stable possession
            if self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested() {
                // Try to find a good pass option first using the standard evaluator
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_BEST")
                                .build(ctx),
                        )),
                    ));
                }

                // Fallback: find any nearby teammate within reasonable distance
                if let Some(target_teammate) = ctx.players().teammates().nearby(100.0).next() {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
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

            // Standard shooting - close enough with clear shot
            if goal_dist <= STANDARD_SHOOTING_DISTANCE
                && ctx.player().has_clear_shot()
                && finishing > 0.55 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            // Distance shooting - long range with good skills and minimal pressure
            if goal_dist <= MAX_SHOOTING_DISTANCE
                && ctx.player().has_clear_shot()
                && long_shots > 0.6
                && finishing > 0.5
                && !ctx.players().opponents().exists(PRESSURE_CHECK_DISTANCE) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            // Enhanced passing decision based on skills and pressing
            // Require stable possession before allowing passes (prevents instant pass after claiming ball)
            if ctx.ball().has_stable_possession() && self.should_pass(ctx) {
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
            // Using new chaining syntax: nearby(100.0).with_ball(ctx)
            if let Some(opponent) = ctx.players().opponents().nearby(100.0).with_ball(ctx).next() {
                let opponent_distance = (opponent.position - ctx.player.position).magnitude();

                // If opponent with ball is very close, tackle immediately
                if opponent_distance < 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }

                // If opponent with ball is nearby, press them (already filtered by nearby(100.0))
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // Emergency: if ball is nearby, stopped, and unowned, go for it immediately
            if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
                let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
                if ball_velocity < 1.0 {
                    // Ball is stopped or nearly stopped - take it directly
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::TakeBall,
                    ));
                }
            }

            if ctx.ball().distance() < 30.0 && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
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
                        path_offset: 5.0, // Fixed offset instead of random
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                );
            }
        }

        // Simplified movement calculation
        if ctx.player.has_ball(ctx) {
            Some(self.calculate_simple_ball_movement(ctx))
        } else if ctx.team().is_control_ball() {
            Some(self.calculate_simple_support_movement(ctx))
        } else {
            Some(self.calculate_simple_defensive_movement(ctx))
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

        // Add small lateral movement based on time for variation
        let lateral = if ctx.in_state_time % 60 < 30 {
            Vector3::new(-to_goal.y * 0.2, to_goal.x * 0.2, 0.0)
        } else {
            Vector3::new(to_goal.y * 0.2, -to_goal.x * 0.2, 0.0)
        };

        let target = player_pos + (to_goal + lateral).normalize() * 40.0;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Simplified support movement
    fn calculate_simple_support_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let player_pos = ctx.player.position;

        // Simple triangle formation with ball
        let angle = if player_pos.y < ctx.context.field_size.height as f32 / 2.0 {
            -45.0_f32.to_radians()
        } else {
            45.0_f32.to_radians()
        };

        let support_offset = Vector3::new(
            angle.cos() * 30.0,
            angle.sin() * 30.0,
            0.0,
        );

        let target = ball_pos + support_offset;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 15.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Simplified defensive movement
    fn calculate_simple_defensive_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Move toward midpoint between ball and starting position
        let ball_pos = ctx.tick_context.positions.ball.position;
        let start_pos = ctx.player.start_position;

        let target = (ball_pos + start_pos) * 0.5;

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
}