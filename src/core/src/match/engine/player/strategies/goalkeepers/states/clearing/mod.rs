use crate::r#match::Ball;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperPunt,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// Goalkeeper clearing state - emergency clearance of the ball away from danger
#[derive(Default, Clone)]
pub struct GoalkeeperClearingState {}

impl StateProcessingHandler for GoalkeeperClearingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we don't have the ball anymore, return to standing
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Execute the clearance kick
        if let Some(event) = self.execute_clearance(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                event,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Stand still while preparing to clear
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Clearing requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperClearingState {
    /// Execute a clearance — lofted hoof up the pitch, struck flat out.
    ///
    /// Old implementation used `MoveBall` with z=0, so the "clearance"
    /// was a ground roll that got intercepted 20m upfield. Now it emits
    /// a proper `ClearBall` event with a solved ballistic arc.
    ///
    /// # It also used to land in his own centre circle
    ///
    /// The arc was solved as `horizontal = distance / hang_ticks`, the
    /// drag-free inversion of gravity alone — and the ball is not
    /// drag-free. Integrated against the physics the engine actually runs
    /// ([`crate::r#match::engine::ball::ball::AIR_DRAG_PER_TICK`]), a ball
    /// solved to travel 335u travels **256u**: a clearance "aimed at the
    /// halfway line" from his own six-yard box came down around the edge
    /// of his own centre circle, 32 m out, with the opposition's midfield
    /// standing over it. `Ball::launch_for_range` inverts the real flight,
    /// so the aim point is now where the ball goes.
    ///
    /// And the aim point itself is no longer a fixed halfway line. An
    /// emergency clearance is hit as hard as the man can hit it; how far
    /// that is comes from the same leg model the punt uses
    /// ([`KeeperPunt::goal_kick_reach`]), so one goalkeeper has one
    /// kicking range whatever state he is in.
    ///
    /// # This kick used to leave the pitch entirely
    ///
    /// The vertical velocity here was a hand-written `4.5 + skill`, with
    /// a comment explaining that "in-engine gravity is strong, so z needs
    /// to be ~5 u/tick". That was true of the gravity it was written
    /// against. The vertical axis is now in METRES and gravity with it
    /// (see [`GRAVITY_PER_TICK`](crate::r#match::engine::ball::ball::GRAVITY_PER_TICK)),
    /// and 4.5 m/tick is 450 m/s straight up — an apex of about **ten
    /// kilometres** and a hang time of a minute and a half.
    ///
    /// What that looked like from the stands is every one of the reported
    /// symptoms at once: the ball leaves the keeper at colossal speed,
    /// climbs out of sight, crosses the whole pitch in a couple of
    /// seconds, sails over the opposite goal, hits the far boundary and
    /// gets clamped back inside with its velocity zeroed — at which point
    /// it drops from altitude to the turf in a single tick. `Clearing` is
    /// the terminal state of nearly every keeper possession (Catching,
    /// PickingUp, Passing, Kicking, Distributing and Throwing all fall
    /// through to it), so it happened constantly.
    ///
    /// The conversion sweep that moved the engine onto a metric vertical
    /// axis rewrote the defender's clearance, the headed clearance and the
    /// shot arc in terms of an APEX, and every one of those carries a
    /// comment warning that a hand-written `z` reads as a sane number
    /// while meaning something absurd. This site was missed. It is now
    /// solved the same way they are: choose how high the hoof goes, which
    /// fixes the hang time, and let the horizontal speed follow from the
    /// distance it has to cover.
    fn execute_clearance(&self, ctx: &StateProcessingContext) -> Option<Event> {
        let kicking_power = ctx.player.skills.goalkeeping.kicking / 20.0;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let mid_y = field_height * 0.5;

        let keeper_pos = ctx.player.position;
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());

        // As far as this leg will send it, from the gloves or off the
        // deck. One keeper, one kicking range.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let reach = if ctx.tick_context.ball.held_in_hands {
            KeeperPunt::punt_reach(ctx, &prof)
        } else {
            KeeperPunt::goal_kick_reach(ctx, &prof)
        };

        // Central-ish and off-centre at random, so the clearance lands
        // where the midfielders can contest it rather than near a
        // sideline, and so it is not predictable.
        let rng = &ctx.context.rng;
        let target_y = mid_y + rng.jitter(0.0, field_height * 0.15);
        // Never hoof it over the far goal line: a clearance that lands in
        // the opposition six-yard box is a free ball for their keeper.
        const GOAL_LINE_MARGIN: f32 = 90.0;
        let target_x = if forward > 0.0 {
            (keeper_pos.x + reach).min(field_width - GOAL_LINE_MARGIN)
        } else {
            (keeper_pos.x - reach).max(GOAL_LINE_MARGIN)
        };
        let target = Vector3::new(target_x, target_y, 0.0);

        // Apex of the hoof, in metres. A goalkeeper's kick from hand is
        // the highest ball in football — 20 m at the top of the arc is
        // normal, and a better striker of the ball gets it higher.
        let apex_metres = 16.0 + kicking_power * 8.0; // 16 - 24 m
        let launch_height = ctx
            .tick_context
            .ball
            .held_in_hands
            .then_some(1.15)
            .unwrap_or(0.0);
        let ball_velocity = Ball::ballistic_launch(keeper_pos, target, apex_metres, launch_height)?;

        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::KeeperReleaseDiag::note_clearance((target - keeper_pos).norm());

        Some(Event::PlayerEvent(PlayerEvent::ClearBall(ball_velocity)))
    }
}
