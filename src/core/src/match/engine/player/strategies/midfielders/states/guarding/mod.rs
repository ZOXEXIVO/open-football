use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, Interception, MidfielderCondition,
};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::common::states::ContactFoul;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

const GUARD_DISTANCE: f32 = 25.0; // Keep a realistic marking distance (don't sit on top of opponent)
/// …and how tight that becomes on a man deep in our own third. 10u =
/// 1.25 m — touch-tight, the distance the back line's own
/// `ideal_marking_distance` already asks for. See the note at the call site.
const TIGHT_GUARD_DISTANCE: f32 = 10.0;
/// Fraction of the pitch, measured from our own goal, over which
/// `danger` ramps from 0 to 1. 0.30 ≈ 31 m: our defensive third and the
/// approach to it, which is where a screening job becomes a marking one.
const DANGER_SPAN: f32 = 0.30;
/// How strongly a guard is pulled back toward his own tactical slot when
/// the man he is on poses no immediate danger. Faded to zero as `danger`
/// rises — see the call site.
const TETHER_STRENGTH: f32 = 0.2;
const MAX_GUARD_RANGE: f32 = 100.0; // Give up guarding if attacker moves too far
/// Range within which a midfielder will TAKE UP a guard, as opposed to
/// the wider `MAX_GUARD_RANGE` at which he gives one up.
///
/// The two used to be the same number, which makes the boundary a
/// flicker generator: `find_guard_target` scans `nearby(MAX_GUARD_RANGE)`,
/// so a target at 99u was pickable, and `Guarding` abandons anything past
/// 100u — one stride either way flipped the decision. `Midfielder:
/// Guarding <-> Midfielder: Returning` survived every other fix on this
/// pair and was still running ~2,300 round trips a match on it alone
/// (`dev_match trace`). Committing only inside the tighter radius leaves
/// a 20u band in which whatever the player is already doing stands.
const GUARD_COMMIT_RANGE: f32 = 80.0;
const TACKLE_TRANSITION_DISTANCE: f32 = 15.0; // Tackle if opponent receives ball nearby
const STAMINA_THRESHOLD: f32 = 15.0;
const PREDICTION_TIME: f32 = 0.25;
const MAX_DISTANCE_FROM_START: f32 = 150.0; // Don't follow opponent too far from tactical zone
const BOUNDARY_MARGIN: f32 = 15.0; // Stay away from field edges

#[derive(Default, Clone)]
pub struct MidfielderGuardingState {}

