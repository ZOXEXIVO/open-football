//! Per-player rolling workload and form rating. Feeds squad rotation,
//! injury risk, and form-based morale. Windows use exponential decay
//! rather than per-day buffers so the struct stays 16 bytes.

use chrono::{Datelike, NaiveDate};

const DECAY_7: f32 = 6.0 / 7.0;
const DECAY_30: f32 = 29.0 / 30.0;

/// EMA coefficient for form_rating. 0.33 gives a half-life of ~2 matches —
/// quick enough to catch a hot streak, slow enough to smooth a one-off.
const FORM_ALPHA: f32 = 0.33;

/// Weekly minutes at which selection starts penalising the player (≈5 × 90).
pub const FATIGUE_LOAD_THRESHOLD: f32 = 450.0;
/// Weekly minutes treated as dangerous overload — injury risk kicks in too.
pub const FATIGUE_LOAD_DANGER: f32 = 650.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerLoad {
    /// Recency-weighted competitive minutes over the trailing ~7 days.
    pub minutes_last_7: f32,
    /// Recency-weighted competitive minutes over the trailing ~30 days.
    pub minutes_last_30: f32,
    /// Packed per-day bit array; bit 0 = today. Counts matches in last 14 days.
    pub matches_last_14_bits: u16,
    /// EMA of effective match ratings (1.0–10.0). Zero until the first match.
    pub form_rating: f32,
    /// Last date (CE ordinal) we aged the windows. 0 means uninitialised.
    pub last_decay_day_ordinal: i32,
}

impl PlayerLoad {
    pub const fn new() -> Self {
        Self {
            minutes_last_7: 0.0,
            minutes_last_30: 0.0,
            matches_last_14_bits: 0,
            form_rating: 0.0,
            last_decay_day_ordinal: 0,
        }
    }

    /// Age the windows by whole days since the last call. Idempotent on
    /// the same date; catches up multi-day gaps in one call.
    pub fn daily_decay(&mut self, today: NaiveDate) {
        let today_ordinal = today.num_days_from_ce();

        if self.last_decay_day_ordinal == 0 {
            self.last_decay_day_ordinal = today_ordinal;
            return;
        }

        let delta_days = (today_ordinal - self.last_decay_day_ordinal).max(0);
        if delta_days == 0 {
            return;
        }
        self.last_decay_day_ordinal = today_ordinal;

        self.minutes_last_7 *= DECAY_7.powi(delta_days);
        self.minutes_last_30 *= DECAY_30.powi(delta_days);

        if delta_days >= 14 {
            self.matches_last_14_bits = 0;
        } else {
            self.matches_last_14_bits <<= delta_days as u32;
            self.matches_last_14_bits &= (1 << 14) - 1;
        }

        // Floor f32 residuals so repeated decays don't leave negligible
        // noise that defeats equality in tests.
        if self.minutes_last_7 < 0.1 {
            self.minutes_last_7 = 0.0;
        }
        if self.minutes_last_30 < 0.1 {
            self.minutes_last_30 = 0.0;
        }
    }

    /// Record a competitive match. Friendlies don't burden workload.
    pub fn record_match_minutes(&mut self, minutes: f32, is_friendly: bool) {
        if is_friendly || minutes <= 0.0 {
            return;
        }
        self.minutes_last_7 += minutes;
        self.minutes_last_30 += minutes;
        self.matches_last_14_bits |= 1;
    }

    /// Fold a match rating into the form EMA. Out-of-range inputs are ignored.
    pub fn update_form(&mut self, rating: f32) {
        if !(1.0..=10.0).contains(&rating) {
            return;
        }
        if self.form_rating <= 0.0 {
            self.form_rating = rating;
        } else {
            self.form_rating = self.form_rating * (1.0 - FORM_ALPHA) + rating * FORM_ALPHA;
        }
    }

    pub fn matches_last_14(&self) -> u8 {
        self.matches_last_14_bits.count_ones() as u8
    }

    pub fn is_fatigued(&self) -> bool {
        self.minutes_last_7 >= FATIGUE_LOAD_THRESHOLD
    }

