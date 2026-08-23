//! Compact day-resolution clock for the mind.
//!
//! Every memory record stores its date as a [`EpochDay`] — a `u16` count
//! of days since [`MindClock::EPOCH`] — rather than a `NaiveDate`. Three
//! reasons:
//!
//! * **Size.** A `NaiveDate` is 4 bytes and carries no arithmetic the
//!   memory stores need; a `u16` halves it and packs an episode into 16
//!   bytes. At tens of thousands of players that difference is the whole
//!   budget.
//! * **Arithmetic.** Every forgetting / drift calculation wants "days
//!   between", which is a `u16` subtraction here instead of a
//!   `signed_duration_since().num_days()` call per episode per recall.
//! * **Determinism.** The stores never need calendar semantics, so they
//!   never risk disagreeing with the rest of the sim about one.
//!
//! `u16` spans ~179 years from the epoch, which puts the ceiling well
//! past any simulated career. Dates before the epoch saturate to day 0
//! rather than wrapping.

use chrono::NaiveDate;

/// Days since [`MindClock::EPOCH`]. See the module docs for why the
/// mind keeps its own clock.
pub type EpochDay = u16;

/// Conversion between the simulator's `NaiveDate` and the mind's
/// compact [`EpochDay`].
pub struct MindClock;

impl MindClock {
    /// Anchor for [`EpochDay`]. Chosen far enough before any simulated
    /// season that no in-sim date saturates, and late enough that the
    /// `u16` ceiling (~2179) is unreachable.
    pub const EPOCH_YEAR: i32 = 2000;

    /// The epoch itself. Constructed rather than `const` because
    /// `NaiveDate::from_ymd_opt` is not a const fn.
    #[inline]
    pub fn epoch() -> NaiveDate {
        NaiveDate::from_ymd_opt(Self::EPOCH_YEAR, 1, 1).expect("mind epoch is a valid date")
    }

    /// Compress a calendar date into an [`EpochDay`]. Dates before the
    /// epoch clamp to 0; dates past the `u16` horizon clamp to
    /// `u16::MAX`. Both clamps are unreachable for in-sim dates and
    /// exist so a stray date can never wrap into a plausible-looking
    /// wrong day.
    #[inline]
    pub fn day(date: NaiveDate) -> EpochDay {
        let days = (date - Self::epoch()).num_days();
        days.clamp(0, u16::MAX as i64) as EpochDay
    }

    /// Expand an [`EpochDay`] back to a calendar date. Round-trips with
    /// [`Self::day`] for every in-sim date.
    #[inline]
    pub fn date(day: EpochDay) -> NaiveDate {
        Self::epoch() + chrono::Duration::days(day as i64)
    }

    /// Days elapsed from `then` to `now`, saturating at 0 when `then`
    /// is in the future. A future timestamp means a record was written
    /// with a date the caller had not reached yet — treat it as "just
    /// happened" rather than letting the subtraction wrap.
    #[inline]
    pub fn elapsed(then: EpochDay, now: EpochDay) -> u16 {
        now.saturating_sub(then)
    }

    /// [`Self::elapsed`] as `f32`, for the continuous curves that
    /// consume it (forgetting, ledger drift).
    #[inline]
    pub fn elapsed_f32(then: EpochDay, now: EpochDay) -> f32 {
        Self::elapsed(then, now) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn epoch_is_day_zero() {
        assert_eq!(MindClock::day(d(2000, 1, 1)), 0);
    }

    #[test]
    fn round_trips_for_in_sim_dates() {
        for date in [d(2026, 8, 23), d(2000, 1, 2), d(2100, 12, 31)] {
            assert_eq!(MindClock::date(MindClock::day(date)), date);
        }
    }

    #[test]
    fn pre_epoch_saturates_rather_than_wrapping() {
        assert_eq!(MindClock::day(d(1985, 6, 1)), 0);
    }

    #[test]
    fn a_decade_is_about_three_thousand_six_hundred_days() {
        let then = MindClock::day(d(2026, 8, 23));
        let now = MindClock::day(d(2036, 8, 23));
        let elapsed = MindClock::elapsed(then, now);
        assert!(
            (3650..=3653).contains(&elapsed),
            "ten years should be ~3652 days, got {elapsed}"
        );
    }

    #[test]
    fn future_timestamps_saturate_to_zero_elapsed() {
        let then = MindClock::day(d(2030, 1, 1));
        let now = MindClock::day(d(2026, 1, 1));
        assert_eq!(MindClock::elapsed(then, now), 0);
    }
}