impl StateProcessingHandler for MidfielderGuardingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // The shirt pull — see `ContactFoul` and the defender marking
        // state, which carries the same block for the same reason.
        if ContactFoul::is_decision_tick(ctx) {
            if let Some(man) = self.find_guard_target(ctx) {
                let gap = (man.position - ctx.player.position).magnitude();
                let losing_him = man.velocity(ctx).norm() > ctx.player.velocity.norm() + 0.08;
                let p = ContactFoul::probability(ctx, gap, losing_him);
                if ctx.context.rng.bernoulli(p) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::CommitFoul(
                            ctx.player.id,
                            ContactFoul::severity(ctx, losing_him),
                        )),
                    ));
                }
            }
        }

        // If we have the ball, run with it
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Team regained possession — support attack
        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // Stamina check
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // Press opponent with ball if nearby — midfielders must engage
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let dist = opponent_with_ball.distance(ctx);
            // Close — tackle aggressively
            if dist < 25.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            // Only the best-positioned player presses further out
            if dist < 100.0 && ctx.team().is_best_player_to_chase_ball() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // Find the opponent to guard
        let guard_target = self.find_guard_target(ctx);

        if let Some(opponent) = guard_target {
            let distance = opponent.distance(ctx);

            // Opponent received the ball — react
            if opponent.has_ball(ctx) {
                if distance < TACKLE_TRANSITION_DISTANCE {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // Ball coming toward guarded opponent — intercept
            if Interception::is_available(ctx) && ctx.ball().distance() < 80.0 && ctx.ball().is_towards_player_with_angle(0.7) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // Opponent too far — give up guarding
            if distance > MAX_GUARD_RANGE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Ball far away on opponent's side — no need to guard
            if !ctx.ball().on_own_side() && ctx.ball().distance() > 300.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }

            // Don't follow opponent too far from tactical position
            let dist_from_start = (ctx.player.position - ctx.player.start_position).magnitude();
            if dist_from_start > MAX_DISTANCE_FROM_START {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Don't get stuck at the boundary following an opponent
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;
            let pos = ctx.player.position;
            let at_boundary = pos.x < BOUNDARY_MARGIN
                || pos.x > field_width - BOUNDARY_MARGIN
                || pos.y < BOUNDARY_MARGIN
                || pos.y > field_height - BOUNDARY_MARGIN;

            if at_boundary {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Continue guarding from distance
            None
        } else {
            // No one to guard — go to Running (NOT Returning, which would loop back here)
            Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ))
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(opponent) = self.find_guard_target(ctx) {
            let opponent_velocity = opponent.velocity(ctx);
            let own_goal = ctx.ball().direction_to_own_goal();

            // Predict where opponent is heading
            let opponent_future = opponent.position + opponent_velocity * PREDICTION_TIME;

            // ── How tight, and how tethered — both scale with danger ──
            //
            // A midfielder tracking a runner into our penalty area is
            // marking, not screening, and the two are different jobs at
            // different distances. Measured before this: midfield runners
            // were held at **9.8 m** by the man nominally marking them
            // (forwards, marked by defenders, sat at 2.8 m), and
            // midfielders went on to take 63% of every shot struck from
            // inside six metres.
            //
            // Both halves of that gap are here. A flat 25u is 3.1 m — a
            // screening distance, fine at the edge of the middle third
            // and nowhere near a man about to receive on the penalty
            // spot; it now closes to 10u (1.25 m) as the man he is
            // marking gets deep, which is what the defenders'
            // `ideal_marking_distance` already asks of them.
            //
            // And the tether was a LERP toward `start_position` — the
            // KICKOFF slot. Blending 20% of a fixed point 40 m upfield
            // into the target pulls the marker off his man in proportion
            // to how deep the man has run, which is exactly backwards:
            // the deeper the run, the more dangerous it is and the more
            // this let go of it. On a runner arriving at the penalty spot
            // it accounted for most of the 9.8 m on its own. It fades out
            // over the same band the marking distance tightens across, so
            // near our own goal the man wins outright — the same rule the
            // back line now follows in `DefensiveLine::hold_shape`.
            let field_len = ctx.context.field_size.width as f32;
            let danger = (1.0
                - (opponent_future.x - own_goal.x).abs() / (field_len * DANGER_SPAN).max(1.0))
            .clamp(0.0, 1.0);
            let guard_distance = GUARD_DISTANCE + (TIGHT_GUARD_DISTANCE - GUARD_DISTANCE) * danger;

            // Position between opponent and our goal at guard_distance away
            let to_goal = (own_goal - opponent_future).normalize();
            let desired_position = opponent_future + to_goal * guard_distance;

            // Blend with tactical position to avoid straying too far
            let tether_strength = TETHER_STRENGTH * (1.0 - danger);
            let desired_position = desired_position * (1.0 - tether_strength)
                + ctx.player.start_position * tether_strength;

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            // Dead zone: close enough — hold position, no jitter
            if distance < 8.0 {
                return Some(Vector3::zeros());
            }

            let direction = to_desired.normalize();

            // Speed based on how far off position we are
            let base_speed = ctx.player.skills.physical.pace * 0.4;
            let urgency = (distance / GUARD_DISTANCE).clamp(0.4, 1.0);

            Some(direction * base_speed * urgency)
        } else {
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Guarding requires constant movement — high intensity
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderGuardingState {
    /// Find the best opponent to guard — memoized per (player, tick):
    /// `process()` and `velocity()` both run the scored scan within one
    /// tick over tick-frozen inputs, so the second call returns the
    /// identical pick (debug oracle on every hit). Shares the
    /// `guard_target` slot with the defender variant — a player is in
    /// exactly one role-specific state per tick.
    /// The attacker this midfielder would pick up.
    ///
    /// `pub(crate)` so the states that hand players INTO `Guarding` can ask
    /// the same question first. `MidfielderRunningState` used to route on
    /// ball position alone ("ball on our side and further than 100u"),
    /// with no reference to whether there was anybody to guard — and this
    /// state's only answer to "nobody to guard" is to hand the player
    /// straight back. That pair ran ~7,400 round trips a match
    /// (`dev_match trace`). The result is memoised per tick in
    /// `player_agg_cache`, so the extra call from the sending state is a
    /// cache hit, not a second scan.
    /// A guard target worth STARTING on — inside `GUARD_COMMIT_RANGE`.
    ///
    /// This is what the states that hand players into `Guarding` should
    /// ask; `find_guard_target` itself scans out to the wider give-up
    /// radius, so using it directly to decide entry puts the commit and
    /// the abandon condition on the same boundary.
    pub(crate) fn find_committable_guard_target(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        self.find_guard_target(ctx)
            .filter(|t| t.distance(ctx) <= GUARD_COMMIT_RANGE)
    }

    pub(crate) fn find_guard_target(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let tick = ctx.current_tick();
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .guard_target;
        match cached {
            Some(target) => {
                debug_assert_eq!(
                    target.map(|p| p.id),
                    self.compute_find_guard_target(ctx).map(|p| p.id),
                    "guard-target memo mismatch (midfielder)"
                );
                target
            }
            None => {
                let target = self.compute_find_guard_target(ctx);
                ctx.tick_context
                    .player_agg_cache
                    .borrow_mut()
                    .slot_mut(ctx.player.id, tick)
                    .guard_target = Some(target);
                target
            }
        }
    }

    /// The scored scan behind [`find_guard_target`](Self::find_guard_target)
    /// — attackers without ball trying to find space.
    fn compute_find_guard_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        // THE ASSIGNMENT WINS.
        //
        // The team plan hands out man-marking duties exclusively across
        // the whole defensive unit; picking a target locally instead
        // means two midfielders can converge on the same opponent while
        // another goes free, which is the failure the plan exists to
        // prevent. The local scan below stays as the fallback for a
        // midfielder the plan gave nobody.
        if let Some(man) = ctx.team().my_mark() {
            return Some(man);
        }

        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;
        // Our grid-stored position, fetched once — Factor 5 used to
        // re-probe both ids through `grid.get` per candidate.
        let my_grid_pos = ctx.tick_context.grid.position_of(ctx.player.id);

        let mut best_target: Option<MatchPlayerLite> = None;
        let mut best_score = f32::MIN;

        for opponent in ctx.players().opponents().nearby(MAX_GUARD_RANGE) {
            // Skip the ball carrier
            if opponent.has_ball(ctx) {
                continue;
            }

            let mut score = 0.0;

            // Factor 1: Proximity to our goal
            let dist_to_goal = (opponent.position - own_goal).magnitude();
            score += (400.0 - dist_to_goal.min(400.0)) / 8.0;

            // Factor 2: Proximity to ball (can receive passes)
            let dist_to_ball = (opponent.position - ball_position).magnitude();
            score += (200.0 - dist_to_ball.min(200.0)) / 8.0;

            // Factor 3: Movement toward our goal
            let velocity = opponent.velocity(ctx);
            let speed = velocity.norm();
            if speed > 1.0 {
                let move_dir = velocity.normalize();
                let to_goal = (own_goal - opponent.position).normalize();
                let alignment = move_dir.dot(&to_goal);
                if alignment > 0.0 {
                    score += alignment * speed * 8.0;
                }
            }

            // Factor 4: Unmarked bonus — no defender or midfielder covering
            // this attacker. "Opponents of the opponent" are our teammates,
            // so query our team around the candidate's grid position
            // directly — same entry set/order as `grid.opponents(opponent.
            // id, 15.0)` (the Lite's position IS the grid-stored one),
            // minus the per-candidate id probe.
            let has_nearby_cover = ctx
                .tick_context
                .grid
                .teammates_full(
                    opponent.id,
                    ctx.player.team_id,
                    opponent.position,
                    0.0,
                    15.0,
                )
                .any(|(gp, _)| gp.id != ctx.player.id);

            if !has_nearby_cover {
                score += 35.0;
            }

            // Factor 5: Closeness to us — same 2D math as `grid.get`
            // (grid-stored positions, identical operand order).
            let dx = opponent.position.x - my_grid_pos.x;
            let dy = opponent.position.y - my_grid_pos.y;
            let dist_to_us = (dx * dx + dy * dy).sqrt();
            score += (60.0 - dist_to_us.min(60.0)) / 3.0;

            if score > best_score {
                best_score = score;
                best_target = Some(opponent);
            }
        }

        best_target
    }
}
