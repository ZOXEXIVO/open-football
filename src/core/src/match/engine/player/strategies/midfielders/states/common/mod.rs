pub mod carry;

use crate::r#match::StateProcessingContext;
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};
use crate::r#match::midfielders::states::MidfielderState;

pub use carry::{LaneAhead, TakeOn, U_PER_M};

/// Where a midfielder's shape position is, and whether he is far enough
/// off it to be worth a recovery run.
pub struct ShapeStation;

impl ShapeStation {
    /// Recovery run is complete inside this radius. Same value
    /// `MidfielderReturningState` has always used to hand back to
    /// `Running`, so "arrived" means exactly what it did before.
    pub const HOME: f32 = 80.0;
    /// A midfielder already home is only sent back out once he has
    /// drifted past this. The gap to `HOME` is the commitment that stops
    /// the two states contradicting each other.
    pub const DISPLACED: f32 = 110.0;

    /// Distance from the player's shape position.
    pub fn drift(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position - ctx.player.start_position).magnitude()
    }

    /// Is there a recovery run left to make?
    ///
    /// Hysteretic on the current state: a player already in `Returning`
    /// keeps going until he is inside `HOME`, while one who is not has to
    /// have drifted past `DISPLACED` before a fresh recovery is worth
    /// starting. Both sides of the hand-off read this, so neither can
    /// contradict the other.
    pub fn should_recover(ctx: &StateProcessingContext) -> bool {
        let returning = matches!(
            ctx.player.state,
            crate::r#match::player::state::PlayerState::Midfielder(MidfielderState::Returning)
        );
        let band = if returning {
            Self::HOME
        } else {
            Self::DISPLACED
        };
        Self::drift(ctx) > band
    }
}

/// Is there a ball here that could actually be intercepted?
///
/// `MidfielderInterceptingState` rejects three situations on sight — he
/// has the ball, somebody is carrying it, or our own side is in control
/// of it — and eleven separate entry points each drew their own,
/// different condition for going in. So the state was routinely entered
/// for a ball it would refuse on the very next line, and it spent its
/// life as a one-tick pass-through: measured at **15,969 exits across
/// three matches with a mean dwell of 0.7 AI ticks, 93.8% of visits
/// lasting a single tick**.
///
/// Patching the guards one at a time does not hold — the first pass
/// added `!is_owned` to seven of them and moved the number by 2%,
/// because the dominant case was our OWN pass in flight (no owner, but
/// `is_control_ball`, which is a reception rather than an interception).
/// One predicate that mirrors the state's own contract is the only
/// version that cannot drift out of step with it.
pub struct Interception;

impl Interception {
    pub fn is_available(ctx: &StateProcessingContext) -> bool {
        !ctx.player.has_ball(ctx) && !ctx.ball().is_owned() && !ctx.team().is_control_ball()
    }
}

/// A decision taken once, for as long as this passage of play lasts.
///
/// Several midfielder decisions used to be a fresh `rng` roll on every
/// tick — a 0.3% chance of striking the clear chance, a 10% chance of
/// the pressed snapshot. Two things are wrong with that and both show
/// on the pitch. Holding the ball becomes a way of buying more chances
/// for the answer to come out "yes", so the way to get a shot is to
/// dawdle rather than to improve the position; and the answer flickers
/// while the situation has not changed, which is the opposite of a
/// player making up his mind. It is also the pattern
/// `feedback_perception_not_statistics` rules out.
///
/// `ownership_duration` counts ticks since this possession started, so
/// the tick it started on names the passage of play. Hashed with the
/// player and a per-call-site salt it gives a number that is fixed for
/// as long as he has the ball, independent between decisions, and
/// different the next time he gets it.
pub struct Opportunity;

impl Opportunity {
    /// A value in `0..1`, constant for this player and this possession.
    /// `salt` separates unrelated decisions so a player who declines one
    /// is not thereby declining all of them.
    pub fn draw(ctx: &StateProcessingContext, salt: u64) -> f32 {
        let possession_start = ctx
            .current_tick()
            .saturating_sub(ctx.tick_context.ball.ownership_duration as u64);
        let mixed = possession_start.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (ctx.player.id as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
            ^ salt.wrapping_mul(0x94D0_49BB_1331_11EB);
        // Avalanche the low bits before taking a slice of the top.
        let mixed = mixed ^ (mixed >> 31);
        let mixed = mixed.wrapping_mul(0x7FB5_D329_728E_A185);
        ((mixed >> 40) as f32) / ((1u32 << 24) as f32)
    }

    /// Same draw, plus the id of a specific opponent — for decisions
    /// that are about a particular man, so that when the next one steps
    /// across it is a new question rather than the same refusal.
    pub fn draw_vs(ctx: &StateProcessingContext, salt: u64, opponent_id: u32) -> f32 {
        Self::draw(
            ctx,
            salt ^ (opponent_id as u64).wrapping_mul(0xE703_7ED1_A0B4_28DB),
        )
    }
}

/// Census of what a midfielder actually DOES with the ball at his feet.
///
/// `MidfielderRunningState::process` is the on-ball decision tree, and
/// every branch of it ends in one of the exits below. Counting them per
/// tick is the only way to see which branch is eating the possession —
/// the aggregate stats can only show the outcome (a pass), never which
/// of the eleven pass-emitting branches produced it.
///
/// Compiled out entirely without `match-logs`; see [`record`].
#[cfg(feature = "match-logs")]
pub mod onball_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// One slot per exit of the on-ball tree, in evaluation order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(usize)]
    pub enum Exit {
        Corner = 0,
        SnapshotShot = 1,
        EmergencyClear = 2,
        ShootClearChance = 3,
        ShootHelper = 4,
        ShootLayoff = 5,
        TempoHold = 6,
        PatientRecycle = 7,
        PatientHold = 8,
        CongestionPass = 9,
        Carry = 10,
        Dribble = 11,
        CounterPass = 12,
        OneTwo = 13,
        DrawRelease = 14,
        Cutback = 15,
        Cross = 16,
        Switch = 17,
        TempoBackPass = 18,
        ShouldPass = 19,
        AntiOscillation = 20,
        NoDecision = 21,
        ForcedTakeOn = 22,
    }