    pub fn is_overloaded(&self) -> bool {
        self.minutes_last_7 >= FATIGUE_LOAD_DANGER
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn new_is_zero() {
        let l = PlayerLoad::new();
        assert_eq!(l.minutes_last_7, 0.0);
        assert_eq!(l.matches_last_14(), 0);
        assert_eq!(l.form_rating, 0.0);
    }

    #[test]
    fn friendly_does_not_accumulate() {
        let mut l = PlayerLoad::new();
        l.record_match_minutes(90.0, true);
        assert_eq!(l.minutes_last_7, 0.0);
        assert_eq!(l.matches_last_14(), 0);
    }

    #[test]
    fn competitive_match_increments_windows() {
        let mut l = PlayerLoad::new();
        l.record_match_minutes(90.0, false);
        assert_eq!(l.minutes_last_7, 90.0);
        assert_eq!(l.minutes_last_30, 90.0);
        assert_eq!(l.matches_last_14(), 1);
    }

    #[test]
    fn form_ema_seeds_from_first_rating() {
        let mut l = PlayerLoad::new();
        l.update_form(7.5);
        assert!((l.form_rating - 7.5).abs() < 1e-4);
    }

    #[test]
    fn form_ema_converges_toward_run_of_ratings() {
        let mut l = PlayerLoad::new();
        l.update_form(6.0);
        for _ in 0..20 {
            l.update_form(8.0);
        }
        // Should be close to 8.0 after many applications, not still 6-ish.
        assert!(l.form_rating > 7.8, "form_rating={}", l.form_rating);
    }

    #[test]
    fn form_ignores_out_of_range() {
        let mut l = PlayerLoad::new();
        l.update_form(15.0);
        l.update_form(-1.0);
        assert_eq!(l.form_rating, 0.0);
    }

    #[test]
    fn daily_decay_ages_minute_windows() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1)); // seed
        l.record_match_minutes(90.0, false);
        assert_eq!(l.minutes_last_7, 90.0);

        // After 7 days of decay, last_7 should have fallen materially
        // (factor (6/7)^7 ≈ 0.34).
        for i in 2..=8 {
            l.daily_decay(d(2025, 1, i));
        }
        assert!(l.minutes_last_7 < 45.0, "after a week: {}", l.minutes_last_7);
        assert!(l.minutes_last_7 > 20.0, "decay shouldn't be complete: {}", l.minutes_last_7);

        // last_30 decays slower — after 7 days it should still be > 70
        // (factor (29/30)^7 ≈ 0.79).
        assert!(l.minutes_last_30 > 65.0, "last_30 after a week: {}", l.minutes_last_30);
    }

    #[test]
    fn daily_decay_is_idempotent_same_day() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1));
        l.record_match_minutes(90.0, false);
        l.daily_decay(d(2025, 1, 1)); // same day — no change
        assert_eq!(l.minutes_last_7, 90.0);
        l.daily_decay(d(2025, 1, 1));
        assert_eq!(l.minutes_last_7, 90.0);
    }

    #[test]
    fn daily_decay_handles_multi_day_gap() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1));
        l.record_match_minutes(90.0, false);
        // Skip 14 days — catch up in one call.
        l.daily_decay(d(2025, 1, 15));
        // (6/7)^14 ≈ 0.117 → ~10.5
        assert!(l.minutes_last_7 < 15.0);
    }

    #[test]
    fn matches_last_14_bit_array_shifts() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1));

        l.record_match_minutes(90.0, false); // day 0
        l.daily_decay(d(2025, 1, 3));         // +2
        l.record_match_minutes(90.0, false);  // day 2
        l.daily_decay(d(2025, 1, 6));         // +3
        l.record_match_minutes(90.0, false);  // day 5
        assert_eq!(l.matches_last_14(), 3);

        // Fourteen days after the first match — it should drop out.
        l.daily_decay(d(2025, 1, 16)); // delta from last=10; first match now at slot 15 → gone
        assert_eq!(l.matches_last_14(), 2);
    }

    #[test]
    fn fatigue_thresholds() {
        let mut l = PlayerLoad::new();
        l.minutes_last_7 = 500.0;
        assert!(l.is_fatigued());
        assert!(!l.is_overloaded());

        l.minutes_last_7 = 700.0;
        assert!(l.is_overloaded());
    }
}
