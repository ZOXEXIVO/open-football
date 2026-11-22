use crate::r#match::defenders::states::DefenderMarkingState;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead, ReadLevel,
};
use crate::r#match::player::events::{FoulCommitted, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::shared::players::ops::marker_evasion::MarkerEvasion;
use crate::r#match::player::strategies::shared::states::{
    ContactFoul, MarkEngagement, StationKeeping,
};
use crate::r#match::{
    ConditionContext, MatchContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **Marking his man** — the one the plan gave him or, with none, the most
/// dangerous man near him the plan has given nobody (`MidfielderRead::man`).
/// Goal-side of him at the distance the back line stands off a man in the
/// same place, no further from his own place than his bound outside his
/// own area. He gives him up beyond `MarkEngagement::RELEASE`, to the
/// bound, and to every door above the mark.
#[derive(Default, Clone)]
pub struct MidfielderGuardingState {}

impl MidfielderGuardingState {
    /// How far ahead of his man a marker reads the run, on an even contest:
    /// effectively where the man is.
    const PREDICTION_TIME: f32 = 0.25;
}

impl StateProcessingHandler for MidfielderGuardingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = MidfielderRead::change_above(ctx, ReadLevel::Mark) {
            return Some(change);
        }
        let Some(man) = MidfielderRead::man(ctx) else {
            return Some(MidfielderRead::change(ctx));
        };
        let gap = man.distance(ctx);
        if gap > MarkEngagement::RELEASE || MidfielderRead::refuses(ctx, &man) {
            return Some(MidfielderRead::change(ctx));
        }
        // The shirt pull — see `ContactFoul` and the defender marking
        // state, which carries the same block for the same reason. Only
        // while the opposition has the ball, which the read has already
        // asked: there is no man to hold once his own side has it.
        let losing_him = man.velocity(ctx).norm() > ctx.player.velocity.norm() + 0.08;
        if ContactFoul::commits_now(ctx, gap, losing_him) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::HoldingShape,
                Event::PlayerEvent(PlayerEvent::CommitFoul(FoulCommitted {
                    fouler: ctx.player.id,
                    fouled: man.id,
                    severity: ContactFoul::severity(ctx, losing_him),
                    source: FoulSource::Holding,
                    challenge: None,
                })),
            ));
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let Some(man) = MidfielderRead::man(ctx) else {
            return Some(Vector3::zeros());
        };
        let profile = DefenderSkillProfile::from_ctx(ctx);
        // Switched off for a moment as his man goes — see the marking
        // state.
        if profile.late_to_the_run(ctx, &man) {
            return Some(ctx.player.velocity);
        }
        let own_goal = ctx.ball().direction_to_own_goal();

        // Predict where his man is heading — as a DUEL, the same contest the
        // back line's marking state runs and the same shared blends
        // (`MarkerEvasion::holder_quality` vs `mover_quality`): a poor
        // reader on a tricky mover aims a read-span behind the man, an
        // elite reader arrives a span ahead.
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let mover_q = ctx
            .context
            .players
            .by_id(man.id)
            .map(|m| MarkerEvasion::mover_quality(m, minute))
            .unwrap_or(0.5);
        let read_edge = (MarkerEvasion::holder_quality(ctx.player) - mover_q).clamp(-0.5, 0.5);
        let prediction = Self::PREDICTION_TIME + read_edge * 2.0 * MarkerEvasion::read_span();
        let man_future = man.position + man.velocity(ctx) * prediction;

        let stand_off =
            DefenderMarkingState::stand_off(own_goal, man.position, profile.ideal_marking_distance);
        let to_goal = (own_goal - man_future)
            .try_normalize(0.01)
            .unwrap_or_default();
        let mut desired = man_future + to_goal * stand_off;

        // The bound from his place, released inside his own area.
        let in_own_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
            .contains(&man.position);
        if !in_own_area {
            let place = ctx.team().my_anchor();
            let from_place = desired - place;
            if from_place.magnitude() > MidfielderRead::BOUND {
                desired = place + from_place.normalize() * MidfielderRead::BOUND;
            }
        }

        // He moves with his man, faster only by what closing the gap needs,
        // and never faster than he can.
        if !MatchContext::midfielder_mark_pace_off() {
            let hold = StationKeeping::hold(man.velocity(ctx), desired - ctx.player.position);
            let top = ctx.player.max_speed_with_condition_cached();
            return Some(hold.cap_magnitude(top));
        }

        // Dead zone: close enough — hold position, no jitter
        if (desired - ctx.player.position).magnitude() < 8.0 {
            return Some(Vector3::zeros());
        }

        // Flat out while he is a stand-off or more from his point, easing
        // as he arrives on it.
        Some(
            SteeringBehavior::Arrive {
                target: desired,
                slowing_distance: stand_off.max(8.0),
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tracks a MOVING MAN, so the cap has to match the run he is
        // tracking — see the note in `defenders/states/marking`. `High`
        // (0.78) against an attacker's `VeryHigh` (0.95) is a marker
        // forbidden to keep up by the movement layer.
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
