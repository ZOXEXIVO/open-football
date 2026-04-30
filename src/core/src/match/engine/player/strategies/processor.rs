use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::CommonInjuredState;
use crate::r#match::defenders::states::{DefenderState, DefenderStrategies};
use crate::r#match::events::{Event, EventCollection};
use crate::r#match::forwarders::states::{ForwardState, ForwardStrategies};
use crate::r#match::goalkeepers::states::state::{GoalkeeperState, GoalkeeperStrategies};
use crate::r#match::midfielders::states::{MidfielderState, MidfielderStrategies};
use crate::r#match::player::memory::PlayerMemory;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::state::PlayerState::{Defender, Forward, Goalkeeper, Midfielder};
use crate::r#match::player::strategies::common::PlayerOperationsImpl;
use crate::r#match::player::strategies::common::PlayersOperationsImpl;
use crate::r#match::team::TeamOperationsImpl;
use crate::r#match::{BallOperationsImpl, GameTickContext, MatchContext, MatchPlayer};
use log::debug;
use nalgebra::Vector3;

pub trait StateProcessingHandler {
    /// Decide whether the state should transition or emit an event this tick.
    fn process(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }
    /// Per-tick velocity contribution. Default: no movement from this state.
    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        None
    }
    /// Side-effects after the state resolves. Default: no-op.
    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl PlayerFieldPositionGroup {
    pub fn process(
        &self,
        in_state_time: u64,
        player: &mut MatchPlayer,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> StateProcessingResult {
        // Universal loose-ball override. Applied once at dispatch time so
        // every state benefits without needing its own copy of the guard.
        // Without this, the "designated chaser" selected by distance could
        // be in a state (Shooting, Finishing, Pressing, Dribbling, …) that
        // had no idea to abandon its current job and claim the ball — and
        // the ball would sit untouched while everyone assumed someone else
        // was going for it.
        //
        // The symmetric case also matters: a player already IN TakeBall
        // who's no longer the closest (ball rolled past them, teammate
        // got closer) should yield back to Running. Without the yield,
        // chasers pile up over time because TakeBall only exits on
        // ownership, not on "someone else is a better chaser now".
        let override_state_time = if Self::should_yield_takeball(*self, player, tick_context) {
            player.state = Self::yield_state_for(*self);
            0
        } else if Self::should_force_takeball(*self, player, tick_context) {
            player.state = Self::takeball_state_for(*self);
            0
        } else {
            in_state_time
        };
        let _ = context; // all needed state lives in player + tick_context

        let player_state = player.state;
        let state_processor =
            StateProcessor::new(override_state_time, player, context, tick_context);

        match player_state {
            // Common states
            PlayerState::Injured => state_processor.process(CommonInjuredState::default()),
            // // Specific states
            Goalkeeper(state) => GoalkeeperStrategies::process(state, state_processor),
            Defender(state) => DefenderStrategies::process(state, state_processor),
            Midfielder(state) => MidfielderStrategies::process(state, state_processor),
            Forward(state) => ForwardStrategies::process(state, state_processor),
        }
    }

    /// TakeBall variant for this position group. Outfield players commit
    /// to claiming a loose ball the same way; goalkeepers get their own
    /// TakeBall which handles the "only if near my box" rules internally.
    #[inline]
    fn takeball_state_for(group: PlayerFieldPositionGroup) -> PlayerState {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::TakeBall),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::TakeBall)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::TakeBall),
        }
    }

    /// Default state to drop into when yielding TakeBall back to the pack.
    /// Outfield players go to Running — their off-ball velocity reshapes
    /// the defensive block with the new chaser designated. GK returns to
    /// Attentive — back to reading the game.
    #[inline]
    fn yield_state_for(group: PlayerFieldPositionGroup) -> PlayerState {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::Standing)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::Running),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::Running)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::Running),
        }
    }

    /// True when this player is in TakeBall but another teammate is
    /// strictly-closer to the ball. Releases the chase so the pack doesn't
    /// accumulate ex-chasers who overshot or got passed by the ball.
    fn should_yield_takeball(
        _group: PlayerFieldPositionGroup,
        player: &MatchPlayer,
        tick_context: &GameTickContext,
    ) -> bool {
        if !matches!(
            player.state,
            PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
                | PlayerState::Defender(DefenderState::TakeBall)
                | PlayerState::Midfielder(MidfielderState::TakeBall)
                | PlayerState::Forward(ForwardState::TakeBall)
        ) {
            return false;
        }
        // If the ball has been claimed, TakeBall's own `process` will
        // handle the transition to Running. Don't front-run it.
        if tick_context.ball.is_owned {
            return false;
        }
        let Some(my_side) = player.side else {
            return false;
        };
        // Use landing_position here to match `should_force_takeball`.
        // If yield used the current aerial position and force used
        // landing, a designated chaser could get yielded mid-flight
        // because a teammate happens to be closer to the ball's apex
        // — and nobody converges on the bounce.
        let ball_pos = tick_context.positions.ball.landing_position;
        let my_dist_sq = (ball_pos - player.position).norm_squared();
        // Hysteresis: only yield if a teammate is MEANINGFULLY closer
        // (by at least HYSTERESIS units). Otherwise tick-to-tick jitter
        // in movement swaps the "closest" designation between teammates
        // every tick, turning the chase into a ping-pong where each
        // player keeps yielding to the other and nobody commits long
        // enough to cover the final few units into the claim radius.
        const HYSTERESIS: f32 = 8.0;
        let yield_threshold_sq = {
            let my_dist = my_dist_sq.sqrt();
            let threshold = (my_dist - HYSTERESIS).max(0.0);
            threshold * threshold
        };
        for tm in tick_context.positions.players.as_slice() {
            if tm.player_id == player.id || tm.side != my_side {
                continue;
            }
            let d_sq = (ball_pos - tm.position).norm_squared();
            if d_sq < yield_threshold_sq {
                return true;
            }
        }
        false
    }

    /// True when this player should ignore their current-state logic and
    /// sprint to claim a loose ball. Fires when:
    ///   - The ball is not owned (free, not in-flight-with-intent),
    ///   - The ball is within meaningful chase range (saves compute on
    ///     balls that have rolled into the far corner — someone closer
    ///     will handle them),
    ///   - This player is the strictly-closest teammate by raw distance
    ///     (no ability weighting — we want exactly one claimant, not the
    ///     tolerance band of `is_best_player_to_chase_ball`),
    ///   - Not already in TakeBall (don't re-trigger and reset timers).
    fn should_force_takeball(
        group: PlayerFieldPositionGroup,
        player: &MatchPlayer,
        tick_context: &GameTickContext,
    ) -> bool {
        // Already chasing — leave the state alone.
        if matches!(
            player.state,
            PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
                | PlayerState::Defender(DefenderState::TakeBall)
                | PlayerState::Midfielder(MidfielderState::TakeBall)
                | PlayerState::Forward(ForwardState::TakeBall)
        ) {
            return false;
        }

        // Ball must actually be loose.
        if tick_context.ball.is_owned {
            return false;
        }

        // See `should_yield_takeball` for why landing position is
        // preferred: lofted clearances need their chaser to converge on
        // the bounce, not the apex. `landing_position == position` for
        // ground balls, so this doesn't change ground-ball behaviour.
        let ball_pos = tick_context.positions.ball.landing_position;

        // Goalkeepers only claim balls near their box — the outfield
        // claimants handle anything further. Prevents the GK sprinting
        // 80m for a loose ball when a defender is 2m from it. GK will
        // transition to TakeBall via their own Standing/Walking guard
        // when the ball actually threatens their area.
        if group == PlayerFieldPositionGroup::Goalkeeper {
            let gk_dist_sq = (ball_pos - player.position).norm_squared();
            if gk_dist_sq > 60.0 * 60.0 {
                return false;
            }
        }

        let my_dist_sq = (ball_pos - player.position).norm_squared();

        // Am I the strictly-closest teammate? Tie-break by player id so
        // two players at exactly equal distance don't both trigger.
        //
        // CRITICAL: use `tick_context.positions.players` (live, updated
        // every tick) rather than `context.players` (a static snapshot
        // taken at match start, frozen thereafter). With the snapshot,
        // every player compared their *current* position against every
        // teammate's *match-start* position — all of them thought they
        // were closest, all of them flipped to TakeBall at once.
        //
        // Team membership is derived from `side` because the live store
        // doesn't carry team_id. Sent-off players are stashed at
        // (-500, -500), so they naturally fail any distance comparison
        // — no explicit filter needed.
        let my_side = match player.side {
            Some(s) => s,
            None => return false,
        };
        for tm in tick_context.positions.players.as_slice() {
            if tm.player_id == player.id || tm.side != my_side {
                continue;
            }
            let d_sq = (ball_pos - tm.position).norm_squared();
            if d_sq < my_dist_sq {
                return false;
            }
            if d_sq == my_dist_sq && tm.player_id < player.id {
                return false;
            }
        }

        true
    }
}

