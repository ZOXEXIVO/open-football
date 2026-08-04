use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderDistanceShootingState {}

impl StateProcessingHandler for MidfielderDistanceShootingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Per-player cooldown — same reasoning as forwards. A long
        // shot requires planted feet and clean contact; a player who
        // just struck the ball hasn't reset yet.
        if !ctx.player().can_shoot() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.player().goal_distance() > 280.0 {
            // Too far from the goal, consider other options
            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            } else if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ));
            }
        }

        // Close to goal with a clear sight — shoot (5.5m; the old 50u
        // gate fired UNCONDITIONALLY from 6.25m with no clarity check)
        let distance_to_goal = ctx.player().goal_distance();
        if distance_to_goal < 44.0 && ctx.player().has_clear_shot() {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Shooting,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("MID_DISTANCE_SHOOTING_CLOSE")
                        .build(ctx),
                )),
            ));
        }

        // An APPROVED strike is authoritative. `MidfielderRunningState`
        // routes every helper-approved shot beyond 13m here rather than
        // to `Shooting`, so re-running this state's own ABSOLUTE skill
        // bars (mid_shot_selection >= 0.44/0.50/0.58) discarded the
        // decision — measured: 93.6% of all long-range approvals arrive
        // through this path and only ~9% ever became a shot. The bars
        // are absolute, so a youth league clears them essentially never,
        // which is why senior football reached an 18% outside-box share
        // while youth sat at 2.5% on identical code.
        if ctx.player.pending_shot_reason.is_some() || self.is_favorable_shooting_opportunity(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Shooting,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason(
                            ctx.player
                                .pending_shot_reason
                                .unwrap_or("MID_DISTANCE_SHOOTING"),
                        )
                        .build(ctx),
                )),
            ));
        }

        // Timeout — prefer passing over forced shot
        if ctx.in_state_time > 60 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 150.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Distance shooting is very high intensity - explosive action
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl MidfielderDistanceShootingState {
    fn is_favorable_shooting_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.player().goal_distance();
        // A long shot only needs a *sight* of goal, not a fully clear
        // lane — it's struck through traffic and the xG model discounts
        // the low clarity. A point-blank wall of defenders (clarity ~0)
        // or being swarmed still aborts.
        let clarity = ctx.player().shot_clarity();
        let close_opponents = ctx.tick_context.grid.opponents(ctx.player.id, 10.0).count();
        if clarity < 0.22 || close_opponents >= 3 {
            return false;
        }

        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let shot_profile = ctx.player().shooting().shot_profile();

        // Tier the distance gates by midfielder shot selection. Lowered
        // from the previous specialist-only bars (0.58 / 0.72) so a
        // competent playmaker — not just a 1-in-a-squad long-range
        // specialist — drives the occasional shot from range.
        if distance_to_goal <= 120.0 {
            mid_profile.mid_shot_selection >= 0.44
                && shot_profile.expected_xg(distance_to_goal, true) >= 0.16
        } else if distance_to_goal <= 160.0 {
            mid_profile.mid_shot_selection >= 0.50
                && close_opponents <= 2
                && shot_profile.expected_xg(distance_to_goal, true) >= 0.10
        } else if distance_to_goal <= 200.0 {
            mid_profile.mid_shot_selection >= 0.58
                && shot_profile.execution_skill >= 0.50
                && shot_profile.expected_xg(distance_to_goal, true) >= 0.055
        } else {
            false
        }
    }

    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Determine if the player should pass based on the game state

        let teammates = ctx.players().teammates();
        let mut open_teammates = teammates
            .all()
            .filter(|teammate| self.is_teammate_open(ctx, teammate));

        let has_open_teammate = open_teammates.next().is_some();
        let under_pressure = self.is_under_pressure(ctx);

        has_open_teammate && under_pressure
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Determine if the player should dribble based on the game state
        let has_space = self.has_space_to_dribble(ctx);
        let under_pressure = self.is_under_pressure(ctx);

        has_space && !under_pressure
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Check if a teammate is open to receive a pass
        let is_in_passing_range =
            (teammate.position - ctx.player.position).norm_squared() <= 30.0 * 30.0;
        let has_clear_passing_lane = self.has_clear_passing_lane(ctx, teammate);

        is_in_passing_range && has_clear_passing_lane
    }

    fn has_clear_passing_lane(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        // Check if there is a clear passing lane to a teammate without any obstructing opponents
        let player_position = ctx.player.position;
        let teammate_position = teammate.position;
        let passing_direction = (teammate_position - player_position).normalize();

        let ray_cast_result = ctx.tick_context.space.cast_ray(
            player_position,
            passing_direction,
            (teammate_position - player_position).magnitude(),
            false,
        );

        ray_cast_result.is_none() // No collisions with opponents
    }

    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_immediate_pressure()
    }

    fn has_space_to_dribble(&self, ctx: &StateProcessingContext) -> bool {
        let dribble_distance = 10.0;
        !ctx.players().opponents().exists(dribble_distance)
    }
}
