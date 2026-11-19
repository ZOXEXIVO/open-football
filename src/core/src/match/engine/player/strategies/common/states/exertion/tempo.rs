//! **How fast a man holding the shape needs to move.** He is priced in
//! time: the pace that has him at his place before the play can come at the
//! space he guards. With time to spare he walks; as it shrinks he jogs, then
//! runs.
//!
//! A defender is priced only while the play waits on his side — his own
//! side on the ball, or the ball dead — and moves at his state's own tier
//! otherwise. A midfielder is priced in every phase: against the
//! opposition's ball the time is how long the play takes to reach his
//! place, and with the ball in behind him he runs at his full effort; with
//! his own side on it he also keeps pace with his place as the shape moves.
//! A forward is priced as a midfielder is, without the run back: nothing
//! behind him is his to recover.

use crate::r#match::player::strategies::RestartHold;
use crate::r#match::{ActivityIntensity, ConditionContext, MovementEffort, StateProcessingContext};
use nalgebra::Vector3;

#[derive(Clone, Copy, Debug)]
pub struct ShapeTempo {
    /// The pace he moves at, u/tick.
    pace: f32,
    /// That pace as a share of his conditioned top speed.
    effort: f32,
}

impl ShapeTempo {
    /// A walk, 1.5 m/s.
    const WALK: f32 = 0.12;
    /// How fast the play can reach a space it is going for: by a pass, 15
    /// m/s.
    const PLAY_PACE: f32 = 1.2;
    /// The time his own side's ball gives him before it can be won and
    /// played into his space, 3 s.
    const TURNOVER_TICKS: f32 = 300.0;
    /// The time a dead ball gives him, 10 s, which the side the clock is
    /// against shortens by up to 70%.
    const RESTART_TICKS: f32 = 1_000.0;
    const RESTART_HURRY: f32 = 0.7;
    /// Over the last stretch he eases into his place: no faster than covers
    /// what is left in this many ticks.
    const ARRIVE_TICKS: f32 = 20.0;

