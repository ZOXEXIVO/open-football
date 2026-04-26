//! Per-player rolling workload, recovery debt, and form rating.
//!
//! Feeds squad rotation, injury risk, and form-based morale. Two concepts
//! coexist by design:
//!
//! * **Minutes** — raw competitive time (kept for backwards-compatibility
//!   and friendly-vs-competitive bookkeeping).
//! * **Physical load** — minutes weighted by position group, intensity,
//!   and post-drain condition. A 90-min keeper and a 90-min wingback
//!   carry very different physical loads.
//!
//! `physical_load_7` and `physical_load_30` are exponentially decayed in
//! the same shape as the minute windows so the struct stays compact and
//! `daily_decay()` remains a single hot path. `recovery_debt` accumulates
//! from heavy sessions and bleeds off with rest/recovery work — the
//! "deeper than condition" tiredness scouts and physios talk about.
//!
//! All new fields default to zero, so old saves and freshly-built
//! players load with a clean slate.
//!
//! Scale notes:
//! * 1.0 unit of physical_load ≈ 1 minute of a center-back at neutral
//!   intensity. A 90-min full-back tops out around 100, a 90-min keeper
//!   around 40. ACWR ratios above ~1.5 are the sports-science danger
//!   zone.
//! * `recovery_debt` is in the same "minute-equivalent" scale so it
//!   decays at the same rate. A heavy match adds 40-80; recovery
//!   sessions burn it back down.

use chrono::{Datelike, NaiveDate};

const DECAY_7: f32 = 6.0 / 7.0;
const DECAY_30: f32 = 29.0 / 30.0;

/// Per-day decay factor for recovery debt — debt half-life ~3 days
/// (a one-off heavy match is mostly gone by next weekend).
const RECOVERY_DEBT_DAILY_DECAY: f32 = 0.79;

/// EMA coefficient for form_rating. 0.33 gives a half-life of ~2 matches —
/// quick enough to catch a hot streak, slow enough to smooth a one-off.
const FORM_ALPHA: f32 = 0.33;

/// Weekly minutes at which selection starts penalising the player (≈5 × 90).
pub const FATIGUE_LOAD_THRESHOLD: f32 = 450.0;
/// Weekly minutes treated as dangerous overload — injury risk kicks in too.
pub const FATIGUE_LOAD_DANGER: f32 = 650.0;

/// Physical-load equivalents of the minute thresholds above. A center-back
/// pegs ≈1 unit / minute, so the bands sit a touch lower to leave room for
/// position weighting and depletion bumps.
pub const PHYSICAL_LOAD_THRESHOLD: f32 = 420.0;
pub const PHYSICAL_LOAD_DANGER: f32 = 620.0;

/// Acute:chronic workload ratio considered an "elevated risk" spike.
/// Sports-science consensus puts the danger zone at 1.5+ — we surface it
/// to selection so coaches rotate around training spikes.
pub const WORKLOAD_SPIKE_RATIO: f32 = 1.4;

/// Recovery debt at which the player feels heavy-legged irrespective of
/// raw weekly minutes — used by selection / UI labels.
pub const RECOVERY_DEBT_HEAVY: f32 = 350.0;

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

    /// Recency-weighted physical-load units (minutes weighted by position,
    /// intensity, and depletion). Decays in the same envelope as
    /// `minutes_last_7`.
    pub physical_load_7: f32,
    /// Recency-weighted 30-day physical load. Used as the chronic baseline
    /// for the acute:chronic ratio.
    pub physical_load_30: f32,
    /// Recency-weighted high-intensity load (sprints, presses, repeated
    /// accelerations). Subset of physical_load_7.
    pub high_intensity_load_7: f32,
    /// Cumulative "deep tiredness" — built up by heavy matches/training
    /// and bled off by recovery sessions. Decays daily on its own clock.
    pub recovery_debt: f32,
    /// Last computed match load (diagnostic / debugging — not driving any
    /// selection logic on its own).
    pub last_match_load: f32,
}

