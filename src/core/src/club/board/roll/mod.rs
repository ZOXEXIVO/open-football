//! Deterministic rolls for a board that has no random number generator.
//!
//! `GlobalContext` carries no seeded RNG, so a board that drew from the
//! global `IntegerUtils::random` would make decisions a save could not
//! replay. Every chance the boardroom takes is instead a hash of the
//! things that actually distinguish the situation — which club, which day,
//! and a per-decision salt — so identical circumstances always produce the
//! identical decision and a reloaded save walks the same path.

use chrono::{Datelike, NaiveDate};

/// A reproducible 0..99 draw.
pub struct DeterministicRoll;

impl DeterministicRoll {
    /// Salt for the monthly takeover watch. The value passed alongside it
    /// is the months the watch has spent in its current status.
    pub const TAKEOVER: u64 = 0;

    /// Salt for the football a fresh owner decides he wants to watch.
    pub const VISION_STYLE: u64 = 0x51;

    /// Mix the club, the calendar day and a caller's salt through a
    /// splitmix64 finalizer and take the result modulo 100.
    ///
    /// Well-distributed but fully reproducible from `(club, date, salt)`,
    /// so saves and tests replay identically.
    pub fn percent(club_id: u32, date: NaiveDate, salt: u64) -> u8 {
        let day = date.num_days_from_ce() as u64;
        let mut x = ((club_id as u64) << 32)
            ^ day.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ salt.wrapping_add(1).wrapping_mul(0xD1B5_4A32_D192_ED03);
        // splitmix64 finalizer.
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        (x % 100) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 3, d).unwrap()
    }

    #[test]
    fn the_same_situation_always_rolls_the_same_number() {
        let a = DeterministicRoll::percent(41, day(9), 2);
        let b = DeterministicRoll::percent(41, day(9), 2);
        assert_eq!(a, b);
        assert!(a < 100);
    }

    #[test]
    fn a_different_club_day_or_salt_rolls_differently() {
        let base = DeterministicRoll::percent(41, day(9), 2);
        let by_club: Vec<u8> = (0..40)
            .map(|c| DeterministicRoll::percent(c, day(9), 2))
            .collect();
        assert!(
            by_club.iter().any(|r| *r != base),
            "every club rolled the same number"
        );
        let by_day: Vec<u8> = (1..28)
            .map(|d| DeterministicRoll::percent(41, day(d), 2))
            .collect();
        assert!(
            by_day.iter().any(|r| *r != base),
            "every day rolled the same number"
        );
        let by_salt: Vec<u8> = (0..40)
            .map(|s| DeterministicRoll::percent(41, day(9), s))
            .collect();
        assert!(
            by_salt.iter().any(|r| *r != base),
            "every salt rolled the same number"
        );
    }
}