pub struct StateProcessor<'p> {
    in_state_time: u64,
    player: &'p mut MatchPlayer,
    context: &'p MatchContext,
    tick_context: &'p GameTickContext,
}

impl<'p> StateProcessor<'p> {
    pub fn new(
        in_state_time: u64,
        player: &'p mut MatchPlayer,
        context: &'p MatchContext,
        tick_context: &'p GameTickContext,
    ) -> Self {
        StateProcessor {
            in_state_time,
            player,
            context,
            tick_context,
        }
    }

    pub fn process<H: StateProcessingHandler>(self, handler: H) -> StateProcessingResult {
        // Match progress drives the late-game fatigue curve. Uses the
        // match half-time constant so debug / release builds both give
        // the correct 0..1 progression over their configured match length.
        let half_ms = crate::r#match::engine::engine::MATCH_HALF_TIME_MS as f32;
        let full_ms = half_ms * 2.0;
        let match_progress = (self.context.total_match_time as f32 / full_ms).clamp(0.0, 1.0);
        let condition_ctx = ConditionContext {
            in_state_time: self.in_state_time,
            player: self.player,
            match_progress,
        };

        // Process player conditions
        handler.process_conditions(condition_ctx);

        self.process_inner(handler)
    }

    pub fn process_inner<H: StateProcessingHandler>(self, handler: H) -> StateProcessingResult {
        let player_id = self.player.id;
        let need_extended_state_logging = self.player.use_extended_state_logging;

        let processing_ctx = self.into_ctx();
        let mut result = StateProcessingResult::new();

        if let Some(velocity) = handler.velocity(&processing_ctx) {
            // Apply coach tempo multiplier to all player movement
            let tempo = processing_ctx.team().coach_instruction().tempo_multiplier();
            result.velocity = Some(velocity * tempo);
        }

        // common logic
        let complete_result = |state_results: StateChangeResult,
                               mut result: StateProcessingResult| {
            // Propagate the tackle-cooldown signal regardless of whether
            // the handler also changed state — a successful tackle
            // returns a state-change + cooldown, but a keep-current-state
            // (None) wouldn't hit the `if let Some(state)` branch below.
            result.start_tackle_cooldown = state_results.start_tackle_cooldown;
            // Propagate the shot reason the same way — tagged at the
            // transition point, consumed by the Shooting state when it
            // composes the Shoot event.
            result.shot_reason = state_results.shot_reason;
            if let Some(state) = state_results.state {
                if need_extended_state_logging {
                    debug!("Player, Id={}, State {:?}", player_id, state);
                }
                result.state = Some(state);
                result.events = state_results.events;
            }
            result
        };

        if let Some(state_result) = handler.process(&processing_ctx) {
            return complete_result(state_result, result);
        }

        result
    }

