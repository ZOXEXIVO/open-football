//! The match clock and the period lengths it runs against.

#[cfg(debug_assertions)]
pub const MATCH_HALF_TIME_MS: u64 = 5 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_HALF_TIME_MS: u64 = 45 * 60 * 1000;

pub const MATCH_TIME_MS: u64 = MATCH_HALF_TIME_MS * 2;

/// One of the two periods of extra time.
#[cfg(debug_assertions)]
pub const MATCH_EXTRA_TIME_MS: u64 = 100 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_EXTRA_TIME_MS: u64 = 15 * 60 * 1000;

pub struct MatchTime {
    pub time: u64,
}

impl Default for MatchTime {
    fn default() -> Self {
        Self::new()
    }
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
}