impl PlayerLoad {
    pub const fn new() -> Self {
        Self {
            minutes_last_7: 0.0,
            minutes_last_30: 0.0,
            matches_last_14_bits: 0,
            form_rating: 0.0,
            last_decay_day_ordinal: 0,
            physical_load_7: 0.0,
            physical_load_30: 0.0,
            high_intensity_load_7: 0.0,
            recovery_debt: 0.0,
            last_match_load: 0.0,
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

        let d7 = DECAY_7.powi(delta_days);
        let d30 = DECAY_30.powi(delta_days);
        let d_debt = RECOVERY_DEBT_DAILY_DECAY.powi(delta_days);

        self.minutes_last_7 *= d7;
        self.minutes_last_30 *= d30;
        self.physical_load_7 *= d7;
        self.physical_load_30 *= d30;
        self.high_intensity_load_7 *= d7;
        self.recovery_debt *= d_debt;

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
        if self.physical_load_7 < 0.1 {
            self.physical_load_7 = 0.0;
        }
        if self.physical_load_30 < 0.1 {
            self.physical_load_30 = 0.0;
        }
        if self.high_intensity_load_7 < 0.1 {
            self.high_intensity_load_7 = 0.0;
        }
        if self.recovery_debt < 0.5 {
            self.recovery_debt = 0.0;
        }
    }

    /// Record a competitive match. Friendlies don't burden minute windows
    /// (rotation/selection should ignore preseason XIs), but call
    /// `record_match_load` separately for friendly load if you want it
    /// counted at a reduced rate.
    pub fn record_match_minutes(&mut self, minutes: f32, is_friendly: bool) {
        if is_friendly || minutes <= 0.0 {
            return;
        }
        self.minutes_last_7 += minutes;
        self.minutes_last_30 += minutes;
        self.matches_last_14_bits |= 1;
    }

    /// Record physical load from a match. Friendlies count at a reduced
    /// rate so cameo minutes still register but pre-season tours don't
    /// flag every player as overloaded. `hi_load` is the high-intensity
    /// share of the same load.
    pub fn record_match_load(&mut self, load: f32, hi_load: f32, is_friendly: bool) {
        if load <= 0.0 {
            return;
        }
        let factor = if is_friendly { 0.45 } else { 1.0 };
        let l = load * factor;
        let h = hi_load.max(0.0) * factor;
        self.physical_load_7 += l;
        self.physical_load_30 += l;
        self.high_intensity_load_7 += h;
        self.last_match_load = l;
    }

    /// Record physical load from a training session. Always counted (no
    /// friendly discount); training never marks a "match" in the 14-day
    /// bit array.
    pub fn record_training_load(&mut self, load: f32, hi_load: f32) {
        if load <= 0.0 {
            return;
        }
        self.physical_load_7 += load;
        self.physical_load_30 += load;
        self.high_intensity_load_7 += hi_load.max(0.0);
    }

    /// Add to recovery debt. Called from match exertion and heavy training.
    pub fn add_recovery_debt(&mut self, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.recovery_debt = (self.recovery_debt + amount).min(2_000.0);
    }

    /// Reduce recovery debt — called by recovery / rest sessions and slow
    /// natural recovery on rest days.
    pub fn consume_recovery_debt(&mut self, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.recovery_debt = (self.recovery_debt - amount).max(0.0);
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
            || self.physical_load_7 >= PHYSICAL_LOAD_THRESHOLD
    }

    pub fn is_overloaded(&self) -> bool {
        self.minutes_last_7 >= FATIGUE_LOAD_DANGER
            || self.physical_load_7 >= PHYSICAL_LOAD_DANGER
    }

    /// Acute:chronic workload ratio. The chronic baseline is the 30-day
    /// load scaled to a one-week window; floor at 1.0 so a player coming
    /// out of the off-season doesn't read as a 30× spike on his first
    /// session back.
    pub fn workload_spike_ratio(&self) -> f32 {
        let chronic_weekly = (self.physical_load_30 / 30.0) * 7.0;
        let denom = chronic_weekly.max(1.0);
        self.physical_load_7 / denom
    }

    /// True when the acute:chronic ratio is in the danger zone — flagged
    /// to selection as a meaningful injury-risk signal.
    pub fn is_workload_spike(&self) -> bool {
        // Need a multi-week chronic baseline before we trust the ratio.
        // With a single match, physical_load_30 ≈ physical_load_7 and the
        // ratio is mathematically guaranteed to read as a spike — that's
        // arithmetic, not reality.
        self.physical_load_30 >= 200.0 && self.workload_spike_ratio() >= WORKLOAD_SPIKE_RATIO
    }

    /// Has the player accumulated enough deep tiredness to need a real
    /// rest, even if his weekly minutes look fine? Used by UI labels.
    pub fn has_heavy_legs(&self) -> bool {
        self.recovery_debt >= RECOVERY_DEBT_HEAVY
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
        assert_eq!(l.physical_load_7, 0.0);
        assert_eq!(l.recovery_debt, 0.0);
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
    fn match_load_records_into_load_windows() {
        let mut l = PlayerLoad::new();
        l.record_match_load(85.0, 25.0, false);
        assert_eq!(l.physical_load_7, 85.0);
        assert_eq!(l.physical_load_30, 85.0);
        assert_eq!(l.high_intensity_load_7, 25.0);
        assert_eq!(l.last_match_load, 85.0);
    }

    #[test]
    fn match_load_friendly_is_discounted_but_present() {
        let mut l = PlayerLoad::new();
        l.record_match_load(100.0, 30.0, true);
        // Friendly factor 0.45 → 45.0 / 13.5
        assert!((l.physical_load_7 - 45.0).abs() < 0.01);
        assert!((l.high_intensity_load_7 - 13.5).abs() < 0.01);
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

        // last_30 decays slower — after 7 days it should still be > 65
        // (factor (29/30)^7 ≈ 0.79).
        assert!(l.minutes_last_30 > 65.0, "last_30 after a week: {}", l.minutes_last_30);
    }

    #[test]
    fn daily_decay_ages_physical_load_windows() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1));
        l.record_match_load(90.0, 25.0, false);
        assert_eq!(l.physical_load_7, 90.0);
        assert_eq!(l.high_intensity_load_7, 25.0);

