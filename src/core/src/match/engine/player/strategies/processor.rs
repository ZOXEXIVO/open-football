use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::CommonInjuredState;
use crate::r#match::defenders::states::{DefenderState, DefenderStrategies};
use crate::r#match::events::{Event, EventCollection};
use crate::r#match::forwarders::states::{ForwardState, ForwardStrategies};
use crate::r#match::goalkeepers::states::common::KeeperSweepLimit;
use crate::r#match::goalkeepers::states::state::{GoalkeeperState, GoalkeeperStrategies};
use crate::r#match::midfielders::states::{MidfielderState, MidfielderStrategies};
use crate::r#match::player::memory::PlayerMemory;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::state::PlayerState::{Defender, Forward, Goalkeeper, Midfielder};
use crate::r#match::player::strategies::common::PlayerOperationsImpl;
use crate::r#match::player::strategies::common::PlayersOperationsImpl;
use crate::r#match::player::strategies::common::states::{
    CornerHold, KeeperReleaseSpace, RestartCarry,
};
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::player_context::LooseBallChase;
use crate::r#match::team::{ShapeDiscipline, TeamOperationsImpl};
use crate::r#match::{BallOperationsImpl, GameTickContext, MatchContext, MatchPlayer, PlayerSide};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::ShapeCensus;
use log::debug;
use nalgebra::Vector3;

/// Whether the loose-ball election stands down while the ball is out of
/// play, and who is exempt from it.
///
/// A dead ball is not a loose ball: only the awarded taker may touch it,
/// and he is not racing anybody for it. Both halves of the election
/// disagreed — [`should_yield_takeball`](PlayerFieldPositionGroup::
/// should_yield_takeball) threw the taker out of `TakeBall` the moment a
/// teammate stood nearer the spot (for a goal kick, always), and
/// `should_force_takeball` sent the other twenty-one at a ball they are
/// not allowed to have.
///
/// `OF_RESTART_HOLD=off` restores the old behaviour so the two can be
/// measured against each other: the change lands during ~5% of the match
/// (157 awaited restarts × 1.8 s), so it moves more than the restart
/// itself and the aggregate has to be checked rather than assumed.
pub struct RestartHold;

impl RestartHold {
    /// False when `OF_RESTART_HOLD=off` — the election runs on dead balls
    /// exactly as it used to.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_RESTART_HOLD")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }

    /// The taker a pending restart belongs to, or `None` when the ball is
    /// in play (or the hold is switched off).
    #[inline]
    pub fn taker(tick_context: &GameTickContext) -> Option<u32> {
        if !Self::armed() {
            return None;
        }
        tick_context.ball.restart_taker
    }
}

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

