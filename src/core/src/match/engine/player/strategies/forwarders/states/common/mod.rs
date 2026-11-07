use crate::r#match::StateProcessingContext;
use crate::r#match::common_states::{EngagementClock, TackleEngagement};
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};
use crate::r#match::player::strategies::players::skills::SkillCurve;

/// The range at which a forward commits to chasing a loose ball down,
/// as one pair of numbers instead of two states guessing separately.
///
/// # Why this exists
///
/// `ForwardReturningState` entered `Intercepting` whenever the ball was
/// within **200u**; `ForwardInterceptingState` gave the chase up and
/// returned whenever the ball was beyond **150u**. Every ball sitting in
/// the 150-200u band therefore satisfied both conditions at once, and the
/// forward alternated between the two states on consecutive AI ticks for
/// as long as it stayed there — measured at 35,270 `Intercepting` exits
/// in a single match, **100% of them after one tick or less**
/// (`dev_match trace`), and ~18,000 round trips each way.
///
/// Overlapping windows like this cannot settle: the fix is that the
/// give-up distance must lie strictly OUTSIDE the commit distance, with
/// the gap between them acting as the commitment. A forward who has set
/// off after a ball keeps going a little past the point where he would
/// have started — which is what chasing a ball actually looks like.
pub struct InterceptionRange;

/// When a forward's press is over. `Pressing` gives up on this and the doors
/// into it ask it first, so he is not sent to a press he would abandon.
pub struct ForwardPress;

impl ForwardPress {
    /// The ball is too far away to be worth chasing.
    pub const MAX_BALL_DISTANCE: f32 = 120.0;
    /// He has gone further from his place in the shape than a press is worth.
    const MAX_DISTANCE_FROM_POSITION: f32 = 200.0;

    pub fn called_off(ctx: &StateProcessingContext) -> bool {
        !TackleEngagement::is_nominated_presser(ctx)
            && (ctx.ball().distance() > Self::MAX_BALL_DISTANCE
                || ctx.player().distance_from_position() > Self::MAX_DISTANCE_FROM_POSITION
                || ctx.ball().on_own_side())
    }
}

/// The forward's run in behind: when he may start one and when he has to
/// stop. Both ends read the same picture, and starting is the stricter of the
/// two, so a run cannot be refused on the tick it begins.
pub struct RunInBehind;

impl RunInBehind {
    /// Seconds over which a run with one man in the way runs out. The
    /// release against a single blocker is what sends a runner back through
    /// `Running`, where his receiving and shot logic lives; a run that never
    /// released parked him behind the line and cut forward shots about 4x.
    const ONE_BLOCKER_RELEASE_WINDOW: f32 = 0.25;
    /// Seconds over which he picks his moment to start past one man.
    const ONE_BLOCKER_COMMIT_WINDOW: f32 = 2.0;

    /// Opponents within reach of his line to goal.
    fn blockers(ctx: &StateProcessingContext) -> usize {
        let player_pos = ctx.player.position;
        let to_goal = (ctx.player().opponent_goal_position() - player_pos).normalize();
        ctx.players()
            .opponents()
            .nearby(30.0)
            .filter(|opp| (opp.position - player_pos).normalize().dot(&to_goal) > 0.3)
            .count()
    }

    /// His chance of beating one man. Sigmoid at 12/20 so the whole 1-20
    /// range of pace matters.
    fn beats_one_man(ctx: &StateProcessingContext) -> f32 {
        SkillCurve::new(ctx.player.skills.physical.pace, 12.0, 0.6).probability()
    }

    /// Is he ahead of whoever has the ball?
    fn ahead_of_the_passer(ctx: &StateProcessingContext) -> bool {
        if let Some(owner_id) = ctx.ball().owner_id()
            && let Some(owner) = ctx.context.players.by_id(owner_id)
            && owner.team_id == ctx.player.team_id
        {
            let passer_pos = ctx.tick_context.positions.players.position(owner_id);
            let to_goal = (ctx.player().opponent_goal_position() - passer_pos).normalize();
            let to_runner = (ctx.player.position - passer_pos).normalize();
            return to_runner.dot(&to_goal) > 0.0;
        }
        true
    }

    /// Can the man on the ball deliver one at all?
    fn passer_can_deliver(ctx: &StateProcessingContext) -> bool {
        if let Some(owner_id) = ctx.ball().owner_id()
            && let Some(owner) = ctx.context.players.by_id(owner_id)
            && owner.team_id == ctx.player.team_id
        {
            let vision = owner.skills.mental.vision / 20.0;
            let passing = owner.skills.technical.passing / 20.0;
            return (vision + passing) / 2.0 > 0.3;
        }
        true
    }

    fn picture_is_on(ctx: &StateProcessingContext) -> bool {
        Self::ahead_of_the_passer(ctx) && !ctx.player().is_tired() && Self::passer_can_deliver(ctx)
    }

    /// Worth starting.
    pub fn may_commit(ctx: &StateProcessingContext) -> bool {
        if !Self::picture_is_on(ctx) {
            return false;
        }
        match Self::blockers(ctx) {
            0 => true,
            1 => ctx.context.rng.bernoulli(EngagementClock::per_tick(
                Self::beats_one_man(ctx),
                Self::ONE_BLOCKER_COMMIT_WINDOW,
            )),
            _ => false,
        }
    }

    /// Worth carrying on with.
    pub fn stays_viable(ctx: &StateProcessingContext) -> bool {
        if !Self::picture_is_on(ctx) {
            return false;
        }
        match Self::blockers(ctx) {
            0 => true,
            1 => !ctx.context.rng.bernoulli(EngagementClock::per_tick(
                1.0 - Self::beats_one_man(ctx),
                Self::ONE_BLOCKER_RELEASE_WINDOW,
            )),
            _ => false,
        }
    }
}

impl InterceptionRange {
    /// Ball must be at least this close before a forward sets off after
    /// it. Tightened from the old 200u entry so the commit range and the
    /// give-up range can be ordered without widening the chase.
    pub const COMMIT: f32 = 150.0;
    /// Chase is abandoned once the ball is further away than this.
    /// Strictly greater than `COMMIT` — that ordering is the whole point.
    pub const GIVE_UP: f32 = 180.0;
}

/// Forward-specific activity intensity configuration
pub struct ForwardConfig;

impl ActivityIntensityConfig for ForwardConfig {
    fn very_high_fatigue() -> f32 {
        8.5 // Forwards: highest fatigue for explosive actions
    }

    fn high_fatigue() -> f32 {
        5.5 // Slightly higher than midfielders
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
        1.6 // Forwards sprint more often than midfielders
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

/// Forward condition processor (type alias for clarity)
pub type ForwardCondition = ConditionProcessor<ForwardConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
