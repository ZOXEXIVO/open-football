use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, Interception,
};
use crate::r#match::{
    ConditionContext, MATCH_TIME_MS, PlayerDistanceFromStartPosition, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// How close an assigned man has to be before a recovering defender
/// abandons the run to his slot and goes to him instead (~19 m). Same
/// figure `running` uses, so a duty means the same thing in every state
/// that holds it.
const MARK_RECOVERY_DISTANCE: f32 = 150.0;

const CLOSE_TO_START_DISTANCE: f32 = 10.0;
const BALL_INTERCEPTION_DISTANCE: f32 = 30.0;

#[derive(Default, Clone)]
pub struct DefenderTrackingBackState {}

impl StateProcessingHandler for DefenderTrackingBackState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // A player who has won the ball is not doing this any more.
        //
        // Nothing here asked whether HE had it, so a defender who
        // intercepted or was simply the nearest body when it arrived
        // carried on with an off-ball job while holding it — the same
        // fixed point `Defender: Marking` was measured freezing on
        // (99% of its stuck ticks with the owner 250-plus AI ticks into
        // the state). `Running` is where a defender's on-ball decisions
        // live, and this is the hand-off `DefenderStandingState` has
        // always made.
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // DUTY BEFORE POSITION — see the note in `returning`. A recovery
        // run with a man assigned is a run at that man, and this state is
        // literally the hard sprint back into shape, so it is the last
        // place the assignment should be dropped.
        if let Some(man) = ctx.team().my_mark() {
            if (man.position - ctx.player.position).magnitude() < MARK_RECOVERY_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Check if the defender has reached their starting position
        if ctx.player().distance_from_start_position() < CLOSE_TO_START_DISTANCE {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // Check if the ball is close and moving towards the player
        if Interception::is_available(ctx)
            && ctx.ball().distance() < BALL_INTERCEPTION_DISTANCE
            && ctx.ball().is_towards_player()
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        // If the team is losing and there's little time left, consider a
        // more aggressive stance. Window = the final ~1/15 of the match
        // (~6 min at full length) — the previous `- 300` subtracted 300
        // MILLISECONDS, so the branch effectively never fired.
        if ctx.team().is_loosing()
            && ctx.context.total_match_time > MATCH_TIME_MS - MATCH_TIME_MS / 15
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // If the player is tired, switch to a less demanding state
        if ctx.player().is_tired() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // If the ball is on the team's own side, prioritize defensive positioning
        if ctx.ball().on_own_side() {
            match ctx.player().position_to_distance() {
                PlayerDistanceFromStartPosition::Big => None, // Continue tracking back
                PlayerDistanceFromStartPosition::Medium => Some(
                    StateChangeResult::with_defender_state(DefenderState::HoldingLine),
                ),
                PlayerDistanceFromStartPosition::Small => Some(
                    StateChangeResult::with_defender_state(DefenderState::Standing),
                ),
            }
        } else {
            None // Continue tracking back if the ball is on the opponent's side
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let start_position = ctx.player.start_position;
        let distance_from_start = ctx.player().distance_from_start_position();

        // Calculate urgency based on game situation
        let urgency = if ctx.ball().on_own_side() {
            // Ball on own side - more urgent to get back
            1.5
        } else if ctx.team().is_loosing()
            && ctx.context.total_match_time > MATCH_TIME_MS - MATCH_TIME_MS / 15
        {
            // Losing late in game - less urgent to defend
            0.8
        } else {
            1.0
        };

        // Use Arrive behavior with slowing distance based on how far we are
        let slowing_distance = if distance_from_start > 50.0 {
            15.0 // Far away - longer slowing distance
        } else {
            10.0 // Close - shorter slowing distance
        };

        Some(
            SteeringBehavior::Arrive {
                target: start_position,
                slowing_distance,
            }
            .calculate(ctx.player)
            .velocity
                * urgency,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tracking back is a hard recovery run to get goal-side — high
        // intensity (the player is literally running back, not jogging).
        // Tracks a MOVING MAN, so the cap has to match the run he is
        // tracking — see the note in `defenders/states/marking`. `High`
        // (0.78) against an attacker's `VeryHigh` (0.95) is a marker
        // forbidden to keep up by the movement layer.
        DefenderCondition::with_velocity(ActivityIntensity::VeryHigh).process(ctx);
    }
}