#[cfg(feature = "match-logs")]
pub mod chase_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Times the dispatcher forced a player into TakeBall, and times it
    /// yielded him back out. If the two are close and both large, the
    /// chase DESIGNATION is flip-flopping rather than the ball changing
    /// hands — the hysteresis is not holding.
    pub static FORCE: AtomicU64 = AtomicU64::new(0);
    pub static YIELD: AtomicU64 = AtomicU64::new(0);
    /// Of the forces, how many happened while a delivery was in flight.
    pub static FORCE_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in [&FORCE, &YIELD, &FORCE_IN_FLIGHT] {
            c.store(0, Ordering::Relaxed);
        }
    }

    pub fn snapshot() -> (u64, u64, u64) {
        (
            FORCE.load(Ordering::Relaxed),
            YIELD.load(Ordering::Relaxed),
            FORCE_IN_FLIGHT.load(Ordering::Relaxed),
        )
    }
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
        // `redirect_to_fresh` zeroes the PERSISTED counter as well as the
        // dispatch value. The two used to disagree — dispatch saw 0 while
        // `player.in_state_time` kept climbing — so the destination state
        // read 0 on its entry tick and a stale value on the next one.
        // That is what made a goalkeeper redirected into `TakeBall` trip
        // its own `in_state_time > 200` give-up guard on tick two and
        // flap straight back to `Standing`.
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            if Self::should_yield_takeball(*self, player, tick_context) {
                chase_diag::YIELD.fetch_add(1, Ordering::Relaxed);
            } else if Self::should_force_takeball(*self, player, context, tick_context) {
                chase_diag::FORCE.fetch_add(1, Ordering::Relaxed);
                if tick_context.ball.is_in_flight_state > 0 {
                    chase_diag::FORCE_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        let override_state_time = if Self::should_yield_takeball(*self, player, tick_context) {
            player.redirect_to_fresh(
                Self::yield_state_for(*self),
                TransitionSource::LooseBallOverride,
            );
            0
        } else if Self::should_force_takeball(*self, player, context, tick_context) {
            player.redirect_to_fresh(
                Self::takeball_state_for(*self),
                TransitionSource::LooseBallOverride,
            );
            0
        } else {
            in_state_time
        };

        let player_state = player.state;
        let state_processor =
            StateProcessor::new(override_state_time, player, context, tick_context);

        let mut result = match player_state {
            // Common states
            PlayerState::Injured => state_processor.process(CommonInjuredState::default()),
            // // Specific states
            Goalkeeper(state) => GoalkeeperStrategies::process(state, state_processor),
            Defender(state) => DefenderStrategies::process(state, state_processor),
            Midfielder(state) => MidfielderStrategies::process(state, state_processor),
            Forward(state) => ForwardStrategies::process(state, state_processor),
        };
        // Universal corner-shape hold, applied at dispatch for the same
        // reason as the loose-ball override above: a corner puts twenty
        // players somewhere their own state did not choose, and not one of
        // the four state machines knows to stay there. See `CornerHold`.
        CornerHold::apply(player, tick_context, &mut result);
        // …and the man carrying a dead ball back to the spot it is taken
        // from, for every restart rather than only the corner. LAST, and
        // deliberately an outright override: the ball rides on his
        // position while he carries it, so anything that moves him moves
        // it. See `RestartCarry`.
        RestartCarry::apply(player, tick_context, &mut result);
        result
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

    /// Which side a player is on, from the live position store. Returns
    /// `None` for an id that isn't on the pitch, which compares unequal
    /// to any real side — the safe answer for the receiving override.
    #[inline]
    fn side_of(player_id: u32, tick_context: &GameTickContext) -> Option<PlayerSide> {
        tick_context
            .positions
            .players
            .as_slice()
            .iter()
            .find(|e| e.player_id == player_id)
            .map(|e| e.side)
    }

    /// True when this player is in TakeBall but another teammate is
    /// strictly-closer to the ball. Releases the chase so the pack doesn't
    /// accumulate ex-chasers who overshot or got passed by the ball.
    /// True when this player is mid-way through an action a real
    /// footballer cannot abort. Committed players are skipped by BOTH
    /// loose-ball redirects and are absent from the chase table, so the
    /// designation naturally falls to the next-closest teammate.
    #[inline]
    fn is_committed(player: &MatchPlayer) -> bool {
        player.state.is_committed_action()
    }

    // `pub(crate)` so `goal_kick_tests` can put the question directly. The
    // election is a pure function of the frozen tick snapshot, and driving
    // it through a whole engine tick would test the dispatcher instead.
    pub(crate) fn should_yield_takeball(
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
        // **The taker of a dead ball is not in a race.** See
        // `BallMetadata::restart_taker`: the election below asks whether a
        // teammate is nearer, and for a goal kick one nearly always is, so
        // the keeper was thrown out of `TakeBall` on the tick after every
        // nudge and never covered a metre.
        if RestartHold::taker(tick_context) == Some(player.id) {
            return false;
        }
        // Mirror of the receiving rule in `should_force_takeball`: the
        // intended receiver holds the chase however close a teammate
        // gets, and his teammates drop it. The two must agree or a
        // player oscillates between being forced in and yielded out.
        if tick_context.ball.is_in_flight_state > 0 {
            if let Some(target_id) = tick_context.ball.pass_target {
                if target_id == player.id {
                    return false;
                }
                if Self::side_of(target_id, tick_context) == player.side {
                    return true;
                }
            }
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
        //
        // ⚠ AND IT HAS TO BE WIDER WHILE THE BALL IS IN THE AIR, because
        // then the target itself is moving: `landing_position` slides as
        // the ball travels, so "closest man to where it will land" is a
        // different player from tick to tick and a 1 m margin on a moving
        // point buys nothing.
        //
        // Measured: **4,987 forces and 17,505 yields a match, 98% of the
        // forces during a delivery in flight** — three and a half yields
        // per force, and `Midfielder: Running <-> Take Ball` the second
        // largest loop in the engine (~20,500 round trips per three
        // matches).
        //
        // ⚠ …AND SUPPRESSING IT COSTS GOALS. Both ways of doing so were
        // measured over three runs each, and both lose in proportion to
        // how much churn they remove:
        //
        //   | variant                    | yields/match | goals |
        //   |----------------------------|--------------|-------|
        //   | as written (8u)            | 17,505       | 4.4   |
        //   | 40u margin while in flight | ~15,100      | 4.95  |
        //   | hold the chase all flight  | 9,952        | 5.25  |
        //
        // The re-election IS the defending. Because `landing_position`
        // moves, asking every tick is how the man who is ACTUALLY closest
        // to where the ball ends up gets there; freezing the designation
        // at the first tick of a delivery commits the wrong man and the
        // pass completes. The churn is a symptom of a moving target, not
        // a bug in the hand-off, and `Midfielder: Running <-> Take Ball`
        // stays near the top of the loop table because of it.
        //
        // Same lesson as the `Running <-> Marking` loop in
        // `defenders/states/marking`: in this engine a two-state cycle is
        // often load-bearing. Measure the match, not the loop count.
        const HYSTERESIS: f32 = 8.0;
        let yield_threshold_sq = {
            let my_dist = my_dist_sq.sqrt();
            let threshold = (my_dist - HYSTERESIS).max(0.0);
            threshold * threshold
        };
        // "Any same-side entry (excluding me) closer than the threshold"
        // ⇔ the side's min distance excluding me beats the threshold —
        // read from the once-per-tick chase table instead of re-scanning
        // the roster for every player.
        let result = match tick_context.chase.best_other(my_side, player.id) {
            Some(best) => best.dist_sq < yield_threshold_sq,
            None => false,
        };
        debug_assert_eq!(
            result,
            Self::yield_takeball_scan(player, tick_context, ball_pos, yield_threshold_sq, my_side),
            "loose-ball yield chase-table mismatch"
        );
        result
    }

    /// Reference implementation of the yield scan — the pre-table
    /// per-player roster walk. Kept as the debug oracle: every table
    /// answer is recomputed and compared in debug/test builds.
    #[allow(dead_code)]
    fn yield_takeball_scan(
        player: &MatchPlayer,
        tick_context: &GameTickContext,
        ball_pos: Vector3<f32>,
        yield_threshold_sq: f32,
        my_side: PlayerSide,
    ) -> bool {
        for tm in tick_context.positions.players.as_slice() {
            if tm.player_id == player.id || tm.side != my_side || !tm.chase_eligible {
                continue;
            }
            if LooseBallChase::chase_dist_sq(tm, ball_pos) < yield_threshold_sq {
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
    // `pub(crate)` for the same reason as `should_yield_takeball`.
    pub(crate) fn should_force_takeball(
        group: PlayerFieldPositionGroup,
        player: &MatchPlayer,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> bool {
        // Mid-action — a keeper already committed to a dive, a header
        // already launched, a defender already sliding. Real footballers
        // cannot abandon these, and the observed transition graph carried
        // exactly those edges (`Goalkeeper: Diving -> Take Ball`,
        // `Forward: Heading -> Take Ball`) before this guard. The player
        // is also absent from the chase table while committed, so the
        // claim passes to the next-closest teammate rather than being
        // dropped.
        if Self::is_committed(player) {
            return false;
        }

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

        // A ball OUT OF PLAY is loose in the sense this test means and in
        // no other: nobody but its taker may touch it, and everyone else
        // converging on it is twenty-one players running at a ball they
        // are not allowed to have. See `BallMetadata::restart_taker`.
        if let Some(taker) = RestartHold::taker(tick_context) {
            return taker == player.id;
        }

        // A pass in the air belongs to its target — see the deadlock
        // described on `BallMetadata::pass_target`. He is his side's
        // chaser and his teammates stand off; the defending side keeps
        // its normal designation so it can contest the ball the moment
        // it comes free.
        if tick_context.ball.is_in_flight_state > 0 {
            if let Some(target_id) = tick_context.ball.pass_target {
                if target_id == player.id {
                    return true;
                }
                if Self::side_of(target_id, tick_context) == player.side {
                    return false;
                }
            }
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
            // **A SHOT AT HIS OWN GOAL IS NOT A LOOSE BALL TO CHASE.**
            //
            // A struck shot is unowned and carries no `pass_target`, so
            // every guard above waves it through, and it lands inside the
            // 60 u radius below by definition — it is aimed at his goal. So
            // the override pulled the keeper out of `PreparingForSave` and
            // into `TakeBall` on the tick a shot came within 7.5 m, and
            // `TakeBall` does not set him: he ran AT the ball like an
            // outfielder going for a loose one, at the `Active` band, with
            // none of `KeeperShotReaction::on_foot`'s set-keeper cap on him.
            // (`Catching` and `Diving` were safe by being committed actions;
            // the set stance deliberately is not, and that was the hole.)
            //
            // ⚠ **This chase was carrying part of the population save
            // rate.** Removing it costs 0.25 goals/match on its own, because
            // the ground it covered in the last 7.5 m of a flight is ground
            // the geometry never credited him with. That is re-derived where
            // it belongs — in `SaveModel::FULL_STRETCH_TICKS`, see the note
            // there — and NOT bought back by letting him sprint at shots.
            //
            // What he does about a shot belongs to the save states, all of
            // it: `KeeperShotReaction` for the set and the read,
            // `KeeperShotDive` for the dive, `KeeperShotSave` for the roll.
            if tick_context
                .ball
                .cached_shot_target
                .as_ref()
                .is_some_and(|t| Some(t.defending_side) == player.side)
            {
                return false;
            }
            let gk_dist_sq = (ball_pos - player.position).norm_squared();
            if gk_dist_sq > 60.0 * 60.0 {
                return false;
            }
            // **…AND ONLY FOR A BALL INSIDE THE GROUND A KEEPER DEFENDS.**
            //
            // The test above is purely relative to HIM: 60 u of anywhere on
            // the pitch. So a keeper who had correctly pushed up to sweep
            // was forced onto a loose ball seven metres away and thirty
            // metres from his own goal out by the touchline — and
            // `GoalkeeperTakeBallState` is a bare `Seek` at the ball with a
            // four-second timeout and no territory of its own, so from there
            // he simply followed it. Measured over 200 matches, states other
            // than the sweep and the recovery accounted for 17% of every
            // tick he spent beyond 25 m from his goal.
            //
            // The bound here is the NARROWEST territory any keeper has, so
            // that the state he is forced into — which applies his own,
            // wider one — never hands him straight back. See
            // [`KeeperSweepLimit::innermost`].
            let goal = match player.side {
                Some(PlayerSide::Right) => context.goal_positions.right,
                Some(PlayerSide::Left) | None => context.goal_positions.left,
            };
            if KeeperSweepLimit::strain(goal, ball_pos, KeeperSweepLimit::innermost()) > 1.0 {
                return false;
            }
        }

        let my_dist_sq = (ball_pos - player.position).norm_squared();

        // Am I the strictly-closest teammate? Tie-break by player id so
        // two players at exactly equal distance don't both trigger.
        //
        // CRITICAL: use the live position store (via the chase table)
        // rather than `context.players` (a static snapshot taken at
        // match start, frozen thereafter). With the snapshot, every
        // player compared their *current* position against every
        // teammate's *match-start* position — all of them thought they
        // were closest, all of them flipped to TakeBall at once.
        //
        // Team membership is derived from `side` because the live store
        // doesn't carry team_id. Sent-off players are stashed at
        // (-500, -500), so they naturally fail any distance comparison
        // — no explicit filter needed.
        //
        // The chase table's lexicographic (dist_sq, id) minimum over the
        // same entries reproduces the old scan exactly: "some other
        // entry is strictly closer, or equally close with a lower id"
        // ⇔ the best-other beats my (my_dist_sq, my id).
        let my_side = match player.side {
            Some(s) => s,
            None => return false,
        };
        let result = tick_context
            .chase
            .is_designated(my_side, player.id, my_dist_sq);
        debug_assert_eq!(
            result,
            Self::force_takeball_scan(player, tick_context, ball_pos, my_dist_sq, my_side),
            "loose-ball force chase-table mismatch"
        );
        result
    }

    /// Reference implementation of the force scan — the pre-table
    /// per-player roster walk, kept as the debug oracle.
    #[allow(dead_code)]
    fn force_takeball_scan(
        player: &MatchPlayer,
        tick_context: &GameTickContext,
        ball_pos: Vector3<f32>,
        my_dist_sq: f32,
        my_side: PlayerSide,
    ) -> bool {
        for tm in tick_context.positions.players.as_slice() {
            if tm.player_id == player.id || tm.side != my_side || !tm.chase_eligible {
                continue;
            }
            // `my_dist_sq` is deliberately the caller's raw distance, as the
            // table query compares it: only the teammate being weighed gets
            // the striker gamble.
            let d_sq = LooseBallChase::chase_dist_sq(tm, ball_pos);
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

        // **The opposing keeper has the ball in his hands: leave his
        // area.** Ahead of everything else, and deliberately ahead of
        // `ShapeDiscipline` — the attacking plan's box slots are inside
        // the area he has to vacate, so shaping this would pull him
        // straight back in. It also has to sit outside the `if let`,
        // because the states that stand still return no velocity at all
        // and a striker idling on the six-yard line is the case being
        // fixed. See [`KeeperReleaseSpace`].
        if let Some((out, effort)) = KeeperReleaseSpace::retreat(&processing_ctx) {
            result.velocity = Some(out);
            result.effort_floor = effort;
        } else if let Some(velocity) = handler.velocity(&processing_ctx) {
            // Positional discipline first: keep the state's own intent
            // inside the space the team plan gave this player. Applied
            // here — the one point every state's movement converges on —
            // rather than inside twenty state machines that cannot see
            // each other. See `ShapeDiscipline`.
            let (shaped, pull) = ShapeDiscipline::apply_with_pull(&processing_ctx, velocity);
            result.shape_recall_pull = pull;
            // Apply coach tempo multiplier to all player movement
            let tempo = processing_ctx.team().coach_instruction().tempo_multiplier();
            result.velocity = Some(shaped * tempo);
        }

        // Shape census — one sample per AI tick per player, at the single
        // point every state passes through. See `ShapeCensus`.
        #[cfg(feature = "match-logs")]
        {
            let moving = result.velocity.is_some_and(|v| v.magnitude() > 0.02);
            let anchor = processing_ctx.team().my_anchor();
            let axis_lag = processing_ctx.player.side.map_or(0.0, |s| {
                s.forward_delta(anchor.x, processing_ctx.player.position.x)
            });
            ShapeCensus::note(
                processing_ctx.player.state.compact_id(),
                (processing_ctx.player.position - anchor).magnitude(),
                axis_lag,
                moving,
            );
            Self::note_keeper_guard(&processing_ctx, moving);
            Self::note_keeper_motion(&processing_ctx, result.velocity);
            Self::note_keeper_excursion(&processing_ctx);
        }

        if let Some(change) = handler.process(&processing_ctx) {
            // Extended per-player state trace — only a real transition is
            // worth a line; event-only results keep the current state.
            if need_extended_state_logging {
                if let Some(state) = change.state {
                    debug!("Player, Id={}, State {:?}", player_id, state);
                }
            }
            // Keeper state churn. A transition out of a state he entered
            // less than 300 ms ago is not a decision he made, it is two
            // gates disagreeing — see [`KeeperMotionDiag`]. A self-return
            // is not a transition at all (states use it to hold, so the
            // event fires and `in_state_time` keeps running) and counting
            // those would drown the signal.
            #[cfg(feature = "match-logs")]
            if let PlayerState::Goalkeeper(from) = processing_ctx.player.state {
                if let Some(PlayerState::Goalkeeper(to)) =
                    change.state.filter(|s| *s != processing_ctx.player.state)
                {
                    let quick = processing_ctx.in_state_time <= 15;
                    crate::mid_run_diag::KeeperMotionDiag::note_transition(quick);
                    // …and WHICH two states, because the scalar above has
                    // never once been enough to act on. See `KeeperPairDiag`.
                    crate::mid_run_diag::KeeperPairDiag::note(from as usize, to as usize, quick);
                } else if change
                    .state
                    .is_some_and(|s| s != processing_ctx.player.state)
                {
                    crate::mid_run_diag::KeeperMotionDiag::note_transition(
                        processing_ctx.in_state_time <= 15,
                    );
                }
            }
            // Fold the handler's result in. `merge_state_change` moves the
            // events across WHETHER OR NOT a transition occurred — an
            // event-only result (state == None) must still reach the
            // EventDispatcher. Dropping those was the cause of the
            // CrossReceiving "ground ball rolls through the receiver" bug.
            result.merge_state_change(change);
        }

        result
    }

    /// One MOTION sample per keeper per AI tick, unconditionally — see
    /// [`KeeperMotionDiag`]. Deliberately not gated on the ball being in
    /// his third the way [`Self::note_keeper_guard`] is: the question this
    /// answers is what he does while it is NOT, which is most of a match.
    #[cfg(feature = "match-logs")]
    fn note_keeper_motion(ctx: &StateProcessingContext, velocity: Option<Vector3<f32>>) {
        use crate::mid_run_diag::KeeperMotionDiag;

        if !matches!(ctx.player.state, PlayerState::Goalkeeper(_)) {
            return;
        }
        let goal = ctx.ball().direction_to_own_goal();
        let ball = ctx.tick_context.positions.ball.position;
        let v = velocity.unwrap_or_else(Vector3::zeros);
        let speed = v.magnitude();
        KeeperMotionDiag::note_tick(
            KeeperMotionDiag::band((ball - goal).magnitude()),
            speed,
            speed <= 0.02,
            (ctx.player.position.x - goal.x).abs(),
        );
        // How sharply he is changing direction. A keeper adjusting his
        // angle turns gently; one being pulled between two targets by a
        // state machine that cannot make its mind up reverses, and that
        // is what reads from the stands as chasing the ball.
        if speed > 0.02 {
            let prev = ctx.tick_context.positions.players.velocity(ctx.player.id);
            if prev.magnitude() > 0.02 {
                let cos = (v.normalize().dot(&prev.normalize())).clamp(-1.0, 1.0);
                KeeperMotionDiag::note_heading(cos.acos());
            }
        }
    }

    /// One EXCURSION sample per keeper per AI tick, unconditionally — see
    /// [`KeeperExcursionDiag`]. Separate from [`Self::note_keeper_motion`]
    /// because that one, like every gate it was written alongside, measures
    /// the excursion on the DEPTH axis only: a keeper at the corner flag
    /// registers there as being on his goal line.
    #[cfg(feature = "match-logs")]
    fn note_keeper_excursion(ctx: &StateProcessingContext) {
        use crate::mid_run_diag::KeeperExcursionDiag as D;

        let PlayerState::Goalkeeper(gk_state) = ctx.player.state else {
            return;
        };
        let goal = ctx.ball().direction_to_own_goal();
        let keeper = ctx.player.position;
        let radial = (keeper - goal).magnitude();
        let lateral = (keeper.y - goal.y).abs();
        let depth = (keeper.x - goal.x).abs();
        // Half the width of a penalty area, in engine units: 40.32 m at
        // 8 units to the metre, halved.
        const HALF_AREA_WIDTH: f32 = 161.28;
        const AREA_DEPTH: f32 = 132.0;

        D::note(0);
        D::add(1, (radial * 100.0) as u64);
        D::peak(2, (radial * 100.0) as u64);
        D::add(3, (lateral * 100.0) as u64);
        D::peak(4, (lateral * 100.0) as u64);

        let band = if radial < 48.0 {
            0
        } else if radial < 88.0 {
            1
        } else if radial < 132.0 {
            2
        } else if radial < 200.0 {
            3
        } else if radial < 256.0 {
            4
        } else {
            5
        };
        D::note(5 + band);

        let wide = lateral > HALF_AREA_WIDTH;
        if wide {
            D::note(11);
        }
        if wide || depth > AREA_DEPTH {
            D::note(12);
        }

        if radial > 200.0 {
            if wide {
                D::note(13);
            }
            // …and was the ball even in play? A keeper fetching a ball that
            // has run out for his own goal kick walks wherever it went,
            // including to the corner flag, and he is entitled to: it is a
            // dead ball and nobody may touch it but him. Counted separately
            // so the residual in this census can be read for what it is
            // rather than as the behaviour the census was added to catch.
            if ctx.tick_context.ball.restart_taker.is_some() {
                D::note(26);
                if wide {
                    D::note(27);
                }
            }
            D::note(match gk_state {
                GoalkeeperState::ComingOut => 14,
                GoalkeeperState::Standing => 15,
                GoalkeeperState::Walking => 16,
                GoalkeeperState::ReturningToGoal => 17,
                GoalkeeperState::PreparingForSave => 18,
                _ => 19,
            });
            if !ctx.ball().is_owned() {
                D::note(23);
            } else if ctx.players().opponents().with_ball().next().is_some() {
                D::note(24);
            }
            let ball = ctx.tick_context.positions.ball.position;
            if (ball.y - goal.y).abs() > HALF_AREA_WIDTH {
                D::note(25);
            }
        }

        if matches!(gk_state, GoalkeeperState::ComingOut) {
            D::add(20, (radial * 100.0) as u64);
            D::note(21);
            D::peak(22, (radial * 100.0) as u64);
        }
    }

    /// One position sample per keeper per AI tick, on ticks where the ball
    /// is live in his defensive third. See [`KeeperGuardDiag`] for what the
    /// numbers mean and why an event counter cannot answer the question.
    #[cfg(feature = "match-logs")]
    fn note_keeper_guard(ctx: &StateProcessingContext, moving: bool) {
        use crate::mid_run_diag::KeeperGuardDiag;

        let PlayerState::Goalkeeper(gk_state) = ctx.player.state else {
            return;
        };
        let goal = ctx.ball().direction_to_own_goal();
        let ball = ctx.tick_context.positions.ball.position;
        // Live ball, in the third he is responsible for. 300u = 37.5 m.
        if (ball - goal).magnitude() > 300.0 || !ctx.ball().on_own_side() {
            return;
        }

        let keeper = ctx.player.position;
        let to_ball = ball - goal;
        let span = to_ball.norm();
        let rel = keeper - goal;
        // Perpendicular distance from the goal-centre→ball line: the
        // bisector he is supposed to be standing on.
        let off_angle = if span > 1.0 {
            (rel.x * to_ball.y - rel.y * to_ball.x).abs() / span
        } else {
            0.0
        };
        let ball_wide = ball.y - goal.y;
        let keeper_wide = keeper.y - goal.y;

        KeeperGuardDiag::note(0);
        KeeperGuardDiag::add(1, (off_angle * 100.0).max(0.0) as u64);
        KeeperGuardDiag::add(2, ((keeper.x - goal.x).abs() * 100.0) as u64);
        // Does reading the game buy anything? Split the same measurement
        // by the keeper's own positioning composite — the one that blends
        // positioning / anticipation / decisions / concentration.
        let read = crate::r#match::player::strategies::players::ops::goalkeeper_skill::
            GoalkeeperSkillProfile::from_ctx(ctx)
            .positioning;
        let (ticks_slot, sum_slot) = if read >= 0.55 {
            (13, 14)
        } else if read <= 0.40 {
            (15, 16)
        } else {
            (usize::MAX, usize::MAX)
        };
        KeeperGuardDiag::note(ticks_slot);
        KeeperGuardDiag::add(sum_slot, (off_angle * 100.0).max(0.0) as u64);
        KeeperGuardDiag::add(21, (read * 1000.0).max(0.0) as u64);
        // Ball 5 m or more off centre and he is displaced toward the far
        // post. There is no reading of the game in which that is right.
        if ball_wide.abs() > 40.0
            && keeper_wide.abs() > 10.0
            && ball_wide.signum() != keeper_wide.signum()
        {
            KeeperGuardDiag::note(3);
        }
        if !moving {
            KeeperGuardDiag::note(4);
        }

        // A man carrying the ball at him, inside 25 m — the situation the
        // report is about.
        let carrier = ctx
            .players()
            .opponents()
            .with_ball()
            .next()
            .is_some_and(|o| (o.position - keeper).magnitude() < 200.0);
        if carrier {
            KeeperGuardDiag::note(5);
            KeeperGuardDiag::add(9, (off_angle * 100.0).max(0.0) as u64);
            match gk_state {
                GoalkeeperState::ComingOut => KeeperGuardDiag::note(6),
                GoalkeeperState::ReturningToGoal => KeeperGuardDiag::note(7),
                GoalkeeperState::Standing | GoalkeeperState::Walking => KeeperGuardDiag::note(8),
                _ => {}
            }
        }
    }

    pub fn into_ctx(self) -> StateProcessingContext<'p> {
        StateProcessingContext::from(self)
    }

    /// Immutable view of the same situation [`Self::process`] will hand
    /// the state handler. `process` consumes the processor, so a
    /// per-group dispatcher that needs to inspect the context before
    /// (or alongside) dispatching reads it here instead.
    pub fn ctx(&self) -> StateProcessingContext<'_> {
        StateProcessingContext {
            in_state_time: self.in_state_time,
            player: self.player,
            context: self.context,
            tick_context: self.tick_context,
        }
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
    /// …and the goalkeeper's much shorter one. See
    /// [`StateChangeResult::start_keeper_cooldown`].
    pub start_keeper_cooldown: bool,
    /// Tagged reason to attach to the next Shoot event fired by this
    /// player. Matches the pass-reason pattern. Written to
    /// `player.pending_shot_reason` by `state.rs` so the Shooting state
    /// can read it when composing the event.
    pub shot_reason: Option<&'static str>,
    /// How hard `ShapeDiscipline` is recalling this player, 0..`MAX_PULL`.
    ///
    /// Consumed by `state.rs` as a FLOOR on the movement speed cap. The
    /// recall is built at the player's full top speed on purpose — the
    /// thing being modelled is a recovery run — but the cap that follows
    /// is keyed to whatever state he happens to be drifting in, and those
    /// are the low tiers: `Standing` is `Recovery` (0.12) and `Returning`
    /// / `CreatingSpace` are `Moderate` (0.52). So a forward 17 m out of
    /// shape was recalled at full speed and then throttled to an amble,
    /// which is precisely what the tether exists to stop. Without this
    /// floor the block measured **54.2 m while defending against a
    /// planned 31.3 m and a real 35-45 m**.
    pub shape_recall_pull: f32,
    /// A second FLOOR on the movement speed cap, for velocities imposed
    /// from outside the state machine.
    ///
    /// Same mechanism and the same reason as `shape_recall_pull`: the cap
    /// `state.rs` applies is keyed to whatever `ActivityIntensity` the
    /// player's CURRENT state declared, and the low tiers are near-total
    /// (`Recovery` is 0.12 of top speed). An override that replaces the
    /// state's own velocity is therefore served at the pace of a state it
    /// is no longer following — a striker told to leave the keeper's area
    /// while nominally `Standing` backs away at a twelfth of walking pace.
    /// See [`KeeperReleaseSpace`].
    pub effort_floor: f32,
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
            start_keeper_cooldown: false,
            shot_reason: None,
            shape_recall_pull: 0.0,
            effort_floor: 0.0,
        }
    }

    /// Fold a per-state [`StateChangeResult`] into this processing result.
    ///
    /// Events propagate together with a state transition. The tackle-
    /// cooldown and shot-reason side-channels propagate unconditionally —
    /// they're consumed regardless of whether the state changed.
    ///
    /// EVENTS REQUIRE A TRANSITION. A handler that emits an event without
    /// one (`state == None`) has that event dropped. This is a real
    /// constraint on state authors, not an accident: propagating
    /// event-only results globally un-masks goalkeeper save events the
    /// engine's scoring calibration was built around (GK saves jump from
    /// ~54% to ~96% of shots on target and dev_match goals/match collapse
    /// ~10×, 0.60 → 0.05), so the contract stays as-is until a dedicated
    /// recalibration pass.
    ///
    /// The one state that violated it — `ForwardCrossReceivingState`'s
    /// ground-ball `RequestBallReceive`, which is why cutbacks rolled
    /// straight through the receiver — now pairs its event with the state
    /// the forward actually moves into. `event_only_result_is_dropped`
    /// below pins the contract so the next author hits a failing test
    /// rather than a silently vanishing event.
    pub fn merge_state_change(&mut self, change: StateChangeResult) {
        self.start_tackle_cooldown = change.start_tackle_cooldown;
        self.start_keeper_cooldown = change.start_keeper_cooldown;
        self.shot_reason = change.shot_reason;
        if change.state.is_some() {
            self.state = change.state;
            self.events = change.events;
        }
    }
}

pub struct StateChangeResult {
    pub state: Option<PlayerState>,

    pub events: EventCollection,

    /// Defender signalled "I just attempted a tackle" — the state.rs
    /// update loop consumes this and bumps `player.tackle_cooldown` so
    /// the next ~100 ticks of Tackling-state entries short-circuit
    /// without rolling an attempt. Must live on the result (not be
    /// applied directly in the state) because `ctx.player` is an
    /// immutable borrow inside the state processor.
    pub start_tackle_cooldown: bool,
    /// Same signal for a GOALKEEPER who has just gone to ground at a
    /// carrier's feet. Its own flag because the two cooldowns are wildly
    /// different lengths and for different reasons: a defender's is thirty
    /// seconds and exists to hold the whole team's tackle COUNT down to a
    /// realistic one, while a keeper's is a few seconds and exists only so
    /// that a smother he loses does not repeat on the tick he gets up. Sharing
    /// the defender's constant would leave a beaten keeper standing and
    /// watching the next man through for half a minute.
    /// See `MatchPlayer::start_keeper_cooldown`.
    pub start_keeper_cooldown: bool,
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
            events: EventCollection::new(),
            start_tackle_cooldown: false,
            start_keeper_cooldown: false,
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

#[cfg(test)]
mod merge_tests {
    use super::{StateChangeResult, StateProcessingResult};
    use crate::r#match::events::Event;
    use crate::r#match::forwarders::states::ForwardState;
    use crate::r#match::player::events::PlayerEvent;
    use crate::r#match::player::state::PlayerState;

    #[test]
    fn state_transition_with_event_propagates_both() {
        // A transition that also carries an event keeps both halves — this
        // is how every save / shot / pass event reaches the dispatcher.
        let mut result = StateProcessingResult::new();
        result.merge_state_change(StateChangeResult::with_forward_state_and_event(
            ForwardState::Heading,
            Event::PlayerEvent(PlayerEvent::RequestBallReceive(3)),
        ));

        assert_eq!(
            result.state,
            Some(PlayerState::Forward(ForwardState::Heading))
        );
        assert!(result.events.has_events());
    }

    #[test]
    fn event_only_result_is_dropped() {
        // The merge contract: events ride along with a transition. An
        // event with no state change is NOT propagated — propagating them
        // globally un-masks goalkeeper save events and collapses scoring
        // (see `merge_state_change`). If this assertion ever flips, it
        // must land together with a GK recalibration.
        //
        // State authors: if you need to emit an event, transition. The
        // `with_*_state_and_event` constructors exist for exactly this.
        let mut result = StateProcessingResult::new();
        result.merge_state_change(StateChangeResult::with_event(Event::PlayerEvent(
            PlayerEvent::RequestBallReceive(7),
        )));

        assert!(result.state.is_none());
        assert!(
            !result.events.has_events(),
            "event-only results are dropped (see merge_state_change)"
        );
    }

    #[test]
    fn no_state_emits_an_event_without_transitioning() {
        // Source-level guard for the merge contract above: an
        // `event_only` result is silently discarded, so no state handler
        // may construct one. `ForwardCrossReceivingState` used to, and its
        // ground-ball receive request vanished every time — cutbacks
        // rolled straight through the forward.
        let states_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("match")
            .join("engine")
            .join("player")
            .join("strategies");

        struct Scanner;
        impl Scanner {
            fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        Self::walk(&path, hits);
                        continue;
                    }
                    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                        continue;
                    }
                    // The processor itself defines and tests the helpers.
                    if path.file_name().and_then(|f| f.to_str()) == Some("processor.rs") {
                        continue;
                    }
                    let Ok(src) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    if src.contains("StateChangeResult::with_event(")
                        || src.contains("StateChangeResult::with_events(")
                    {
                        hits.push(path.display().to_string());
                    }
                }
            }
        }

        let mut hits = Vec::new();
        Scanner::walk(&states_dir, &mut hits);
        assert!(
            hits.is_empty(),
            "state handler(s) build an event-only StateChangeResult, whose events \
             the merge drops — pair the event with a transition via \
             `with_*_state_and_event`: {hits:?}"
        );
    }

    #[test]
    fn side_channels_propagate_without_state_change() {
        // The tackle-cooldown and shot-reason side-channels DO propagate
        // regardless of state — a tackle that keeps the current state still
        // starts its cooldown.
        let mut result = StateProcessingResult::new();
        let mut change = StateChangeResult::new();
        change.start_tackle_cooldown = true;
        change.shot_reason = Some("TEST");
        result.merge_state_change(change);

        assert!(result.start_tackle_cooldown);
        assert_eq!(result.shot_reason, Some("TEST"));
        assert!(result.state.is_none());
    }
}
