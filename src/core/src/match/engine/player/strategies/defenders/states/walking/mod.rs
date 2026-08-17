use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, Interception,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior, VectorExtensions,
};
use nalgebra::Vector3;

const INTERCEPTION_DISTANCE: f32 = 150.0;
const MARKING_DISTANCE: f32 = 50.0;
const PRESSING_DISTANCE: f32 = 80.0;
const TACKLE_DISTANCE: f32 = 25.0;

#[derive(Default, Clone)]
pub struct DefenderWalkingState {}

impl StateProcessingHandler for DefenderWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Attacking corner: centre-backs push up to attack the delivery.
        if !ctx.player.has_ball(ctx)
            && ctx
                .player
                .tactical_position
                .current_position
                .is_central_defender()
            && ctx.ball().is_team_attacking_corner()
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::AttackingCorner,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // Loose-ball claim lives in the dispatcher.

        // Priority 1: Check for opponents with the ball nearby - be aggressive!
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = ctx.player.position.distance_to(&opponent.position);

            // Tackle if very close
            if distance_to_opponent < TACKLE_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            // Press if nearby
            if distance_to_opponent < PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // Mark if within marking range
            if distance_to_opponent < MARKING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Priority 2: Check for nearby opponents without the ball to mark
        if let Some(opponent_to_mark) = ctx.players().opponents().without_ball().next() {
            let distance = ctx.player.position.distance_to(&opponent_to_mark.position);
            if distance < MARKING_DISTANCE / 2.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Priority 2.5: When ball is on own side and opponent advancing, provide cover
        if ctx.ball().on_own_side() {
            if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                let distance = opponent.distance(ctx);
                if distance < 120.0 {
                    // Close enough to press or support
                    if distance < PRESSING_DISTANCE {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Pressing,
                        ));
                    }
                    // Provide cover depth — position between attacker and goal
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Covering,
                    ));
                }
            }
        }

        // Priority 3: Intercept ball if it's coming towards player
        if Interception::is_available(ctx)
            && ctx.ball().is_towards_player_with_angle(0.8)
            && ctx.ball().distance() < INTERCEPTION_DISTANCE
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        // Priority 4: Recovery run when he is a long way out of the block
        // and nothing needs him where he stands. Anchor-relative, so
        // "out of position" tracks the block as it slides rather than
        // measuring against the spot he kicked off from.
        const RECOVERY_RUN: f32 = 100.0;
        if ctx.team().distance_from_anchor() > RECOVERY_RUN && !self.has_nearby_threats(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // Anything short of that is walked off by this state's own
        // velocity, which steers at the same anchor.
        //
        // What used to be here was a `PlayerEvent::MovePlayer` — and that
        // event **assigns `player.position` directly** (see
        // `handle_move_player_event`). So a walking defender was TELEPORTED
        // every tick he was more than 2u (25 cm) from
        // `team_centroid * 0.7 + ball * 0.3`: a destination every defender
        // computes identically, reached instantly, ignoring his own speed,
        // his velocity, and the collision clamp. Four defenders were being
        // snapped onto one point, continuously, whenever they were not
        // marking or pressing.

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Check if player should follow waypoints
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                // Player has waypoints defined, follow them
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        crowd_offset: ctx.player().separation_offset(),
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        // Walk into the team's live anchor. Two things used to happen
        // here and neither was football: a periodic `Wander` around the
        // kickoff dot, and a fallback that steered at
        // `team_centroid * 0.7 + ball * 0.3` — a quantity every defender
        // computes identically, so all four walked at the same point.
        // The anchor is per-player and exclusive by construction.
        let anchor = ctx.team().my_anchor();
        let to_anchor = anchor - ctx.player.position;
        if to_anchor.magnitude() < 5.0 {
            return Some(Vector3::zeros());
        }

        Some(
            SteeringBehavior::Arrive {
                target: anchor,
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity
                * 0.45,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking at low speed allows some recovery, velocity-based to account for pace
        DefenderCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}

impl DefenderWalkingState {
    fn has_nearby_threats(&self, ctx: &StateProcessingContext) -> bool {
        let threat_distance = 20.0; // Adjust this value as needed

        if ctx.players().opponents().exists(threat_distance) {
            return true;
        }

        // Check if the ball is close and moving towards the player
        let ball_distance = ctx.ball().distance();
        let ball_speed = ctx.ball().speed();
        let ball_towards_player = ctx.ball().is_towards_player();

        // Per-tick ball speeds are single-digit (shots ~1-2, lofted
        // clearances ~6-7), so the threat gate keys on "meaningfully
        // moving", not the old unreachable > 10.0.
        if ball_distance < threat_distance && ball_speed > 2.0 && ball_towards_player {
            return true;
        }

        false
    }
}
