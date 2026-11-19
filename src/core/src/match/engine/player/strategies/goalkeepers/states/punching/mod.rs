use crate::r#match::engine::ball::ball::AerialReach;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperRead, ReadLevel,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    Ball, ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// How long the contact and the landing take.
///
/// UNITS: AI ticks, and the AI runs on every SECOND engine tick, so one is
/// **20 ms** — 22 is ~0.44 s, quicker than a dive because he is punching
/// from his feet or from a leap rather than lying on the floor with the
/// ball. A punch used to resolve and leave inside a single tick, which is
/// invisible and let him re-enter the state on the next one. It is a
/// committed action (`PlayerState::is_committed_action`) so nothing else
/// can pull him out of it either.
const MIN_PUNCH_TICKS: u64 = 22;

#[derive(Default, Clone)]
pub struct GoalkeeperPunchingState {}

impl GoalkeeperPunchingState {
    /// The lowest ball a fist plays. A ball at the chest is caught and one
    /// at the shins is kicked or dived on; the punch is the contact for a
    /// ball at his head or above, where the hands are behind it.
    pub const FLOOR: f32 = AerialReach::HEAD;
    /// The lowest a punch goes up: 4 m, over the heads it was punched away
    /// from.
    const MIN_APEX: f32 = 4.0;
    /// Off the goal's own axis toward the nearer touchline, before the
    /// spray: 45°.
    const WIDE: f32 = std::f32::consts::FRAC_PI_4;
    /// A punch is a deflection under pressure, not an aimed pass: ±20°.
    const SPRAY: f32 = 0.35;

    /// Out from his goal, turned toward the touchline nearer the ball. A
    /// ball dead in the middle goes to the side the spray falls on.
    fn punch_direction(ctx: &StateProcessingContext) -> Vector3<f32> {
        let out = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let spray = ctx.context.rng.random_range(-Self::SPRAY..Self::SPRAY);
        let off_centre =
            ctx.tick_context.positions.ball.position.y - ctx.ball().direction_to_own_goal().y;
        let wide = if off_centre.abs() > f32::EPSILON {
            off_centre.signum()
        } else {
            spray.signum()
        };
        let angle = Self::WIDE + spray;
        Vector3::new(out * angle.cos(), wide * angle.sin(), 0.0)
    }

    /// **The punch**, thrown on the aerial contest's word that his fist is
    /// on the ball (`ClaimOutcome::Punched`); the state he goes into is the
    /// follow-through. Out and wide, so a punch from the middle of his goal
    /// does not drop on the penalty spot for the next man in; flatter and
    /// shorter than a kicked clearance, up 4–6 m and 15–25 m long, solved
    /// from an apex like every other kick in the engine.
    pub fn strike(ctx: &StateProcessingContext) -> StateChangeResult {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::KeeperActionDiag::note(6);
        let direction = Self::punch_direction(ctx);
        let rise = Ball::launch_speed_for_apex(Self::MIN_APEX + prof.aerial_command * 2.0);
        let range = 120.0 + prof.aerial_command * 80.0;
        let power = (range / Ball::hang_ticks(rise).max(1.0)).clamp(0.30, 2.2);
        let mut result = StateChangeResult::with_goalkeeper_state(GoalkeeperState::Punching);
        result.events.add_player_event(PlayerEvent::ClearBall(
            ctx.player.id,
            Vector3::new(direction.x * power, direction.y * power, rise),
        ));
        result
    }
}

impl StateProcessingHandler for GoalkeeperPunchingState {
    /// The punch was thrown on the way in ([`Self::strike`]); this is the
    /// follow-through. He is landing, not winding up again, and the read
    /// has him once he is down.
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.in_state_time < MIN_PUNCH_TICKS {
            return None;
        }
        KeeperRead::from(ctx, ReadLevel::Possession).go()
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while punching
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Punching is a very high intensity activity requiring explosive effort
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