    /// A/B control: with `OF_DEFENDER_TEMPO=off` every defender state keeps
    /// the fixed tier it declared before. Read once per process — debug
    /// infrastructure, do not remove.
    pub fn defenders_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_DEFENDER_TEMPO").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_MIDFIELDER_TEMPO=off` a midfielder's shape and
    /// recovery states keep their fixed tiers and the recall runs flat out.
    /// Read once per process — debug infrastructure, do not remove.
    pub fn midfielders_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_MIDFIELDER_TEMPO").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_FORWARD_TEMPO=off` a forward's shape and
    /// recovery states keep their fixed tiers and the recall runs flat out.
    /// Read once per process — debug infrastructure, do not remove.
    pub fn forwards_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_FORWARD_TEMPO").is_ok_and(|v| v == "off"))
    }

    /// Does the play wait on a defender's side: his side on the ball, or
    /// the ball dead?
    pub fn applies(ctx: &StateProcessingContext) -> bool {
        !Self::defenders_off()
            && (RestartHold::taker(ctx.tick_context).is_some() || ctx.team().is_control_ball())
    }

    /// The pace to `target` while the play waits on his side.
    pub fn to(ctx: &StateProcessingContext, target: Vector3<f32>) -> Self {
        Self::covering(ctx, target, Self::time_to_play(ctx, target), 0.0)
    }

    /// A midfielder's pace to his place, `target`, in every phase.
    pub fn midfielder(ctx: &StateProcessingContext, target: Vector3<f32>) -> Self {
        let dead = RestartHold::taker(ctx.tick_context).is_some();
        if dead || ctx.team().is_control_ball() {
            let keep_up = if dead { 0.0 } else { ctx.team().shape().pace };
            return Self::covering(ctx, target, Self::time_to_play(ctx, target), keep_up);
        }
        if Self::ball_in_behind(ctx) {
            return ShapeTempo {
                pace: ctx.player.max_speed_with_condition_cached(),
                effort: 1.0,
            };
        }
        let ball = ctx.tick_context.positions.ball.position;
        let time = (ball - target).xy().norm() / Self::PLAY_PACE;
        Self::covering(ctx, target, time, 0.0)
    }

    /// A forward's pace to his place, `target`: in time for the play to
    /// reach it against the opposition's ball; with his own side on it the
    /// shape's own pace plus what closing the gap needs; with the ball dead,
    /// in time for the restart. Nothing behind him asks a forward to sprint
    /// back: his place is the front of the block.
    pub fn forward(ctx: &StateProcessingContext, target: Vector3<f32>) -> Self {
        let dead = RestartHold::taker(ctx.tick_context).is_some();
        if dead || ctx.team().is_control_ball() {
            let keep_up = if dead { 0.0 } else { ctx.team().shape().pace };
            return Self::covering(ctx, target, Self::time_to_play(ctx, target), keep_up);
        }
        let ball = ctx.tick_context.positions.ball.position;
        let time = (ball - target).xy().norm() / Self::PLAY_PACE;
        Self::covering(ctx, target, time, 0.0)
    }

    /// The pace the shape's recall runs a man back to `target` at, where
    /// his role's tempo governs it: a midfielder's or a forward's own, in
    /// every phase.
    pub fn recall_pace(ctx: &StateProcessingContext, target: Vector3<f32>) -> Option<f32> {
        let position = &ctx.player.tactical_position.current_position;
        if position.is_midfielder() && !Self::midfielders_off() {
            Some(Self::midfielder(ctx, target).pace)
        } else if position.is_forward() && !Self::forwards_off() {
            Some(Self::forward(ctx, target).pace)
        } else {
            None
        }
    }

    /// His own steering from `position`, at this pace: the direction is the
    /// state's, the speed the tempo's, easing in over the last stretch to
    /// `target`.
    pub fn steer(
        &self,
        position: Vector3<f32>,
        target: Vector3<f32>,
        velocity: Vector3<f32>,
    ) -> Vector3<f32> {
        let Some(direction) = velocity.try_normalize(1.0e-6) else {
            return velocity;
        };
        let left = (target - position).xy().norm();
        direction * self.pace.min(left / Self::ARRIVE_TICKS)
    }

    pub fn pace(&self) -> f32 {
        self.pace
    }

    /// The share of his top speed the effort cap has to allow him.
    pub fn effort(&self) -> f32 {
        self.effort
    }

    /// The tier a defender's shape state declares for his fatigue: while
    /// the play waits on his side, the one that contains the pace he is
    /// moving at; otherwise the state's own `fixed` one.
    pub fn declared(ctx: &ConditionContext, fixed: ActivityIntensity) -> ActivityIntensity {
        if Self::defenders_off() || !(ctx.side_on_ball || ctx.ball_dead) {
            return fixed;
        }
        Self::containing(ctx)
    }

    /// The tier a midfielder's shape or recovery state declares: the one
    /// that contains the pace he is moving at, in every phase.
    pub fn midfielder_declared(
        ctx: &ConditionContext,
        fixed: ActivityIntensity,
    ) -> ActivityIntensity {
        if Self::midfielders_off() {
            return fixed;
        }
        Self::containing(ctx)
    }

    /// The tier a forward's shape or recovery state declares: the one that
    /// contains the pace he is moving at, in every phase.
    pub fn forward_declared(ctx: &ConditionContext, fixed: ActivityIntensity) -> ActivityIntensity {
        if Self::forwards_off() {
            return fixed;
        }
        Self::containing(ctx)
    }

    fn containing(ctx: &ConditionContext) -> ActivityIntensity {
        let player = &ctx.player;
        let top = player.max_speed_with_condition_cached().max(1.0e-3);
        MovementEffort::tier_containing(
            player.velocity.norm() / top,
            player.player_attributes.condition_percentage(),
        )
    }

    /// The pace that covers the distance to `target` in `time` ticks, on
    /// top of `keep_up`, never below a walk.
    fn covering(
        ctx: &StateProcessingContext,
        target: Vector3<f32>,
        time: f32,
        keep_up: f32,
    ) -> Self {
        let full = ctx.player.max_speed_with_condition_cached().max(1.0e-3);
        let distance = (target - ctx.player.position).xy().norm();
        let pace = (keep_up + distance / time.max(1.0)).clamp(Self::WALK.min(full), full);
        ShapeTempo {
            pace,
            effort: (pace / full).clamp(0.0, 1.0),
        }
    }

    /// The opposition has the ball goal-side of him.
    fn ball_in_behind(ctx: &StateProcessingContext) -> bool {
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return false;
        };
        let forward = ctx.player.side.map_or(1.0, |side| side.forward_dir_x());
        (ctx.player.position.x - carrier.position.x) * forward > 0.0
    }

    /// Ticks until the play can come at `target`: the restart's time with
    /// the ball dead, otherwise the ball's distance at the play's pace plus
    /// the turnover his own side's ball gives him.
    fn time_to_play(ctx: &StateProcessingContext, target: Vector3<f32>) -> f32 {
        if RestartHold::taker(ctx.tick_context).is_some() {
            let hurry = ctx
                .tick_context
                .ball
                .restarting_team
                .map_or(0.0, |team| ctx.context.restart_hurry(team));
            return Self::RESTART_TICKS * (1.0 - Self::RESTART_HURRY * hurry);
        }
        let ball = ctx.tick_context.positions.ball.position;
        (ball - target).xy().norm() / Self::PLAY_PACE + Self::TURNOVER_TICKS
    }
}
