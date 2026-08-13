use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, Opportunity, U_PER_M,
};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// Depth of the penalty area from the goal line (16.5 m). Same figure the
/// forward and midfielder box blocks use.
const PENALTY_AREA_DEPTH: f32 = 132.0;

/// Per-call-site salt for `Opportunity`.
const LONG_SHOT_SALT: u64 = 0xC2B2_AE3D_27D4_EB4F;

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

        // Matches the absolute cap in `evaluate_forward_shot_decision` so
        // a specialist routed here from 35 m isn't immediately bounced
        // back out to passing by a tighter bar than the one that sent him.
        if ctx.player().goal_distance() > 320.0 {
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

        // Inside the penalty area — shoot.
        //
        // Was 44u (5.5 m) plus a binary `has_clear_shot()`, the same
        // tap-in gate the forward's box block carried. A player only
        // reaches this state having ALREADY been approved to strike by
        // the helper, so once he is in the area the question is settled;
        // making him carry to within five metres of the line first is
        // what turns an approved shot into another lap of the box.
        let distance_to_goal = ctx.player().goal_distance();
        if distance_to_goal < PENALTY_AREA_DEPTH {
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
    /// Fallback for a player who reached this state without a
    /// helper-approved strike behind him.
    ///
    /// Was three distance tiers each with its own absolute bar on
    /// `mid_shot_selection` (0.44 / 0.50 / 0.58) plus an xG floor. Two
    /// problems, one of them fatal: the bars are ABSOLUTE, so whether a
    /// long shot exists in a division depended on the division rather
    /// than on the situation — senior football reached an 18% outside-box
    /// share and youth football 2.5% on identical code — and a tier
    /// boundary at 15 m means a man one step further back is a different
    /// footballer.
    ///
    /// One continuous willingness instead, falling with distance and
    /// with traffic, rising with the striking skills, against a bar
    /// drawn once per possession so he does not re-ask it every tick.
    fn is_favorable_shooting_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.player().goal_distance();
        // A long shot only needs a *sight* of goal, not a fully clear
        // lane — it's struck through traffic and the xG model discounts
        // the low clarity. A point-blank wall of defenders (clarity ~0)
        // still aborts.
        let clarity = ctx.player().shot_clarity();
        if clarity < 0.22 {
            return false;
        }

        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let shot_profile = ctx.player().shooting().shot_profile();

        // Room to strike: each man near him takes something off the
        // swing, on a curve rather than at a head-count of three.
        const CROWD_REACH: f32 = 4.0 * U_PER_M;
        let mut crowding = 0.0f32;
        for (_id, dist) in ctx.tick_context.grid.opponents(ctx.player.id, CROWD_REACH) {
            let proximity = 1.0 - (dist / CROWD_REACH).clamp(0.0, 1.0);
            crowding += proximity;
        }
        let room = 1.0 / (1.0 + crowding * 0.9);

        // Distance: comfortable at the edge of the area, fading out to
        // the 40 m the helper itself calls hopeless.
        const COMFORTABLE: f32 = 16.5 * U_PER_M;
        const HOPELESS: f32 = 40.0 * U_PER_M;
        let reach = 1.0
            - ((distance_to_goal - COMFORTABLE) / (HOPELESS - COMFORTABLE)).clamp(0.0, 1.0);

        let strike = (mid_profile.mid_shot_selection * 0.60
            + shot_profile.execution_skill * 0.40)
            .clamp(0.0, 1.0);

        let willingness = strike * reach.powf(0.75) * room * clarity.clamp(0.0, 1.0).powf(0.35);

        let spread = Opportunity::draw(ctx, LONG_SHOT_SALT);
        willingness >= 0.24 + spread * 0.26
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
        // 30u is 3.75 m, so "in passing range" excluded every pass a
        // footballer would call a pass and this whole `should_pass`
        // branch could not fire. 30 m is a midfield ball.
        const PASSING_RANGE: f32 = 30.0 * U_PER_M;
        let is_in_passing_range =
            (teammate.position - ctx.player.position).norm_squared() <= PASSING_RANGE * PASSING_RANGE;
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
        // 10u was 1.25 m — nobody within arm's length, which is not what
        // "space to dribble" means.
        const DRIBBLE_SPACE: f32 = 4.0 * U_PER_M;
        !ctx.players().opponents().exists(DRIBBLE_SPACE)
    }
}