        for i in 2..=8 {
            l.daily_decay(d(2025, 1, i));
        }
        assert!(l.physical_load_7 < 45.0);
        assert!(l.physical_load_30 > 65.0);
        assert!(l.high_intensity_load_7 < 13.0);
    }

    #[test]
    fn recovery_debt_decays_faster_than_minute_windows() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1));
        l.add_recovery_debt(400.0);
        l.record_match_minutes(90.0, false);

        for i in 2..=4 {
            l.daily_decay(d(2025, 1, i));
        }
        // After 3 days, debt half-life ≈ 3d → roughly halved.
        assert!(l.recovery_debt < 250.0, "debt after 3d: {}", l.recovery_debt);
        // Minutes window decays slower: (6/7)^3 ≈ 0.63 → ~57.
        assert!(l.minutes_last_7 > 50.0);
    }

    #[test]
    fn consume_recovery_debt_drains_but_floors_at_zero() {
        let mut l = PlayerLoad::new();
        l.add_recovery_debt(100.0);
        l.consume_recovery_debt(40.0);
        assert!((l.recovery_debt - 60.0).abs() < 0.01);
        l.consume_recovery_debt(1_000.0);
        assert_eq!(l.recovery_debt, 0.0);
    }

    #[test]
    fn workload_spike_ratio_needs_baseline() {
        let mut l = PlayerLoad::new();
        // No baseline: should not flag a spike yet.
        l.record_match_load(95.0, 25.0, false);
        assert!(!l.is_workload_spike(), "single match shouldn't spike");
    }

    #[test]
    fn workload_spike_after_three_matches_in_one_week() {
        let mut l = PlayerLoad::new();
        l.daily_decay(d(2025, 1, 1));
        // Build a chronic baseline: one match a week for 4 weeks.
        for week in 0..4 {
            l.daily_decay(d(2025, 1, 1 + week * 7));
            l.record_match_load(85.0, 25.0, false);
        }
        // Now stack three matches in 7 days.
        l.daily_decay(d(2025, 2, 1));
        l.record_match_load(85.0, 25.0, false);
        l.daily_decay(d(2025, 2, 4));
        l.record_match_load(85.0, 25.0, false);
        l.daily_decay(d(2025, 2, 7));
        l.record_match_load(85.0, 25.0, false);

        let ratio = l.workload_spike_ratio();
        assert!(ratio >= WORKLOAD_SPIKE_RATIO, "ratio={}", ratio);
        assert!(l.is_workload_spike());
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

    #[test]
    fn physical_load_thresholds_independent_of_minutes() {
        let mut l = PlayerLoad::new();
        // Heavy load with no minutes (edge case but exercises the OR
        // branch in is_fatigued).
        l.physical_load_7 = PHYSICAL_LOAD_THRESHOLD + 5.0;
        assert!(l.is_fatigued());
    }
}
