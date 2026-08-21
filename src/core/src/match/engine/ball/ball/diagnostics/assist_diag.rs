//! Why a goal did or didn't carry an assist. The credited-assist rate is
//! a headline realism number (real football assists ~70% of goals), and
//! the count alone can't say whether the resolver is too strict or the
//! engine simply isn't scoring off passes. These split the outcomes at
//! the one decision point that knows: `assist_for_goal`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Non-own goals that reached the resolver.
pub static GOALS: AtomicU64 = AtomicU64::new(0);
/// Pass chain was empty — nothing was recorded, or a clear wiped it.
pub static EMPTY_CHAIN: AtomicU64 = AtomicU64::new(0);
/// Newest chain entry belongs to the conceding team: the scoring team
/// won the ball and finished without completing a pass of its own.
pub static OPPONENT_CHAIN: AtomicU64 = AtomicU64::new(0);
/// Of those, how many still had a scoring-team pass deeper in the
/// ring — i.e. the same-possession rule is what rejected them, not
/// the absence of a teammate's pass.
pub static OPPONENT_CHAIN_HAS_TEAMMATE: AtomicU64 = AtomicU64::new(0);
/// Age in ticks of the blocking opponent entry, summed.
pub static OPPONENT_CHAIN_AGE: AtomicU64 = AtomicU64::new(0);
/// Only the scorer appears in the chain (they passed, got it back).
pub static SCORER_ONLY: AtomicU64 = AtomicU64::new(0);
/// A teammate's pass was there but older than `ASSIST_WINDOW_TICKS`.
pub static STALE: AtomicU64 = AtomicU64::new(0);
pub static CREDITED: AtomicU64 = AtomicU64::new(0);
/// Sum of (goal tick − assist pass tick) over credited assists, so
/// the harness can print the mean delay and size the window.
pub static CREDITED_DELAY_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn reset() {
    for c in [
        &GOALS,
        &EMPTY_CHAIN,
        &OPPONENT_CHAIN,
        &OPPONENT_CHAIN_HAS_TEAMMATE,
        &OPPONENT_CHAIN_AGE,
        &SCORER_ONLY,
        &STALE,
        &CREDITED,
        &CREDITED_DELAY_TICKS,
    ] {
        c.store(0, Ordering::Relaxed);
    }
}

/// `(goals, empty, opponent, scorer_only, stale, credited, delay_sum)`
pub fn snapshot() -> (u64, u64, u64, u64, u64, u64, u64) {
    (
        GOALS.load(Ordering::Relaxed),
        EMPTY_CHAIN.load(Ordering::Relaxed),
        OPPONENT_CHAIN.load(Ordering::Relaxed),
        SCORER_ONLY.load(Ordering::Relaxed),
        STALE.load(Ordering::Relaxed),
        CREDITED.load(Ordering::Relaxed),
        CREDITED_DELAY_TICKS.load(Ordering::Relaxed),
    )
}

/// `(opponent_chain_with_teammate_deeper, opponent_entry_age_sum)`
pub fn opponent_chain_detail() -> (u64, u64) {
    (
        OPPONENT_CHAIN_HAS_TEAMMATE.load(Ordering::Relaxed),
        OPPONENT_CHAIN_AGE.load(Ordering::Relaxed),
    )
}
