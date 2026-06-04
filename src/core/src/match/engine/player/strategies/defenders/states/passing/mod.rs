use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderPassingState {}

impl StateProcessingHandler for DefenderPassingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the defender still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to appropriate state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // ── Counter-attack outlet ──────────────────────────────────────
        //
        // Real-football upset mechanic: when a defender wins the ball
        // in their own half AND the opposition has over-committed
        // forward (5+ opponents in our defensive half), the canonical
        // response is a long ball over the press to a forward making a
        // run, not a safe sideways pass to another defender. Without
        // this, conservative weak teams keep recycling possession
        // backward, never reach the final third with momentum, and
        // never threaten the strong team. Adding the outlet directly
        // addresses the 0% upset rate at extreme skill gaps in
        // audit_engine_gap: weak teams reach the final third 65×/match
        // already but only shoot 0.08× per entry; a counter-attack
        // gives them a chance to ARRIVE with momentum instead of being
        // tackled out of a buildup phase.
        if let Some(target) = Self::find_counter_attack_target(ctx) {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target.id)
                        .with_reason("DEF_COUNTER_ATTACK")
                        .build(ctx),
                )),
            ));
        }

        // Under heavy pressure — prefer a safe pass, any safe pass. The
        // old rule "clear if safe pass < 20u" was too eager; a short
        // safe pass is still a ball-retention win. Only escalate to
        // Clearing when truly no safe pass exists. Bulk of the 80+
        // clearances per match came from this branch firing whenever a
        // short passing option was available but too close.
        if ctx.player().pressure().is_under_heavy_pressure() {
            // Profile-driven branch: a low buildup_profile defender is
            // told to clear directly under heavy pressure (the spec's
            // `must_clear_under_pressure`). Higher buildup_profile +
            // press_resistance lets us play out via a safe pass.
            let def_profile = DefenderSkillProfile::from_ctx(ctx);
            if def_profile.must_clear_under_pressure() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }
            return if let Some(safe_option) = ctx.player().passing().find_safe_pass_option() {
                Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(safe_option.id)
                            .with_reason("DEF_PASSING_UNDER_PRESSURE")
                            .build(ctx),
                    )),
                ))
            } else {
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ))
            };
        }

        // If teammates are tired, prefer a safe short pass
        if self.are_teammates_tired(ctx) {
            if let Some(safe_target) = ctx
                .player()
                .passing()
                .find_safe_pass_option_with_distance(100.0)
            {
                let dist = (safe_target.position - ctx.player.position).magnitude();
                if dist >= 20.0 {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("DEF_PASSING_TIRED_SHORT")
                                .build(ctx),
                        )),
                    ));
                }
            }
        }

        // Normal passing situation - evaluate options more carefully
        // Defenders use shorter max distance (200 units) to avoid wild long passes
        if let Some((best_target, _reason)) = ctx
            .player()
            .passing()
            .find_best_pass_option_with_distance(200.0)
        {
            // ANTI-LOOP: Ensure pass target is far enough away for the ball to actually reach them.
            // Very short passes (< 30 units) with low pass force create claim-pass-reclaim loops.
            let pass_distance = (best_target.position - ctx.player.position).magnitude();

            // Also verify the pass isn't going backward toward own goal
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - ctx.player.position).normalize();
            let to_target = (best_target.position - ctx.player.position).normalize();
            let forward_component = to_target.dot(&to_goal);

            if pass_distance >= 30.0 && forward_component > -0.3 {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(best_target.id)
                            .with_reason("DEF_PASSING_NORMAL")
                            .build(ctx),
                    )),
                ));
            }
        }

        // If no good passing option and close to own goal, consider clearing
        if ctx.player().defensive().in_dangerous_position() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Clearing,
            ));
        }

        // If viable to dribble out of pressure (wait before bailing to prevent Running↔Passing oscillation)
        if ctx.in_state_time > 20 && self.can_dribble_effectively(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Time-based fallback - don't get stuck in this state too long
        if ctx.in_state_time > 50 {
            // If we've been in this state for a while, make a decision

            // Try to find a safe pass option (directionally aware) rather than any random teammate
            if let Some(safe_target) = ctx
                .player()
                .passing()
                .find_safe_pass_option_with_distance(200.0)
            {
                let dist = (safe_target.position - ctx.player.position).magnitude();
                if dist >= 20.0 {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("DEF_PASSING_TIMEOUT")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // If no safe option, clear the ball rather than making a wild pass
            if ctx.in_state_time > 65 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            // Otherwise start running with the ball
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // While holding the ball and looking for pass options, move slowly or stand still

        // If player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            // Calculate target position based on the defensive situation
            if let Some(target_position) =
                ctx.player().movement().calculate_better_passing_position()
            {
                return Some(
                    SteeringBehavior::Arrive {
                        target: target_position,
                        slowing_distance: 5.0, // Short distance for subtle movement
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        // Default to very slow movement or stationary
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing is a quick action with minimal physical effort - very low intensity
        DefenderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl DefenderPassingState {
    /// Detect a counter-attack opportunity and return the long-ball
    /// target if one exists. Returns `Some` only when:
    ///
    ///   1. The team has just won possession (≤50 ticks ago ~= 0.5s).
    ///      Beyond that the moment has passed — the opposition reorganises.
    ///   2. The opposition has over-committed: ≥5 opponents are in our
    ///      defensive half (we won the ball during their build-up).
    ///   3. A teammate is already in the opposition's half AND meaningfully
    ///      forward of the defender (50u..350u).
    ///
    /// All three gates have to fire. In normal possession phases the
    /// `ownership_duration` gate alone keeps the helper inert, so this
    /// doesn't affect calm build-up play — only the post-turnover moment
    /// where the canonical real-football response IS a long ball.
    fn find_counter_attack_target(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        // Recent possession only — beyond this the opposition has
        // reorganised and the long ball loses its counter-attack value.
        // 100 ticks ~= 1s gives the defender time to pop out of Standing
        // → Running → Passing while still inside the transition window.
        let ownership_duration = ctx.tick_context.ball.ownership_duration;
        if ownership_duration > 100 {
            return None;
        }

        let side = ctx.player.side?;
        let field_w = ctx.context.field_size.width as f32;
        let halfway_x = field_w * 0.5;

        // Over-commitment check: at least 5 opponents in our defensive
        // half. Under 5 means they're already organised in their own
        // half — counter wouldn't catch them out of position.
        let opponents_in_our_half = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| match side {
                PlayerSide::Left => opp.position.x < halfway_x,
                PlayerSide::Right => opp.position.x > halfway_x,
            })
            .count();
        if opponents_in_our_half < 5 {
            return None;
        }

        // Furthest-forward teammate within 300u.
        let target = ctx.players().teammates().nearby_to_opponent_goal()?;

        // Target must be in opposition's half — a teammate stuck in our
        // own half is no use as a counter outlet.
        let target_in_opp_half = match side {
            PlayerSide::Left => target.position.x > halfway_x,
            PlayerSide::Right => target.position.x < halfway_x,
        };
        if !target_in_opp_half {
            return None;
        }

        // Sensible long-ball range. Too short = not a counter (regular
        // build-up). Too long = unrealistic switch.
        let pass_distance = (target.position - ctx.player.position).magnitude();
        if !(50.0..=350.0).contains(&pass_distance) {
            return None;
        }

        // Lane-clearness gate. The counter outlet should only fire when
        // the long ball has a real chance of getting through; otherwise
        // we're spraying long passes into the opposition's spine where
        // strong midfielders sweep them up and we hand possession back
        // even faster than a safe sideways pass would. Allow at most
        // ONE opponent within 14u of the lane between passer and target.
        // Without this gate, the audit data showed weak-team pass
        // accuracy dropped 3% (every extra forward pass = lost ball)
        // and the counter never produced a shot.
        let to_target = target.position - ctx.player.position;
        let target_dir = to_target.normalize();
        let lane_length = to_target.magnitude();
        let opponents_in_lane = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| {
                let rel = opp.position - ctx.player.position;
                let along = rel.dot(&target_dir);
                if along < 5.0 || along > lane_length - 5.0 {
                    return false;
                }
                let proj = ctx.player.position + target_dir * along;
                (opp.position - proj).magnitude() < 14.0
            })
            .count();
        if opponents_in_lane > 1 {
            return None;
        }

        Some(target)
    }

    /// Determine if player can effectively dribble out of the current situation
    fn can_dribble_effectively(&self, ctx: &StateProcessingContext) -> bool {
        // Check player's dribbling skill
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;

        // Check if there's space to dribble into
        let opposition_ahead = ctx.players().opponents().nearby(20.0).count();

        // Defenders typically need more space and skill to dribble effectively
        dribbling_skill > 0.8 && opposition_ahead < 1
    }

    /// Determine if player should adjust position to find better passing angles
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        // Don't adjust if we've been in state too long
        if ctx.in_state_time > 40 {
            return false;
        }

        let under_immediate_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let has_clear_option = ctx.player().passing().find_best_pass_option().is_some();

        // Adjust position if not under immediate pressure and no clear options
        !under_immediate_pressure && !has_clear_option
    }

    /// Check if nearby teammates are tired (average condition below threshold)
    fn are_teammates_tired(&self, ctx: &StateProcessingContext) -> bool {
        let mut total_condition = 0u32;
        let mut count = 0u32;

        for teammate in ctx.players().teammates().nearby(150.0) {
            if let Some(player) = ctx.context.players.by_id(teammate.id) {
                total_condition += player.player_attributes.condition_percentage();
                count += 1;
            }
        }

        if count == 0 {
            return false;
        }

        let avg_condition = total_condition / count;
        avg_condition < 40
    }
}
