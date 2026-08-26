use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::CrossModel;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::{CrossDiag, WideDiag};
use nalgebra::Vector3;

/// Ticks of wind-up before the ball is struck. Matches the midfielder /
/// forward crossing states so the delivery cadence is role-independent.
const CROSS_EXECUTION_TIME: u64 = 5;

/// The overlapping fullback's delivery.
///
/// Defenders had no crossing state at all: a fullback who had carried the
/// ball into the attacking third could only pass, shoot or keep running,
/// so the single most common source of chances in modern football — the
/// overlap and cross — was unreachable from the defender state machine.
/// `PushingUp`, `Overlapping` and `Running` now hand off here when a wide
/// defender wins an advanced position with the ball.
///
/// # It used to hand-roll its own target finder, and that made it not a cross
///
/// This state carried a private `find_cross_target` — a second, simpler
/// copy of what [`CrossModel::pick`] does — and the event it emitted
/// carried neither `with_cross_type` nor `with_target_point`. Downstream
/// that is the difference between a CROSS and an ordinary pass: the
/// trajectory solver keeps an untyped ball low, the aerial contest
/// (`resolve_cross_contest`, which needs a descending ball at z 1.5-2.9)
/// never arms, and `CrossDiag` never sees it. So the one delivery in the
/// game that is supposed to be a full-back's whole reward for a 40 m
/// sprint arrived as a square ball along the floor, and every count of
/// "crosses struck" was blind to it.
///
/// It now uses the shared model, like the midfielder and forward states.
/// One definition of what a cross is, three roles that can strike one.
#[derive(Default, Clone)]
pub struct DefenderCrossingState {}

impl StateProcessingHandler for DefenderCrossingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Lost it in the wind-up — a fullback caught upfield gets back.
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TrackingBack,
            ));
        }

        if ctx.in_state_time > CROSS_EXECUTION_TIME {
            if let Some(decision) = CrossModel::pick(ctx) {
                #[cfg(feature = "match-logs")]
                {
                    CrossDiag::note(decision.cross_type);
                    let goal = ctx.player().opponent_goal_position();
                    WideDiag::note_delivery(
                        0,
                        (goal.x - ctx.player.position.x).abs(),
                        (ctx.player.position.y - goal.y).abs() < 165.0,
                    );
                }
                // Delivered — the fullback immediately recovers their
                // shape rather than loitering on the byline.
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::TrackingBack,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(decision.target_id)
                            .with_cross_type(decision.cross_type)
                            .with_target_point(decision.aim_point)
                            .with_reason("DEF_OVERLAP_CROSS")
                            .build(ctx),
                    )),
                ));
            }

            // Nobody in the box — recycle rather than hit a hopeful ball.
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Passing,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Planted over the ball for the delivery.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Whipping in a cross after an overlapping run is explosive work.
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
