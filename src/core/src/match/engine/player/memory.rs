/// Per-player, per-match scratch memory.
///
/// Deliberately small and entirely live: every field here is written by
/// the event pipeline and read by a decision path. An earlier version
/// also carried an intention queue (`IntentionKind` / `TimedIntention`),
/// a rolling `recent_events` log and a `confidence` scalar — none of
/// which had a single caller anywhere in the engine. They cost a pair of
/// `Vec` allocations per player per match plus a decay pass every 100
/// ticks and influenced nothing, so they were removed rather than left
/// as a trap for the next reader.
///
/// The one live consumer of the old event log was
/// `MemoryEventType::PassCompleted`, which incremented a `pass_streak`
/// that the forward passing state turned into a scoring bonus. Since
/// `record_event` was only ever called with `ShotTaken`, that branch
/// never ran and the bonus was always exactly zero — the term is gone
/// with it.
///
/// In-match psychology lives in [`PsychologyState`] on the match context
/// (confidence / nervousness, fed by real events and read by the pass
/// evaluator, the ownership duel and the state machine) — that is the
/// single source of truth for how a player is feeling.
///
/// [`PsychologyState`]: crate::r#match::engine::psychology::PsychologyState
#[derive(Debug, Clone, Default)]
pub struct PlayerMemory {
    pub last_shot_tick: u64,
    pub shots_taken: u32,
    pub shots_on_target: u32,
    /// Tick of the last shot-vs-pass decision the player made, regardless
    /// of whether a shot fired. Used by the shot-decision cadence so a
    /// forward in shooting range doesn't roll a fresh willingness die
    /// every tick — real strikers commit to one decision per ~half-second
    /// of carrying the ball, not 100 decisions per second.
    pub last_shot_decision_tick: u64,

    pub last_xg: f32,
    pub last_xg_tick: u64,
    /// Sum of expected-goals across every shot the player took this match.
    pub xg_total: f32,
}

impl PlayerMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Can this player take a shot right now? **Always yes.**
    ///
    /// 2026-08-16: the per-player shot cooldown is REMOVED. It was 200
    /// ticks (2 s), justified as "momentum carries them forward, the ball
    /// has left their feet, stance is broken" — but that is a description
    /// of a player who has just kicked the ball, and such a player does
    /// not have the ball to shoot again anyway. **There is no cooldown in
    /// football.** A striker whose shot is parved back to him hits it
    /// again immediately; that is one of the game's commonest goals, and a
    /// timer cannot tell it apart from spam.
    ///
    /// What actually stops a player shooting twice in a row is that he
    /// must regain the ball first, which the ownership model already
    /// enforces. Anything beyond that was a quota standing in for
    /// defending, and defending is where it belongs — see `SHOT_BAR_BASE`
    /// for the rest of the teardown.
    ///
    /// Kept as a method rather than deleted at the call sites so the
    /// pressing work has an obvious place to reintroduce a real
    /// constraint (fatigue, balance, body shape) if one is wanted.
    pub fn can_shoot(&self, _current_tick: u64) -> bool {
        true
    }

    /// Credit a shot at the moment it is struck. `shots_on_target` is NOT
    /// credited here — it's credited lazily by `credit_shot_on_target`
    /// when the ball actually reaches the goal frame (keeper save or
    /// goal). Before that split, any shot aimed between the posts counted
    /// as on-target even when a defender blocked it or it sailed over the
    /// bar, leaving ~49% of "on-target" shots with no corresponding
    /// save-or-goal outcome.
    pub fn record_shot(&mut self, tick: u64) {
        self.last_shot_tick = tick;
        self.shots_taken += 1;
    }

    /// Post-hoc on-target credit. Called when a shot actually reaches
    /// the goal frame (keeper save or goal). Kept separate from
    /// `record_shot` because the outcome isn't known at launch — the
    /// ball has to travel, and a defender can still block it in flight.
    pub fn credit_shot_on_target(&mut self) {
        self.shots_on_target += 1;
    }

    pub fn record_shot_xg(&mut self, tick: u64, xg: f32) {
        self.last_xg = xg;
        self.last_xg_tick = tick;
        self.xg_total += xg;
    }
}
