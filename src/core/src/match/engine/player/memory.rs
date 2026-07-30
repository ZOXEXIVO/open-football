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

    /// Can this player take a shot right now?
    ///
    /// After shooting, a player is physically unable to strike again
    /// instantly — momentum carries them forward, the ball has left
    /// their feet, stance is broken. Real football: rebound shots
    /// (saved → tap-in by the same striker) take ~1-2 seconds. The
    /// per-possession shot cap and team-level cooldown handle "shot
    /// spam" prevention; this cooldown only needs to be long enough
    /// to make a back-to-back strike physically unrealistic, not to
    /// cap match-long shot totals (the team cooldown does that).
    ///
    /// 200 ticks (2 sim seconds) — balanced for rebound goals to
    /// remain possible from a parry/loose-ball scramble. The previous
    /// 800-tick lockout blocked all rebound mechanics, removing one
    /// of football's key goal patterns.
    pub fn can_shoot(&self, current_tick: u64) -> bool {
        const PLAYER_SHOT_COOLDOWN_TICKS: u64 = 200;
        if self.shots_taken == 0 {
            return true;
        }
        current_tick.saturating_sub(self.last_shot_tick) >= PLAYER_SHOT_COOLDOWN_TICKS
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
