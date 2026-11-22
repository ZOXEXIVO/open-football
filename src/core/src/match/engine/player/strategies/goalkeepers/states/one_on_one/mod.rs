use crate::r#match::goalkeepers::states::shared::{
    ActivityIntensity, GoalkeeperCondition, KeeperOneOnOne, KeeperRead, ReadLevel,
};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::KeeperDuelDiag;
use nalgebra::Vector3;

/// **A man through on goal, met with one plan.**
///
/// He closes the carrier down to the stand-off point on the line from his
/// goal to the ball (`KeeperOneOnOne::point`) and holds it there, set,
/// rather than running through it at a man who has the ball under control.
/// The smother takes over once the ball is inside his spread, and a strike
/// hands him to the save. Whichever state he was in when the breakaway
/// began, this is where he plays it from.
#[derive(Default, Clone)]
pub struct GoalkeeperOneOnOneState {}

impl GoalkeeperOneOnOneState {
    /// He gives the duel this long before a failed read can end it: 0.2 s,
    /// so a stutter in the carrier's run does not send him back and forth.
    const DISENGAGE_TICKS: u64 = 10;
    /// At the point he stands still: 2u = 25 cm.
    const SET_AT: f32 = 2.0;

    /// Book why the duel ended — see [`KeeperDuelDiag`] — and whether it
    /// ended inside 300 ms of starting.
    #[inline]
    fn note_exit(_ctx: &StateProcessingContext, _reason: usize) {
        #[cfg(feature = "match-logs")]
        KeeperDuelDiag::note_exit(_reason, _ctx.in_state_time <= 15);
    }
}

impl StateProcessingHandler for GoalkeeperOneOnOneState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // The ball his, a shot struck, the ball in his spread.
        if let Some(change) = KeeperRead::above(ctx, ReadLevel::OneOnOne).go() {
            Self::note_exit(ctx, 0);
            return Some(change);
        }
        if ctx.in_state_time < Self::DISENGAGE_TICKS {
            return None;
        }
        let lapse = KeeperOneOnOne::lapse(ctx)?;
        Self::note_exit(ctx, lapse as usize + 1);
        KeeperRead::from(ctx, ReadLevel::Aerial).go()
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let point = KeeperOneOnOne::point(ctx, &prof);
        if (point - ctx.player.position).magnitude() < Self::SET_AT {
            return Some(Vector3::zeros());
        }
        let speed_boost =
            (1.6 + prof.shot_stopping * 0.6 + prof.dive_reach * 0.5) * prof.explosive_mult;
        Some(
            SteeringBehavior::Arrive {
                target: point,
                slowing_distance: 8.0,
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
