use crate::r#match::Ball;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
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
    /// Execute a clearance — lofted hoof toward the halfway line.
    ///
    /// Old implementation used `MoveBall` with z=0, so the "clearance"
    /// was a ground roll that got intercepted 20m upfield. Now it emits
    /// a proper `ClearBall` event with a solved ballistic arc.
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
        use crate::r#match::PlayerSide;

        let kicking_power = ctx.player.skills.goalkeeping.kicking / 20.0;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let halfway_x = field_width * 0.5;
        let mid_y = field_height * 0.5;

        let keeper_pos = ctx.player.position;

        // Target the halfway line, slightly off-centre (random to avoid
        // predictability). Clearances aim central-ish so they land where
        // the midfielders can contest them rather than near a sideline.
        let rng = &ctx.context.rng;
        let y_jitter: f32 = rng.random_range(-field_height * 0.15..field_height * 0.15);
        let target_y = mid_y + y_jitter;

        // Direction always upfield — away from own goal, toward opponent half.
        let target_x = match ctx.player.side {
            Some(PlayerSide::Left) => halfway_x, // home kicks toward x = halfway
            Some(PlayerSide::Right) => halfway_x, // away kicks toward x = halfway (same spot)
            None => halfway_x,
        };

        let horizontal_to_target =
            Vector3::new(target_x - keeper_pos.x, target_y - keeper_pos.y, 0.0);
        let horizontal_dist = horizontal_to_target.norm().max(0.1);
        let horizontal_dir = horizontal_to_target / horizontal_dist;

        // Apex of the hoof, in metres. A goalkeeper's kick from hand is
        // the highest ball in football — 20 m at the top of the arc is
        // normal, and a better striker of the ball gets it higher.
        let apex_metres = 16.0 + kicking_power * 8.0; // 16 - 24 m
        let z_velocity = Ball::launch_speed_for_apex(apex_metres);

        // The arc's own hang time is the whole budget: reach the aim
        // point inside it and the ball lands where it was aimed. Clamped
        // to a realistic kick — 2.6 u/tick is 32 m/s — so a keeper stuck
        // very deep in his own half cannot solve his way to a rocket.
        let hang = Ball::hang_ticks(z_velocity).max(1.0);
        let horizontal_speed = (horizontal_dist / hang).clamp(0.30, 2.6);
        let horizontal_velocity = horizontal_dir * horizontal_speed;

        let ball_velocity = Vector3::new(horizontal_velocity.x, horizontal_velocity.y, z_velocity);

        Some(Event::PlayerEvent(PlayerEvent::ClearBall(ball_velocity)))
    }
}
