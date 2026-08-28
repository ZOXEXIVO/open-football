use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfieldPlay, MidfieldRole, MidfielderCondition, Opportunity,
};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::ThroughBall;
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    Ball, ConditionContext, MatchPlayerLite, PassEvaluator, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// Edge of the penalty area (16.5 m) and the distance beyond which the
/// shot helper itself calls a strike hopeless (35 m).
const COMFORTABLE_RANGE: f32 = 132.0;
const LONG_SHOT_LIMIT: f32 = 280.0;

/// Per-call-site salt for `Opportunity`.
const BAILOUT_SALT: u64 = 0x3C79_AC49_2BA7_B653;

#[derive(Default, Clone)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Running
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // "Shoot instead of pass" pivot — every midfielder routes through
        // the shared forward helper. Was AM-only, with non-AMs using the
        // legacy `should_shoot_instead_of_pass` deterministic election;
        // that side-door fired on hardcoded range / selection bars and
        // bypassed the helper's willingness roll, xG floor and
        // anti-monopoly cap. Unified (2026-06-11) with the rest of the
        // MID shot paths during the fatigue-normalization rebalance so
        // chance-quality logic lives in one place.
        if let ShotDecision::Shoot { reason } = evaluate_forward_shot_decision(ctx, "MID_PASS_FWD")
        {
            return Some(
                StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                    .with_shot_reason(reason),
            );
        }

        // Emergency clearance — midfielder has the ball very close to
        // our own goal AND under heavy pressure AND has no safe pass
        // option. A blanket "clear when pressured in defensive third"
        // trigger fired 80+ times per match per team — each clearance
        // is a possession-flip at the halfway line, and each flip
        // produces a counter-attack chance. Gating tight here brings
        // the clearance rate down toward real football's ~15/team.
        //
        // Midfielders don't have a dedicated Clearing state, so emit
        // the ClearBall event directly and transition to Running.
        let under_pressure = self.is_under_heavy_pressure(ctx);
        if under_pressure && self.in_box_danger_zone(ctx) {
            let has_safe_pass = ctx.player().passing().find_safe_pass_option().is_some();
            if !has_safe_pass {
                if let Some(event) = self.emit_emergency_clearance(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        event,
                    ));
                }
            }
        }

        // Brief scanning delay before executing pass (unless under pressure)
        let min_scan_time: u64 = if under_pressure { 2 } else { 5 };

        if ctx.in_state_time >= min_scan_time {
            // First, the ball that beats a line.
            //
            // This used to be `find_breakthrough_pass_option`, which
            // asked `would_pass_break_defensive_lines` — a function that
            // returns **false unless at least two opponents are standing
            // in the passing lane**. So the "breakthrough" pass was, by
            // construction, only ever a ball played THROUGH A CROWD, and
            // never the ball slid into an empty channel that the phrase
            // describes. It also aimed at the receiver's feet, where a
            // through ball's whole point is the grass in front of him.
            // The `!on_own_side` fence went with it: a ball out of your
            // own half that puts a man in behind is the best pass in
            // football, not one to be refused on territory.
            //
            // See [`ThroughBall`], which solves the meeting point, checks
            // he is onside when it is struck and that he wins the race to
            // it, and aims at a POINT rather than at a man.
            let role = MidfieldRole::of(ctx, &MidfielderSkillProfile::from_ctx(ctx));
            if let Some(ball) = (!MidfieldPlay::legacy())
                .then(|| ThroughBall::find(ctx, role.creation))
                .flatten()
            {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(ball.target_id)
                            .with_target_point(ball.aim_point)
                            .with_reason(ball.kind.reason())
                            .build(ctx),
                    )),
                ));
            }

            // Find the best regular pass option with improved logic
            if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_PASSING_STATE")
                            .build(ctx),
                    )),
                ));
            }
        }

        // If no good passing option after waiting, try something else
        // Under heavy pressure, bail out faster to dribble away
        let bail_time = if self.is_under_heavy_pressure(ctx) {
            10
        } else {
            20
        };
        if ctx.in_state_time > bail_time {
            let goal_dist = ctx.ball().distance_to_opponent_goal();

            // The AM carve-out that used to sit here called
            // `evaluate_forward_shot_decision` a second time, under a
            // different tag, for attacking midfielders only. The FIRST
            // line of this state already calls it — every tick, for
            // every midfielder — so the carve-out could only ever repeat
            // an answer that had just been given, and the position gate
            // said a #10 is allowed a shot that an #8 in the identical
            // spot is not. Both are gone; the helper is the one place
            // the shoot/pass question is asked.
            //
            // What is left is the genuinely different situation: he has
            // been looking for a pass and there isn't one. That earns a
            // slightly bolder look than the helper's standing bar, on a
            // continuous willingness rather than the two absolute skill
            // bars (0.46 / 0.58) this used to carry. Absolute bars mean
            // the DIVISION decides whether long shots exist — the same
            // fault the DistanceShooting tiers had, where senior
            // football reached an 18% outside-box share and youth 2.5%
            // on identical code.
            let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
            let shot_profile = ctx.player().shooting().shot_profile();
            let sighted = ctx.player().has_clear_shot() && ctx.player().shooting().has_good_angle();

            if sighted && goal_dist < LONG_SHOT_LIMIT {
                // Falls from 1 at the edge of the box to 0 where the
                // helper itself calls a strike hopeless.
                let reach = 1.0
                    - ((goal_dist - COMFORTABLE_RANGE) / (LONG_SHOT_LIMIT - COMFORTABLE_RANGE))
                        .clamp(0.0, 1.0);
                let strike =
                    mid_profile.mid_shot_selection * 0.55 + shot_profile.execution_skill * 0.45;
                // No outlet is itself a reason: a player with nothing on
                // has less to lose by hitting it.
                let willingness = strike * (0.45 + reach.powf(0.6) * 0.75);
                let spread = Opportunity::draw(ctx, BAILOUT_SALT);
                if willingness >= 0.34 + spread * 0.30 {
                    return Some(if goal_dist > COMFORTABLE_RANGE {
                        StateChangeResult::with_midfielder_state(MidfielderState::DistanceShooting)
                            .with_shot_reason("MID_PASS_BAILOUT_DISTANCE")
                    } else {
                        StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                            .with_shot_reason("MID_PASS_BAILOUT_SHOOT")
                    });
                }
            }

            // No pass and no shot — carry it and look again from
            // somewhere better.
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Dribbling,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If under heavy pressure, shield the ball and create space
        if self.is_under_heavy_pressure(ctx) {
            // Move away from nearest opponent to create passing space
            if let Some(nearest_opponent) = ctx.players().opponents().nearby(15.0).next() {
                let away_from_opponent =
                    (ctx.player.position - nearest_opponent.position).normalize();
                // Shield ball by moving perpendicular to goal direction
                let to_goal =
                    (ctx.player().opponent_goal_position() - ctx.player.position).normalize();
                let perpendicular = Vector3::new(-to_goal.y, to_goal.x, 0.0);
                let escape_direction = (away_from_opponent * 0.7 + perpendicular * 0.3).normalize();
                return Some(escape_direction * 2.5 + ctx.player().separation_velocity());
            }
        }

        // Adjust position to find better passing angles if needed
        if self.should_adjust_position(ctx) {
            if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
                return Some(
                    SteeringBehavior::Arrive {
                        target: self.calculate_better_passing_position(ctx, &nearest_teammate),
                        slowing_distance: 30.0,
                    }
                    .calculate(ctx.player)
                    .velocity
                        + ctx.player().separation_velocity(),
                );
            }
        }

        // Default: stationary while scanning for pass options
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing is low intensity - minimal fatigue
        MidfielderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl MidfielderPassingState {
    /// Best ordinary pass on — the ball to feet, as against the ball
    /// into space above.
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(ctx, 400.0)
    }

    #[allow(dead_code)]
    fn has_clear_passing_lane(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let player_position = ctx.player.position;
        let teammate_position = teammate.position;
        let passing_direction = (teammate_position - player_position).normalize();
        let pass_distance = (teammate_position - player_position).magnitude();

        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;

        let base_lane_width = 3.0;
        let skill_factor = 0.6 + (pass_skill * 0.2) + (vision_skill * 0.2);
        let lane_width = base_lane_width * skill_factor;

        let intercepting_opponents = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let to_opponent = opponent.position - player_position;
                let projection_distance = to_opponent.dot(&passing_direction);

                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                let projected_point = player_position + passing_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).magnitude();

                let interception_skill = ctx.player().skills(opponent.id).technical.tackling / 20.0;
                let effective_width = lane_width * (1.0 - interception_skill * 0.3);

                perp_distance < effective_width
            })
            .count();

        intercepting_opponents == 0
    }

    /// Check if player is heavily marked
    #[allow(dead_code)]
    fn is_heavily_marked(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        const MARKING_DISTANCE: f32 = 5.0;
        const MAX_MARKERS: usize = 2;

        // Use pre-computed distances: opponents near teammate
        let mut marker_count = 0;
        let mut single_marker_id = 0u32;
        let mut single_marker_dist = 0.0f32;
        for (opp_id, dist) in ctx
            .tick_context
            .grid
            .opponents(teammate.id, MARKING_DISTANCE)
        {
            marker_count += 1;
            single_marker_id = opp_id;
            single_marker_dist = dist;
        }

        if marker_count >= MAX_MARKERS {
            return true;
        }

        if marker_count == 1 {
            // Effectiveness of a single tight marker scales with their
            // positioning (sigmoid pivot at 16/20) and proximity.
            // Treat the combined signal as "heavily marked" when above
            // 0.5 — replaces the hard `> 16.0 && < 2.5` cliff that
            // turned every sub-elite marker into a non-factor.
            let marking_skill = ctx.player().skills(single_marker_id).mental.positioning;
            let skill_p = SkillCurve::new(marking_skill, 16.0, 0.6).probability();
            let proximity = (1.0 - (single_marker_dist / MARKING_DISTANCE)).clamp(0.0, 1.0);
            if skill_p * proximity > 0.40 {
                return true;
            }
        }

        false
    }

    /// Check if teammate is in good position
    #[allow(dead_code)]
    fn is_in_good_position(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let is_backward_pass = match ctx.player.side {
            Some(PlayerSide::Left) => teammate.position.x < ctx.player.position.x,
            Some(PlayerSide::Right) => teammate.position.x > ctx.player.position.x,
            None => false,
        };

        let player_goal_distance =
            (ctx.player.position - ctx.player().opponent_goal_position()).magnitude();
        let teammate_goal_distance =
            (teammate.position - ctx.player().opponent_goal_position()).magnitude();
        let advances_toward_goal = teammate_goal_distance < player_goal_distance;

        if is_backward_pass {
            let under_pressure = self.is_under_heavy_pressure(ctx);
            let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
            // Backward passes require either pressure escape or genuine
            // long-pass profile (switch / recycle judgement, not raw vision).
            return under_pressure || mid_profile.allows_switch_play();
        }

        let teammate_will_be_pressured =
            ctx.tick_context
                .grid
                .opponents(teammate.id, 15.0)
                .any(|(opp_id, _dist)| {
                    let opp_pos = ctx.tick_context.positions.players.position(opp_id);
                    let opponent_velocity = ctx.tick_context.positions.players.velocity(opp_id);
                    let future_opponent_pos = opp_pos + opponent_velocity * 10.0;
                    let future_distance = (future_opponent_pos - teammate.position).magnitude();
                    future_distance < 5.0
                });

        advances_toward_goal && !teammate_will_be_pressured
    }

    /// Check if under heavy pressure
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_heavy_pressure()
    }

    /// True if the midfielder is right next to our own goal — inside
    /// ~18-yard-box distance. Tighter than "defensive third": a pass
    /// from the third is still often the right call, but from 20m out
    /// in front of our net it's safer to hoof.
    fn in_box_danger_zone(&self, ctx: &StateProcessingContext) -> bool {
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_to_own_goal = (ctx.tick_context.positions.ball.position - own_goal).magnitude();
        // ~18 yards = ~16.5m = ~130u on an 840u pitch.
        ball_to_own_goal < 130.0
    }

    /// Hoof-clearance toward the halfway line. Mirrors the defender
    /// Clearing state: lofted z, horizontal aimed at the centre of the
    /// pitch at midfield, so the ball lands in contested zone instead
    /// of rolling into opponents' feet near our goal.
    fn emit_emergency_clearance(&self, ctx: &StateProcessingContext) -> Option<Event> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let halfway_x = field_width * 0.5;
        let mid_y = field_height * 0.5;

        // Target: halfway line, centre-ish. Pull Y toward centre so the
        // ball doesn't drift to a sideline.
        let target_x = match ctx.player.side {
            Some(PlayerSide::Left) => halfway_x.max(ball_pos.x + 40.0),
            Some(PlayerSide::Right) => halfway_x.min(ball_pos.x - 40.0),
            None => halfway_x,
        };
        let target_y = ball_pos.y + (mid_y - ball_pos.y) * 0.6;

        let to_target = Vector3::new(target_x - ball_pos.x, target_y - ball_pos.y, 0.0);
        let dist = to_target.norm().max(0.1);
        let dir = to_target / dist;

        // Solved from an apex, not written. The vertical axis is in
        // METRES (see `GRAVITY_PER_TICK`), so the `5.0` this used to
        // carry was 500 m/s straight up — a **12.7 km** apex and a hang
        // time of a minute and a half. It was the single worst launch
        // left in the engine after the metric conversion, and the ball
        // flight census pinned it by its apex alone.
        const OUTLET_APEX_M: f32 = 10.0;
        let z_velocity = Ball::launch_speed_for_apex(OUTLET_APEX_M);
        // The arc's hang time is the budget: reach the aim point inside
        // it and the hoof lands where it was aimed.
        let hang = Ball::hang_ticks(z_velocity).max(1.0);
        let horizontal_speed = (dist / hang).clamp(0.30, 2.6);

        let ball_velocity = Vector3::new(
            dir.x * horizontal_speed,
            dir.y * horizontal_speed,
            z_velocity,
        );

        Some(Event::PlayerEvent(PlayerEvent::ClearBall(ball_velocity)))
    }

    /// Check if should adjust position
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        self.find_best_pass_option(ctx).is_none() && !self.is_under_heavy_pressure(ctx)
    }

    /// Calculate better position for passing
    fn calculate_better_passing_position(
        &self,
        ctx: &StateProcessingContext,
        target: &MatchPlayerLite,
    ) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        let target_pos = target.position;

        let to_target = target_pos - player_pos;
        let direction = to_target.normalize();

        let perpendicular = Vector3::new(-direction.y, direction.x, 0.0);
        let adjustment = perpendicular * 5.0;

        player_pos + adjustment
    }
}