    pub const EXITS: usize = 23;
    pub const NAMES: [&str; EXITS] = [
        "corner",
        "snapshot-shot",
        "emergency-clear",
        "shoot:clear-chance",
        "shoot:helper",
        "shoot:layoff-pass",
        "hold:coach-tempo",
        "pass:patient-recycle",
        "hold:patient",
        "pass:congestion",
        "CARRY",
        "DRIBBLE",
        "pass:counter",
        "pass:one-two",
        "pass:draw-release",
        "pass:cutback",
        "cross",
        "pass:switch",
        "pass:tempo-back",
        "pass:should_pass",
        "anti-oscillation",
        "no-decision (hold)",
        "DRIBBLE (forced)",
    ];

    const ZERO: AtomicU64 = AtomicU64::new(0);
    pub static EXIT_TICKS: [AtomicU64; EXITS] = [ZERO; EXITS];

    /// Distribution of "opponents in the take-on cone" seen at the
    /// dribble gate, bucketed 0 / 1 / 2 / 3+. The gate refuses at 0 and
    /// (without the skill) at 1, so this says whether the refusal is a
    /// geometry problem or a skill problem.
    pub static AHEAD_BUCKETS: [AtomicU64; 4] = [ZERO; 4];
    /// Ticks the dribble gate saw an opponent to beat but the carrier's
    /// skill profile refused the take-on.
    pub static DRIBBLE_SKILL_REFUSED: AtomicU64 = AtomicU64::new(0);
    /// Ticks the take-on was geometrically available AND allowed.
    pub static DRIBBLE_ALLOWED: AtomicU64 = AtomicU64::new(0);

    pub fn record(exit: Exit) {
        EXIT_TICKS[exit as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ahead(n: usize, allowed: bool) {
        AHEAD_BUCKETS[n.min(3)].fetch_add(1, Ordering::Relaxed);
        if n > 0 {
            if allowed {
                DRIBBLE_ALLOWED.fetch_add(1, Ordering::Relaxed);
            } else {
                DRIBBLE_SKILL_REFUSED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn reset() {
        for c in EXIT_TICKS.iter().chain(AHEAD_BUCKETS.iter()) {
            c.store(0, Ordering::Relaxed);
        }
        DRIBBLE_SKILL_REFUSED.store(0, Ordering::Relaxed);
        DRIBBLE_ALLOWED.store(0, Ordering::Relaxed);
    }

    /// `(per-exit tick counts, ahead-bucket counts, skill-refused, allowed)`.
    pub fn snapshot() -> ([u64; EXITS], [u64; 4], u64, u64) {
        let mut exits = [0u64; EXITS];
        for (i, slot) in EXIT_TICKS.iter().enumerate() {
            exits[i] = slot.load(Ordering::Relaxed);
        }
        let mut ahead = [0u64; 4];
        for (i, slot) in AHEAD_BUCKETS.iter().enumerate() {
            ahead[i] = slot.load(Ordering::Relaxed);
        }
        (
            exits,
            ahead,
            DRIBBLE_SKILL_REFUSED.load(Ordering::Relaxed),
            DRIBBLE_ALLOWED.load(Ordering::Relaxed),
        )
    }
}

/// No-op shim so the call sites stay unconditional.
#[cfg(not(feature = "match-logs"))]
#[allow(dead_code)]
pub mod onball_diag {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Exit {
        Corner,
        SnapshotShot,
        EmergencyClear,
        ShootClearChance,
        ShootHelper,
        ShootLayoff,
        TempoHold,
        PatientRecycle,
        PatientHold,
        CongestionPass,
        Carry,
        Dribble,
        CounterPass,
        OneTwo,
        DrawRelease,
        Cutback,
        Cross,
        Switch,
        TempoBackPass,
        ShouldPass,
        AntiOscillation,
        NoDecision,
        ForcedTakeOn,
    }

    #[inline(always)]
    pub fn record(_exit: Exit) {}
    #[inline(always)]
    pub fn record_ahead(_n: usize, _allowed: bool) {}
}

/// Midfielder-specific activity intensity configuration
pub struct MidfielderConfig;

impl ActivityIntensityConfig for MidfielderConfig {
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

/// Midfielder condition processor (type alias for clarity)
pub type MidfielderCondition = ConditionProcessor<MidfielderConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
