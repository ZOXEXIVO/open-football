use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_PASS_DURATION: u64 = 30; // Ticks before trying alternative action (reduced for faster decision-making)
const MIN_POSITION_ADJUSTMENT_TIME: u64 = 5; // Minimum ticks before adjusting position (prevents immediate twitching)
const MAX_POSITION_ADJUSTMENT_TIME: u64 = 20; // Maximum ticks to spend adjusting position

#[derive(Default, Clone)]
pub struct ForwardPassingState {}

impl StateProcessingHandler for ForwardPassingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Running state
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // Very close to goal — ask whether the shot is actually on, using
        // the SAME helper `ForwardShootingState` will ask on arrival.
        //
        // This used to be a bare `distance < 40 && has_clear_shot()`, which
        // is not the question the shooting state answers. `Shooting` runs
        // `evaluate_forward_shot_decision` as its last-mile check and, when
        // that says pass, sends the forward straight back here — where the
        // crude geometric test still said shoot. The two never agreed, and
        // `Forward: Passing <-> Forward: Shooting` became the single
        // largest remaining loop in the engine at ~14,400 round trips per
        // match (`dev_match trace`). Because both are on-ball states they
        // are also exempt from the decision-commitment guard, by design —
        // an on-ball picture must stay reactive — so nothing else was
        // damping it.
        //
        // Routing through the helper means the decision is made once, with
        // the xG / pass-EV / clear-shot gates the rest of the engine uses,
        // and `with_shot_reason` carries it forward so `Shooting` honours
        // the verdict instead of re-rolling it (see `dispatch_shot` in the
        // dribbling state for the same pattern).
        //
        // Asked ONCE, on entry. The helper rolls shot willingness, so
        // calling it every tick would turn a single chance into a per-tick
        // lottery the forward eventually wins by attrition — the
        // "resolve once per opportunity, never a per-tick lottery" rule
        // the rest of the shot code follows. A chance is one chance; the
        // player looks up, decides, and commits for this visit.
        // `MAX_PASS_DURATION` bounds how long that is, and losing the ball
        // or a defender closing still exits below.
        //
        // Measured over 5x60 matches, adding this shot path costs nothing
        // in goals (4.74 -> 5.01 per match, inside a +/-0.3 run-to-run
        // spread) while removing the loop.
        if distance_to_goal < 40.0 && ctx.in_state_time == 0 {
            match evaluate_forward_shot_decision(ctx, "FWD_PASSING_CLOSE") {
                ShotDecision::Shoot { reason } => {
                    return Some(
                        StateChangeResult::with_forward_state(ForwardState::Shooting)
                            .with_shot_reason(reason),
                    );
                }
                // Helper says the shot isn't on — carry on looking for the
                // pass, which is what this state is for.
                ShotDecision::Pass | ShotDecision::Hold => {}
            }
        }

        // Brief scanning delay before executing pass (unless under pressure)
        let under_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let min_scan_time = if under_pressure { 3 } else { 8 };

        if ctx.in_state_time >= min_scan_time
            && let Some((target_teammate, _)) = ctx.player().passing().find_best_pass_option()
        {
            // Execute the pass
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::builder()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target_teammate.id)
                        .with_reason("FWD_PASSING_STATE")
                        .build(ctx),
                )),
            ));
        }

        // No good pass option found. Hysteresis against Dribbling: we
        // only route BACK to Dribbling if a defender is VERY close (<8u,
        // tight enough that we need to beat them) AND we've had a real
        // scan window. A single chaser at 15-20u isn't "close enough to
        // need dribbling" — keep running with the ball. The old 20u
        // trigger flickered against Dribbling's 15u "no space" rule.
        if distance_to_goal < 200.0 {
            let very_close_defender = ctx.players().opponents().exists(8.0);
            return if very_close_defender && ctx.in_state_time >= 15 {
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ))
            } else {
                Some(StateChangeResult::with_forward_state(ForwardState::Running))
            };
        }

        // If under excessive pressure, consider going back to dribbling
        if self.is_under_heavy_pressure(ctx) {
            if self.can_dribble_effectively(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            } else {
                return Some(StateChangeResult::with_forward_state(ForwardState::Running));
            }
        }

        if ctx.in_state_time > MAX_PASS_DURATION {
            // Timeout — drop back to Running to reassess; HoldingUpPlay
            // was a dead state that only proxied Passing behaviour.
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If the player should adjust position to find better passing angles
        if self.should_adjust_position(ctx) {
            // Look for space to move into
            let steering_velocity = SteeringBehavior::Arrive {
                target: self.calculate_better_passing_position(ctx),
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity;

            // Apply reduced separation to avoid interference with deliberate movement
            let separation = ctx.player().separation_velocity() * 0.3;

            return Some(steering_velocity + separation);
        }

        None
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing is low intensity - minimal fatigue
        ForwardCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl ForwardPassingState {
    /// Check if player is under heavy pressure from opponents
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_heavy_pressure()
    }

    /// Determine if player can effectively dribble out of pressure.
    /// Dribbling+agility blended via two sigmoid pivots (both at 10/20)
    /// so the full 1-20 range maps to a smooth probability instead of a
    /// hard `> 0.5` cliff that collapsed the whole lower half.
    fn can_dribble_effectively(&self, ctx: &StateProcessingContext) -> bool {
        let has_space = !ctx.players().opponents().exists(15.0);
        if !has_space {
            return false;
        }
        let drib_p =
            SkillCurve::new(ctx.player.skills.technical.dribbling, 10.0, 0.6).probability();
        let agi_p = SkillCurve::new(ctx.player.skills.physical.agility, 10.0, 0.6).probability();
        // Weighted blend matches old `drib*0.7 + agi*0.3`.
        let combined = drib_p * 0.7 + agi_p * 0.3;
        ctx.context.rng.unit_f32() < combined
    }

    /// Determine if player should adjust position to find better passing angles
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        // Only adjust position within a specific time window to prevent endless twitching
        let in_adjustment_window = ctx.in_state_time >= MIN_POSITION_ADJUSTMENT_TIME
            && ctx.in_state_time <= MAX_POSITION_ADJUSTMENT_TIME;

        // If no good passing option and not under immediate pressure and within time window
        in_adjustment_window
            && ctx.player().passing().find_best_pass_option().is_none()
            && !self.is_under_heavy_pressure(ctx)
    }

    /// Calculate a better position for finding passing angles - forwards look for
    /// spaces that open up shooting opportunities first, passing lanes second
    fn calculate_better_passing_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Get positions
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // First priority: move to a better shooting position if possible
        if ctx.ball().distance_to_opponent_goal() < 250.0 {
            // Look for space between defenders toward goal
            if let Some(space) = self.find_space_between_opponents_toward_goal(ctx) {
                return space;
            }
        }

        // Second priority: find space for a better passing angle
        let closest_teammate = ctx.players().teammates().nearby(150.0).next();

        if let Some(teammate) = closest_teammate {
            // Find a position that improves angle to this teammate
            let to_teammate = teammate.position - player_pos;
            let teammate_direction = to_teammate.normalize();

            // Move slightly perpendicular to create a better angle
            let perpendicular = Vector3::new(-teammate_direction.y, teammate_direction.x, 0.0);
            let adjustment = perpendicular * 5.0; // Reduced from 8.0 to prevent excessive twitching

            return player_pos + adjustment;
        }

        // Default to moving toward goal if no better option
        let to_goal = goal_pos - player_pos;
        let goal_direction = to_goal.normalize();
        player_pos + goal_direction * 5.0 // Reduced from 10.0 to prevent excessive movement
    }

    /// Look for space between defenders toward the goal
    fn find_space_between_opponents_toward_goal(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal_direction = (goal_pos - player_pos).normalize();

        // Collect opponent POSITIONS only (24 bytes each), not full player
        // refs — we only need positions for the O(n²) gap scan.
        let goal_distance = (goal_pos - player_pos).magnitude();
        // Inline storage: at most 11 opponents pass the projection
        // filter (one team), and we only need positions for the gap
        // scan. Skips the per-call Vec allocation.
        const MAX_OPPONENT_POSITIONS: usize = 11;
        let mut opponent_positions: [Vector3<f32>; MAX_OPPONENT_POSITIONS] =
            [Vector3::zeros(); MAX_OPPONENT_POSITIONS];
        let mut opponent_positions_len: usize = 0;
        for opp in ctx.players().opponents().all() {
            let to_opp = opp.position - player_pos;
            let projection = to_opp.dot(&to_goal_direction);
            if projection > 0.0 && projection < goal_distance {
                if opponent_positions_len >= MAX_OPPONENT_POSITIONS {
                    break;
                }
                opponent_positions[opponent_positions_len] = opp.position;
                opponent_positions_len += 1;
            }
        }

        if opponent_positions_len < 2 {
            return None;
        }

        // Find the pair of opponents with the largest gap between them
        let mut best_gap = None;
        let mut max_gap_width = 0.0;

        for i in 0..opponent_positions_len {
            for j in i + 1..opponent_positions_len {
                let pos_i = opponent_positions[i];
                let pos_j = opponent_positions[j];

                let midpoint = (pos_i + pos_j) * 0.5;
                let gap_width = (pos_i - pos_j).magnitude();

                let to_midpoint = midpoint - player_pos;
                let dot_product = to_midpoint.dot(&to_goal_direction);

                if dot_product > 0.0 && gap_width > max_gap_width {
                    max_gap_width = gap_width;
                    best_gap = Some(midpoint);
                }
            }
        }

        best_gap
    }
}
