use crate::r#match::StateProcessingContext;
use crate::r#match::engine::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};
use crate::r#match::player::strategies::players::DefensiveRole;

/// The one rule every defender obeys regardless of which state he is in:
/// **do not let the ball sit behind you.**
///
/// # Why this is a cross-state constraint and not a state
///
/// Line depth used to live in exactly one place — the zonal target in
/// `DefenderHoldingLineState::velocity` — and that state hands off to
/// `Pressing` / `Running` / `Covering` / `Marking` on every
/// `DefensiveRole` branch the moment `opponents().with_ball()` is `Some`.
/// So the only code in the engine that knew about defensive depth was
/// dead in precisely the situation it existed for: opposition
/// possession. The team-shared `defensive_line_x` did not cover for it
/// either — that value is read only as a deviation guard, never as a
/// steering target, and it is computed from game phase alone without
/// ever consulting where the ball is.
///
/// The measured result (`dev_match stats`, `block_diag`): shots struck
/// from 11.4m against a back four sitting at **34.6m** from its own goal,
/// with **0.18** opposition outfielders goal-side of the ball against a
/// real 2-4, and 0.0% of 187k shot-ticks ever finding a defender in the
/// lane. `blocks` read 0.01 per defender per match against a real ~0.9,
/// and every shot in the game was effectively a one-on-one with the
/// keeper — which is also why keeper quality was so far and away the
/// most load-bearing attribute in a squad.
///
/// # The rule
///
/// If the ball is goal-side of me and I am not the man engaging it, my
/// movement must carry me back toward my own goal. The `Primary` role is
/// exempt: somebody has to go to the ball, and that is his job. Everyone
/// else recovers. Lateral movement is untouched, so marking, covering
/// and shuttling all keep working — this only overrides the depth
/// component, which no defender state was managing at all.
///
/// Naturally inert whenever the ball is upfield of the defender, which
/// is every possession spent in the opposition half.
pub struct DefensiveRecovery;

impl DefensiveRecovery {
    /// Base recovery speed, before the pace and recovery multipliers.
    /// See the note at the call site for why this is the line-holding
    /// base rather than the player's nominal max speed.
    const RECOVERY_SPEED: f32 = 3.0;

    /// How far goal-side of the ball a defender is trying to get, in
    /// game units (24u = 3m).
    ///
    /// The rule needs a margin, not just "level with the ball". Without
    /// one it switches off the instant a defender reaches the ball's own
    /// depth, his state's velocity resumes and pulls him back upfield,
    /// and the equilibrium settles ABOVE the ball — measured 19.8m
    /// against a ball at 11.7m, with only 0.27 defenders actually
    /// goal-side. Recovering to a cover point instead puts the
    /// equilibrium where defending happens: between the ball and the
    /// goal, close enough to get a body in the way.
    /// Tuned, not guessed: 14u was measured against 24u and cost
    /// defensive quality for nothing — goal-side presence fell 1.04 →
    /// 0.72 per shot and blocks 0.07 → 0.05, while goals/match sat at
    /// 2.22 either way. The population goals level is set by the rule
    /// EXISTING (defenders being goal-side at all), not by how far
    /// goal-side they get, so there is no trade to make here and the
    /// margin should be the one that defends best.
    const COVER_MARGIN: f32 = 24.0;

    /// The depth (x) velocity this defender must be running at, or
    /// `None` when the rule is inert and his state keeps its own.
    ///
    /// Returns only the depth component on purpose: lateral movement is
    /// the state's business — marking, covering and shuttling all keep
    /// working — and depth is the one axis no defender state was
    /// managing at all.
    pub fn depth_override(ctx: &StateProcessingContext) -> Option<f32> {
        // The man on the ball is not defending a line.
        if ctx.player.has_ball(ctx) {
            return None;
        }
        let own_goal_x = ctx.ball().direction_to_own_goal().x;
        let me_x = ctx.player.position.x;
        // Unit direction from me toward my own goal, so the test below
        // reads the same on both sides of the pitch.
        let to_goal = (own_goal_x - me_x).signum();
        // Am I already at the cover point or goal-side of it?
        let ball_x = ctx.tick_context.positions.ball.position.x;
        let cover_x = ball_x + to_goal * Self::COVER_MARGIN;
        if (cover_x - me_x) * to_goal <= 0.0 {
            return None;
        }
        // Somebody has to go to the ball.
        if matches!(
            ctx.player().defensive().defensive_role_for_ball_carrier(),
            DefensiveRole::Primary
        ) {
            return None;
        }
        // Deliberately the same base speed `DefenderHoldingLineState`
        // steers at, NOT `max_speed_with_condition_cached`. The states
        // do not agree on a speed scale — holding the line steers at
        // 3.0, covering at `pace * 0.9` (~12), and both exceed the
        // player's nominal max speed, which only the `SteeringBehavior`
        // paths clamp against. Recovering at the nominal max therefore
        // makes a defender uniquely slower than every attacker he is
        // chasing: measured, it dropped goal-side presence from 1.06 to
        // 0.90 per shot. Matching the line-holding base keeps him on the
        // same scale as the players around him. `recovery_run_mult` and
        // the pace term still make a quick defender genuinely quicker.
        let profile = DefenderSkillProfile::from_ctx(ctx);
        let pace = (ctx.player.skills.physical.pace / 20.0).clamp(0.6, 1.2);
        Some(to_goal * Self::RECOVERY_SPEED * pace * profile.recovery_run_mult)
    }
}

/// Defender-specific activity intensity configuration
pub struct DefenderConfig;

impl ActivityIntensityConfig for DefenderConfig {
    fn very_high_fatigue() -> f32 {
        8.0 // Explosive actions tire quickly
    }

    fn high_fatigue() -> f32 {
        5.0 // Base from running state
    }

    fn moderate_fatigue() -> f32 {
        3.0
    }

    fn low_fatigue() -> f32 {
        1.0
    }

    fn recovery_rate() -> f32 {
        -3.0
    }

    fn sprint_multiplier() -> f32 {
        1.5 // Sprinting
    }

    fn jogging_multiplier() -> f32 {
        0.6
    }

    fn walking_multiplier() -> f32 {
        0.3
    }

    fn low_condition_threshold() -> i16 {
        LOW_CONDITION_THRESHOLD
    }

    fn jadedness_interval() -> u64 {
        FIELD_PLAYER_JADEDNESS_INTERVAL
    }

    fn jadedness_increment() -> i16 {
        JADEDNESS_INCREMENT
    }
}

/// Defender condition processor (type alias for clarity)
pub type DefenderCondition = ConditionProcessor<DefenderConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
