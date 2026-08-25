//! The match clock: the 10 ms tick, the period boundaries, stoppage-time
//! accounting, and the goal/concede timestamps the rest of the engine
//! measures "recently" against.

use super::match_context::MatchContext;
use crate::r#match::{MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MatchState, MatchTime, PlayerSide};

/// How much match clock one engine tick is worth, in milliseconds.
///
/// Public because a tick is the unit every patience bound, cooldown and
/// commitment clock in the engine is written in, so anything converting a
/// duration into ticks needs this number rather than its own copy of `10`.
pub const MATCH_TIME_INCREMENT_MS: u64 = 10;
const MAX_STOPPAGE_PER_PERIOD_MS: u64 = 15 * 60 * 1000;

impl MatchContext {
    pub fn increment_time(&mut self) -> bool {
        let new_time = self.time.increment(MATCH_TIME_INCREMENT_MS);

        self.total_match_time += MATCH_TIME_INCREMENT_MS;

        match self.state.match_state {
            MatchState::FirstHalf | MatchState::SecondHalf => {
                new_time < MATCH_HALF_TIME_MS + self.period_stoppage_time_ms
            }
            MatchState::ExtraTime => new_time < MATCH_EXTRA_TIME_MS + self.period_stoppage_time_ms,
            _ => false,
        }
    }

    pub fn reset_period_time(&mut self) {
        self.time = MatchTime::new();
        self.period_stoppage_time_ms = 0;
    }

    pub fn add_time(&mut self, time: u64) {
        self.time.increment(time);
        self.total_match_time += time;
    }

    pub fn record_stoppage_time(&mut self, time: u64) {
        if !matches!(
            self.state.match_state,
            MatchState::FirstHalf | MatchState::SecondHalf | MatchState::ExtraTime
        ) {
            return;
        }

        let room = MAX_STOPPAGE_PER_PERIOD_MS.saturating_sub(self.period_stoppage_time_ms);
        let added = time.min(room);
        self.period_stoppage_time_ms += added;
        self.additional_time_ms += added;
    }

    pub fn current_tick(&self) -> u64 {
        self.total_match_time / 10
    }

    pub fn can_shoot_after_goal(&self) -> bool {
        true
    }

    pub fn record_goal_tick(&mut self) {
        self.last_goal_tick = self.current_tick();
    }

    /// Mark that the given side just conceded a goal. Read by the
    /// forward shot decision to dampen willingness in the immediate
    /// post-concede window. See `last_conceded_tick` docs for the
    /// mechanism rationale.
    pub fn record_conceded(&mut self, side: PlayerSide) {
        let tick = self.current_tick();
        let idx = match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        };
        self.last_conceded_tick[idx] = tick;
    }

    /// Did the given side concede within the last `window_ticks` ticks?
    /// One tick is 10 ms of match time, so 6000 ticks ≈ 60 s.
    pub fn conceded_recently(&self, side: PlayerSide, window_ticks: u64) -> bool {
        let idx = match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        };
        let last = self.last_conceded_tick[idx];
        if last == u64::MAX {
            return false;
        }
        self.current_tick().saturating_sub(last) < window_ticks
    }
}
