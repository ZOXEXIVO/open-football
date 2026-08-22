//! The match clock and the period lengths it runs against.

#[cfg(debug_assertions)]
pub const MATCH_HALF_TIME_MS: u64 = 5 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_HALF_TIME_MS: u64 = 45 * 60 * 1000;

pub const MATCH_TIME_MS: u64 = MATCH_HALF_TIME_MS * 2;

/// Extra time is a single continuous 30-minute period in this simulation.
/// Real football splits it into 2×15 with an interval; we skip the break
/// since there's no tactical depth to add between the two halves here.
#[cfg(debug_assertions)]
pub const MATCH_EXTRA_TIME_MS: u64 = 3 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_EXTRA_TIME_MS: u64 = 30 * 60 * 1000;

pub struct MatchTime {
    pub time: u64,
}

impl MatchTime {
    pub fn new() -> Self {
        MatchTime { time: 0 }
    }

    #[inline]
    pub fn increment(&mut self, val: u64) -> u64 {
        self.time += val;
        self.time
    }

    pub fn is_running_out(&self) -> bool {
        self.time > (2 * MATCH_TIME_MS / 3)
    }
}