    pub fn into_ctx(self) -> StateProcessingContext<'p> {
        StateProcessingContext::from(self)
    }
}

pub struct ConditionContext<'sp> {
    pub in_state_time: u64,
    pub player: &'sp mut MatchPlayer,
    /// Match progress 0.0..1.0 (0 = kickoff, 1.0 = 90'). Feeds the
    /// second-half fatigue-curve: recovery slows and sprint cost rises
    /// as the match progresses, so late-game players genuinely fade.
    pub match_progress: f32,
}

pub struct StateProcessingContext<'sp> {
    pub in_state_time: u64,
    pub player: &'sp MatchPlayer,
    pub context: &'sp MatchContext,
    pub tick_context: &'sp GameTickContext,
}

impl<'sp> StateProcessingContext<'sp> {
    #[inline]
    pub fn ball(&'sp self) -> BallOperationsImpl<'sp> {
        BallOperationsImpl::new(self)
    }

    #[inline]
    pub fn player(&'sp self) -> PlayerOperationsImpl<'sp> {
        PlayerOperationsImpl::new(self)
    }

    #[inline]
    pub fn players(&'sp self) -> PlayersOperationsImpl<'sp> {
        PlayersOperationsImpl::new(self)
    }

    #[inline]
    pub fn team(&'sp self) -> TeamOperationsImpl<'sp> {
        TeamOperationsImpl::new(self)
    }

    #[inline]
    pub fn memory(&self) -> &PlayerMemory {
        &self.player.memory
    }

    #[inline]
    pub fn current_tick(&self) -> u64 {
        self.context.current_tick()
    }
}

impl<'sp> From<StateProcessor<'sp>> for StateProcessingContext<'sp> {
    fn from(value: StateProcessor<'sp>) -> Self {
        StateProcessingContext {
            in_state_time: value.in_state_time,
            player: value.player,
            context: value.context,
            tick_context: value.tick_context,
        }
    }
}

pub struct StateProcessingResult {
    pub state: Option<PlayerState>,
    pub velocity: Option<Vector3<f32>>,
    pub events: EventCollection,
    /// Propagated up from the per-state `StateChangeResult`. Consumed by
    /// `state.rs` to bump `player.tackle_cooldown`.
    pub start_tackle_cooldown: bool,
    /// Tagged reason to attach to the next Shoot event fired by this
    /// player. Matches the pass-reason pattern. Written to
    /// `player.pending_shot_reason` by `state.rs` so the Shooting state
    /// can read it when composing the event.
    pub shot_reason: Option<&'static str>,
}

impl Default for StateProcessingResult {
    fn default() -> Self {
        Self::new()
    }
}

impl StateProcessingResult {
    pub fn new() -> Self {
        StateProcessingResult {
            state: None,
            velocity: None,
            events: EventCollection::new(),
            start_tackle_cooldown: false,
            shot_reason: None,
        }
    }
}

pub struct StateChangeResult {
    pub state: Option<PlayerState>,
    pub velocity: Option<Vector3<f32>>,

    pub events: EventCollection,

    /// Defender signalled "I just attempted a tackle" — the state.rs
    /// update loop consumes this and bumps `player.tackle_cooldown` so
    /// the next ~100 ticks of Tackling-state entries short-circuit
    /// without rolling an attempt. Must live on the result (not be
    /// applied directly in the state) because `ctx.player` is an
    /// immutable borrow inside the state processor.
    pub start_tackle_cooldown: bool,
    /// Tag the NEXT Shoot event fired by this player with this reason.
    /// Set by transitions to the Shooting state so the resulting
    /// Shoot event carries the decision-path context. Mirrors how
    /// pass events carry `with_reason(...)` — see Shooting state
    /// for the consumer.
    pub shot_reason: Option<&'static str>,
}

impl Default for StateChangeResult {
    fn default() -> Self {
        Self::new()
    }
}

impl StateChangeResult {
    pub fn new() -> Self {
        StateChangeResult {
            state: None,
            velocity: None,
            events: EventCollection::new(),
            start_tackle_cooldown: false,
            shot_reason: None,
        }
    }

    /// Tag the next Shoot event fired by this player with `reason`.
    /// Fluent helper to keep transition sites readable —
    /// `StateChangeResult::with_forward_state(Shooting).with_shot_reason("FWD_PRIO_06")`.
    pub fn with_shot_reason(mut self, reason: &'static str) -> Self {
        self.shot_reason = Some(reason);
        self
    }

    pub fn with(state: PlayerState) -> Self {
        StateChangeResult {
            state: Some(state),
            ..Self::new()
        }
    }

    pub fn with_goalkeeper_state(state: GoalkeeperState) -> Self {
        StateChangeResult {
            state: Some(Goalkeeper(state)),
            ..Self::new()
        }
    }

    pub fn with_goalkeeper_state_and_event(state: GoalkeeperState, event: Event) -> Self {
        StateChangeResult {
            state: Some(Goalkeeper(state)),
            events: EventCollection::with_event(event),
            ..Self::new()
        }
    }

    pub fn with_defender_state(state: DefenderState) -> Self {
        StateChangeResult {
            state: Some(Defender(state)),
            ..Self::new()
        }
    }

    pub fn with_defender_state_and_event(state: DefenderState, event: Event) -> Self {
        StateChangeResult {
            state: Some(Defender(state)),
            events: EventCollection::with_event(event),
            ..Self::new()
        }
    }

    pub fn with_midfielder_state(state: MidfielderState) -> Self {
        StateChangeResult {
            state: Some(Midfielder(state)),
            ..Self::new()
        }
    }

    pub fn with_midfielder_state_and_event(state: MidfielderState, event: Event) -> Self {
        StateChangeResult {
            state: Some(Midfielder(state)),
            events: EventCollection::with_event(event),
            ..Self::new()
        }
    }

    pub fn with_forward_state(state: ForwardState) -> Self {
        StateChangeResult {
            state: Some(Forward(state)),
            ..Self::new()
        }
    }

    pub fn with_forward_state_and_event(state: ForwardState, event: Event) -> Self {
        StateChangeResult {
            state: Some(Forward(state)),
            events: EventCollection::with_event(event),
            ..Self::new()
        }
    }

    pub fn with_event(event: Event) -> Self {
        StateChangeResult {
            events: EventCollection::with_event(event),
            ..Self::new()
        }
    }

    pub fn with_events(events: EventCollection) -> Self {
        StateChangeResult {
            events,
            ..Self::new()
        }
    }
}
