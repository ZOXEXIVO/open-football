use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PlayerEvent, PassingEventContext};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use crate::PlayerFieldPositionGroup;
use nalgebra::Vector3;
use rand::RngExt;

/// Types of goalkeeper distribution
#[derive(Debug, Clone, Copy, PartialEq)]
enum GoalkeeperDistributionType {
    /// Short pass to nearby defender (0-30m)
    ShortPass,
    /// Medium pass to midfielder (30-60m)
    MediumPass,
    /// Long kick downfield to forward (60-100m+)
    LongKick,
    /// Clearance - just get the ball away from danger
    Clearance,
    /// Throw using long throw skill (15-40m)
    Throw,
}

const UNDER_PRESSURE_DISTANCE: f32 = 25.0;
const SAFE_PASS_DISTANCE: f32 = 30.0;
const MEDIUM_PASS_DISTANCE: f32 = 60.0;
const LONG_KICK_MIN_DISTANCE: f32 = 60.0; // Reduced from 100.0

#[derive(Default)]
pub struct GoalkeeperPassingState {}

impl StateProcessingHandler for GoalkeeperPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Determine distribution type based on pressure and game situation
        let distribution_type = self.decide_distribution_type(ctx);

        // Execute the appropriate distribution
        match distribution_type {
            GoalkeeperDistributionType::Clearance => {
                // Emergency clearance - transition to clearing state
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Clearing,
                ));
            }
            GoalkeeperDistributionType::ShortPass => {
                if let Some(teammate) = self.find_short_pass_target(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state_and_event(
                        GoalkeeperState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(teammate.id)
                                .with_pass_force(2.5) // Gentle pass
                                .with_reason("GK_PASSING_SHORT")
                                .build(ctx)
                        )),
                    ));
                }
            }
            GoalkeeperDistributionType::MediumPass => {
                if let Some(teammate) = self.find_medium_pass_target(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state_and_event(
                        GoalkeeperState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(teammate.id)
                                .with_pass_force(4.5) // Medium power
                                .with_reason("GK_PASSING_MEDIUM")
                                .build(ctx)
                        )),
                    ));
                }
            }
            GoalkeeperDistributionType::LongKick => {
                if let Some(event) = self.execute_long_kick(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state_and_event(
                        GoalkeeperState::Standing,
                        event,
                    ));
                }
            }
            GoalkeeperDistributionType::Throw => {
                if let Some(teammate) = self.find_throw_target(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state_and_event(
                        GoalkeeperState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(teammate.id)
                                .with_pass_force(3.5) // Throw power
                                .with_reason("GK_PASSING_THROW")
                                .build(ctx)
                        )),
                    ));
                }
            }
        }

        // Timeout - just do something
        if ctx.in_state_time > 30 {
            // Default to clearance after waiting too long
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Clearing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperPassingState {
    /// Decide which type of distribution to use based on pressure and situation
    fn decide_distribution_type(&self, ctx: &StateProcessingContext) -> GoalkeeperDistributionType {
        // Check for immediate pressure
        let opponents_nearby = ctx.players().opponents().nearby(UNDER_PRESSURE_DISTANCE).count();
        let under_heavy_pressure = opponents_nearby >= 2;
        let under_pressure = opponents_nearby >= 1;

        // Goalkeeper skills
        let vision = ctx.player.skills.mental.vision / 20.0;
        let kicking_power = ctx.player.skills.technical.long_throws / 20.0;
        let passing = ctx.player.skills.technical.passing / 20.0;
        let _decisions = ctx.player.skills.mental.decisions / 20.0;

        // Under heavy pressure - clear it!
        if under_heavy_pressure {
            return GoalkeeperDistributionType::Clearance;
        }

        // Check available options at different ranges
        let has_short_option = self.count_safe_teammates(ctx, SAFE_PASS_DISTANCE) > 0;
        let has_medium_option = self.count_safe_teammates(ctx, MEDIUM_PASS_DISTANCE) > 0;
        let has_long_option = self.count_teammates_in_space(ctx, LONG_KICK_MIN_DISTANCE) > 0;

        // Decision logic based on pressure and skills
        if under_pressure {
            // Some pressure - prefer quick throw or short pass to safe defender
            if kicking_power > 0.7 && has_medium_option {
                return GoalkeeperDistributionType::Throw;
            } else if has_short_option {
                return GoalkeeperDistributionType::ShortPass;
            } else {
                return GoalkeeperDistributionType::Clearance;
            }
        }

        // No immediate pressure - use vision and decision making
        let mut rng = rand::rng();
        let decision_random: f32 = rng.random();

        // Better vision = more likely to attempt long distribution
        if vision > 0.7 && has_long_option && decision_random < (vision * 0.6) {
            return GoalkeeperDistributionType::LongKick;
        }

        // Good kicking - prefer throws for medium range
        if kicking_power > 0.6 && has_medium_option && decision_random < 0.4 {
            return GoalkeeperDistributionType::Throw;
        }

        // Good passing - prefer build-up play
        if passing > 0.6 && has_medium_option && decision_random < (passing * 0.7) {
            return GoalkeeperDistributionType::MediumPass;
        }

        // Default: safe short pass if available, otherwise medium
        if has_short_option {
            GoalkeeperDistributionType::ShortPass
        } else if has_medium_option {
            GoalkeeperDistributionType::MediumPass
        } else {
            GoalkeeperDistributionType::LongKick
        }
    }

    /// Find the best target for a short pass (to nearby defenders)
    fn find_short_pass_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let mut best_target = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(SAFE_PASS_DISTANCE) {
            let distance = teammate.distance(ctx);

            // Prefer defenders for short passes
            let is_defender = matches!(
                teammate.tactical_positions.position_group(),
                PlayerFieldPositionGroup::Defender
            );

            // Check if teammate is under pressure
            let opponents_near_teammate = ctx.tick_context.distances.opponents(teammate.id, 10.0).count();
            let is_safe = opponents_near_teammate == 0;

            // Check if teammate is in good position (not marked)
            let space_factor = if is_safe {
                2.0
            } else if opponents_near_teammate == 1 {
                0.7
            } else {
                0.2
            };

            // Prefer closer, safe teammates who are defenders
            let position_bonus = if is_defender { 1.5 } else { 1.0 };
            let distance_score = (SAFE_PASS_DISTANCE - distance) / SAFE_PASS_DISTANCE;

            let score = distance_score * space_factor * position_bonus;

            if score > best_score {
                best_score = score;
                best_target = Some(teammate);
            }
        }

        best_target
    }

    /// Find the best target for a medium pass (to midfielders)
    fn find_medium_pass_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let mut best_target = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(MEDIUM_PASS_DISTANCE) {
            let distance = teammate.distance(ctx);

            // Skip if too close (use short pass instead)
            if distance < SAFE_PASS_DISTANCE {
                continue;
            }

            // Prefer midfielders for medium passes
            let is_midfielder = matches!(
                teammate.tactical_positions.position_group(),
                PlayerFieldPositionGroup::Midfielder
            );

            // Check space around receiver
            let opponents_near = ctx.tick_context.distances.opponents(teammate.id, 12.0).count();
            let space_factor = match opponents_near {
                0 => 2.0,
                1 => 1.2,
                _ => 0.5,
            };

            // Forward progress
            let forward_progress = (teammate.position.x - ctx.player.position.x).max(0.0);
            let progress_factor = forward_progress / 100.0;

            let position_bonus = if is_midfielder { 1.3 } else { 1.0 };

            let score = space_factor * progress_factor * position_bonus;

            if score > best_score {
                best_score = score;
                best_target = Some(teammate);
            }
        }

        best_target
    }

    /// Find the best target for a throw (using long throw skill)
    fn find_throw_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let kicking_power = ctx.player.skills.technical.long_throws / 20.0;
        let max_throw_distance = 25.0 + (kicking_power * 25.0); // 25-50m range

        let mut best_target = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(max_throw_distance) {
            let distance = teammate.distance(ctx);

            // Throws work best at 20-40m range
            if distance < 15.0 || distance > max_throw_distance {
                continue;
            }

            // Check if on the wing (throws often go wide)
            let is_wide = teammate.position.y.abs() > (ctx.context.field_size.height as f32 * 0.3);

            // Space around receiver
            let opponents_near = ctx.tick_context.distances.opponents(teammate.id, 10.0).count();
            let space_factor = match opponents_near {
                0 => 2.0,
                1 => 1.0,
                _ => 0.4,
            };

            let wide_bonus = if is_wide { 1.3 } else { 1.0 };
            let distance_factor = 1.0 - ((distance - 20.0).abs() / max_throw_distance);

            let score = space_factor * wide_bonus * distance_factor;

            if score > best_score {
                best_score = score;
                best_target = Some(teammate);
            }
        }

        best_target
    }

    /// Execute a long kick downfield
    fn execute_long_kick(&self, ctx: &StateProcessingContext) -> Option<Event> {
        let _vision = ctx.player.skills.mental.vision / 20.0;
        let kicking_power = ctx.player.skills.technical.long_throws / 20.0;
        let _technique = ctx.player.skills.technical.technique / 20.0;

        // Reduced max distance for more realistic kicks
        let max_distance = ctx.context.field_size.width as f32 * 0.8; // Reduced from 1.5

        let mut best_target = None;
        let mut best_score = 0.0;

        // Look for forwards in good positions
        for teammate in ctx.players().teammates().nearby(max_distance) {
            let distance = teammate.distance(ctx);

            // Only consider long range
            if distance < LONG_KICK_MIN_DISTANCE {
                continue;
            }

            // Strongly prefer forwards
            let is_forward = matches!(
                teammate.tactical_positions.position_group(),
                PlayerFieldPositionGroup::Forward
            );

            if !is_forward {
                continue;
            }

            // Forward progress (reduced requirement)
            let forward_progress = teammate.position.x - ctx.player.position.x;
            if forward_progress < 30.0 {
                continue; // Don't kick to players not upfield
            }

            // Space around receiver
            let opponents_near = ctx.tick_context.distances.opponents(teammate.id, 15.0).count();
            let space_factor = match opponents_near {
                0 => 3.0,
                1 => 1.5,
                2 => 0.8,
                _ => 0.3,
            };

            // Distance capability based on kicking power (reduced ranges)
            let distance_factor = if distance > 150.0 {
                kicking_power * 1.5 // Only strong kickers can reach far
            } else {
                1.0 + kicking_power * 0.5
            };

            let score = space_factor * distance_factor * (forward_progress / ctx.context.field_size.width as f32);

            if score > best_score {
                best_score = score;
                best_target = Some(teammate);
            }
        }

        if let Some(target) = best_target {
            // Use moderate force for long kicks (reduced from 6.0-8.5)
            let kick_force = 4.5 + (kicking_power * 1.5); // 4.5-6.0 range

            Some(Event::PlayerEvent(PlayerEvent::PassTo(
                PassingEventContext::new()
                    .with_from_player_id(ctx.player.id)
                    .with_to_player_id(target.id)
                    .with_pass_force(kick_force)
                    .with_reason("GK_PASSING_LONG_KICK")
                    .build(ctx)
            )))
        } else {
            // No good target - will need to clear from a different state
            None
        }
    }

    /// Count teammates in safe positions within range
    fn count_safe_teammates(&self, ctx: &StateProcessingContext, range: f32) -> usize {
        ctx.players()
            .teammates()
            .nearby(range)
            .filter(|teammate| {
                // Check if teammate is not heavily marked
                let opponents_near = ctx.tick_context.distances.opponents(teammate.id, 8.0).count();
                opponents_near < 2
            })
            .count()
    }

    /// Count teammates in space at long range
    fn count_teammates_in_space(&self, ctx: &StateProcessingContext, min_distance: f32) -> usize {
        // Reduced max distance for more realistic searches
        let max_distance = ctx.context.field_size.width as f32 * 0.8;

        ctx.players()
            .teammates()
            .nearby(max_distance)
            .filter(|teammate| {
                let distance = teammate.distance(ctx);
                if distance < min_distance {
                    return false;
                }

                // Must be forward
                let is_forward = matches!(
                    teammate.tactical_positions.position_group(),
                    PlayerFieldPositionGroup::Forward
                );

                if !is_forward {
                    return false;
                }

                // Must have some space
                let opponents_near = ctx.tick_context.distances.opponents(teammate.id, 15.0).count();
                opponents_near <= 1
            })
            .count()
    }
}
