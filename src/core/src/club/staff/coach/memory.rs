//! Persistent per-coach memory of player observations.
//!
//! Stored on [`Staff::coach_memory`] and updated at the post-match
//! dispatch layer (where the head coach for the side is known). The
//! memory is the coach's *interpretation* of a body of work — not a
//! replica of `PlayerStatistics`. The same matches feed both, but each
//! coach reads them through their own perception lens and personality,
//! so two coaches running the same player will form different memories.
//!
//! All fields default cleanly. A freshly-generated coach with no
//! observations behaves neutrally — every memory read either returns
//! `None` (no record) or a record where the EMAs are seeded to the
//! first match's rating, the streaks are zero, and the trust signals
//! sit at the neutral baseline (0.5). The selection / substitution
//! callers all have a "no memory" fall-back that defers to the existing
//! score, so old saves and freshly-built staff load without surprise.

use crate::club::staff::CoachProfile;
use chrono::NaiveDate;
use std::collections::HashMap;

/// EMA coefficient for `recent_rating_ema` — half-life ~3 matches.
/// Higher = faster reaction to the latest performance, used by
/// negative/recency-biased coaches to overreact more aggressively.
const RECENT_FORM_ALPHA: f32 = 0.50;

/// EMA coefficient for `long_form_rating` — half-life ~10 matches.
/// The "is this who he is" baseline that recency bias decays toward
/// when the coach hasn't watched the player recently.
const LONG_FORM_ALPHA: f32 = 0.15;

/// Rating below which a match counts as "poor" for streak tracking.
const POOR_RATING_THRESHOLD: f32 = 5.9;

/// Rating at or above which a match counts as "strong" for streak tracking.
const STRONG_RATING_THRESHOLD: f32 = 7.2;

/// Rating below which a single match increments the sliding low-rating count.
const LOW_RATING_BAR: f32 = 6.0;

/// Rating at or above which a single match increments the sliding high-rating count.
const HIGH_RATING_BAR: f32 = 7.0;

/// Sliding window for `recent_low_rating_count` / `recent_high_rating_count`.
/// Five matches captures the "form check" window most managers operate on.
const RECENT_WINDOW: u8 = 5;
const RECENT_MASK: u8 = (1 << RECENT_WINDOW) - 1;

/// EMA coefficient for the trust signals (tactical / big_match / training).
/// Slower than form so a single match doesn't reshape the relationship.
const TRUST_ALPHA: f32 = 0.12;

/// EMA coefficient for `professionalism_read` after the first
/// observation. Very low so the first impression is sticky — a coach
/// rarely revises their professionalism read on the basis of one match.
const PROFESSIONALISM_ALPHA: f32 = 0.04;

/// EMA coefficient for `role_fit_confidence` — moderate, so a player
/// who keeps performing out of position eventually shifts the coach's
/// confidence in their natural role.
const ROLE_FIT_ALPHA: f32 = 0.10;

/// Days of inactivity after which the coach's memory of a player
/// softens by [`INACTIVE_DECAY_PER_STEP`] per `INACTIVE_DECAY_DAYS`.
/// Old bad form does not permanently punish a player who hasn't
/// featured recently — the streaks reset and the EMAs drift back
/// toward the long_form baseline.
const INACTIVE_DECAY_DAYS: i64 = 30;
const INACTIVE_DECAY_PER_STEP: f32 = 0.25;

/// Structured flags the coach attaches to a player. Not free text — every
/// variant is a small, bounded signal a downstream decision can read by
/// name. Encoded as a u32 bit-set so the memory record stays compact.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoachMemoryFlags(u32);

impl CoachMemoryFlags {
    /// Recent costly error / red card cost the team — the coach has not
    /// yet forgotten. Decays on next clean appearance.
    pub const STICKY_DOUBT: u32 = 1 << 0;
    /// Played well in a derby / cup-final / continental knockout.
    pub const BIG_MATCH_PROVEN: u32 = 1 << 1;
    /// Failed in a big match — the coach saw it and remembers.
    pub const BIG_MATCH_FAILED: u32 = 1 << 2;
    /// Coach was forced to hook the player off early in a recent match.
    pub const EARLY_HOOK_RECENT: u32 = 1 << 3;
    /// Coach has formed a positive overall impression — survives mild dips.
    pub const TRUSTED_CORE: u32 = 1 << 4;

    pub fn contains(&self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    pub fn insert(&mut self, flag: u32) {
        self.0 |= flag;
    }

    pub fn remove(&mut self, flag: u32) {
        self.0 &= !flag;
    }

    pub fn bits(&self) -> u32 {
        self.0
    }
}

/// Per-player memory the coach builds from match observations.
///
/// All EMAs are 0.0 until the first observation; the [`MemoryEngine`]
/// seeds them on first contact. `last_observed_date` going past the
/// inactivity window softens streaks and pulls EMAs back toward the
/// long-form baseline, so a player who hasn't played for the coach in
/// months isn't carrying a stale streak forward forever.
#[derive(Debug, Clone)]
pub struct CoachMemory {
    pub player_id: u32,
    pub matches_observed: u16,
    pub recent_rating_ema: f32,
    pub long_form_rating: f32,
    pub poor_match_streak: u8,
    pub strong_match_streak: u8,
    pub recent_low_rating_count: u8,
    pub recent_high_rating_count: u8,
    pub last_observed_date: Option<NaiveDate>,
    /// One-shot rating delta vs the long-form baseline from the most
    /// recent match. Lets the coach distinguish "had a one-off bad
    /// night against expectations" from "is who we thought he was".
    pub trust_delta: f32,
    /// Coach's read of tactical reliability (0..1). Rises with role
    /// discipline / late-game composure, falls with early hooks and
    /// costly errors. Updated via EMA so it never flips on one match.
    pub tactical_trust: f32,
    /// Coach's read of big-match temperament (0..1). Only updated when
    /// the match is itself big — routine league games leave it untouched.
    pub big_match_trust: f32,
    /// Coach's impression of training contributions (0..1). Drifts toward
    /// `professionalism_read` over time when no explicit training signal.
    pub training_trust: f32,
    /// Sticky read of professionalism (0..1) — set at first observation,
    /// only revised slowly afterwards.
    pub professionalism_read: f32,
    /// Confidence in this player's role fit (0..1) — updated as the
    /// coach sees them perform in their natural slot vs an emergency one.
    pub role_fit_confidence: f32,
    pub flags: CoachMemoryFlags,
    /// Sliding 5-match bit mask: bit 0 = most recent match was poor.
    /// Used to refresh `recent_low_rating_count` without storing per-
    /// match history.
    low_window_mask: u8,
    /// Sliding 5-match bit mask: bit 0 = most recent match was strong.
    high_window_mask: u8,
}

impl CoachMemory {
    /// Has the coach seen enough matches to have a confident opinion?
    /// Below this the assessment layer falls back to softer adjustments
    /// — first impressions don't override existing scoring signal.
    pub fn is_well_observed(&self) -> bool {
        self.matches_observed >= 4
    }

    /// Coach's expected rating for the player — the long-form baseline.
    /// Used by the assessment layer to size form pressure relative to
    /// what the coach actually expected from him.
    pub fn expected_rating(&self) -> f32 {
        if self.long_form_rating > 0.0 {
            self.long_form_rating
        } else if self.recent_rating_ema > 0.0 {
            self.recent_rating_ema
        } else {
            6.7
        }
    }

    /// Form pressure in [0.0, 1.0] — the coach's *interpretation* of
    /// how badly recent ratings have fallen below the long-form
    /// baseline. Higher = more pressure to drop the player. Returns 0
    /// when the recent EMA is at or above the baseline (no pressure).
    pub fn form_pressure(&self) -> f32 {
        if !self.is_well_observed() {
            return 0.0;
        }
        let expected = self.expected_rating();
        if expected <= 0.0 || self.recent_rating_ema <= 0.0 {
            return 0.0;
        }
        let gap = (expected - self.recent_rating_ema).max(0.0);
        let streak_bump = (self.poor_match_streak.min(4) as f32) * 0.06;
        let window_bump = (self.recent_low_rating_count.min(4) as f32) * 0.05;
        (gap * 0.5 + streak_bump + window_bump).clamp(0.0, 1.0)
    }

    /// Form lift in [0.0, 1.0] — symmetric counterpart to `form_pressure`.
    /// A player on a hot streak feels the lift; the assessment layer can
    /// turn it into a small selection bonus.
    pub fn form_lift(&self) -> f32 {
        if !self.is_well_observed() {
            return 0.0;
        }
        let expected = self.expected_rating();
        if expected <= 0.0 || self.recent_rating_ema <= 0.0 {
            return 0.0;
        }
        let gap = (self.recent_rating_ema - expected).max(0.0);
        let streak_bump = (self.strong_match_streak.min(4) as f32) * 0.05;
        let window_bump = (self.recent_high_rating_count.min(4) as f32) * 0.04;
        (gap * 0.4 + streak_bump + window_bump).clamp(0.0, 1.0)
    }
}

impl Default for CoachMemory {
    fn default() -> Self {
        CoachMemory {
            player_id: 0,
            matches_observed: 0,
            recent_rating_ema: 0.0,
            long_form_rating: 0.0,
            poor_match_streak: 0,
            strong_match_streak: 0,
            recent_low_rating_count: 0,
            recent_high_rating_count: 0,
            last_observed_date: None,
            trust_delta: 0.0,
            tactical_trust: 0.5,
            big_match_trust: 0.5,
            training_trust: 0.5,
            professionalism_read: 0.5,
            role_fit_confidence: 0.5,
            flags: CoachMemoryFlags::default(),
            low_window_mask: 0,
            high_window_mask: 0,
        }
    }
}

/// Per-coach map of player memories. Lives on [`Staff`] and is
/// updated at the league/match dispatch layer where the head coach
/// for the side is known.
#[derive(Debug, Clone, Default)]
pub struct CoachMemoryStore {
    records: HashMap<u32, CoachMemory>,
}

impl CoachMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only borrow of the memory record for `player_id`, if any.
    pub fn get(&self, player_id: u32) -> Option<&CoachMemory> {
        self.records.get(&player_id)
    }

    /// Mutable borrow — only used by callers that need to clear flags
    /// after applying them (e.g. the assessment layer when a sticky
    /// doubt is resolved by a clean appearance).
    pub fn get_mut(&mut self, player_id: u32) -> Option<&mut CoachMemory> {
        self.records.get_mut(&player_id)
    }

    /// Drop a player from memory entirely. Used when the player leaves
    /// the club (transfer / release) so we don't grow the map without
    /// bound and don't risk a future stale read.
    pub fn forget(&mut self, player_id: u32) {
        self.records.remove(&player_id);
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Apply one match observation. Personality (`profile`) shapes how
    /// hard the coach reacts: a high-recency-bias coach updates the
    /// recent EMA more aggressively; a high-judging-accuracy coach
    /// trusts a single signal less.
    pub fn observe(&mut self, obs: &CoachMatchObservation, profile: &CoachProfile) {
        let record = self
            .records
            .entry(obs.player_id)
            .or_insert_with(|| CoachMemory {
                player_id: obs.player_id,
                ..CoachMemory::default()
            });
        MemoryEngine::apply(record, obs, profile);
    }

    /// Soften streak counters and pull EMAs toward the long-form
    /// baseline for any player the coach hasn't observed in
    /// [`INACTIVE_DECAY_DAYS`] days. Called from `Staff::simulate`
    /// monthly so dormant records don't punish a returning player.
    pub fn decay_inactive(&mut self, today: NaiveDate) {
        for record in self.records.values_mut() {
            MemoryEngine::decay_if_inactive(record, today);
        }
    }

    /// Iterate every memory record — diagnostics, tests, save export.
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &CoachMemory)> {
        self.records.iter()
    }
}

/// Snapshot of what happened to one player in one match. Constructed
/// at the league/match dispatch layer and fed straight to
/// [`CoachMemoryStore::observe`]. Stored on the wire-format side as
/// soon as the match completes — `Player::on_match_played` consumes
/// the rich `MatchOutcome` separately for stats / morale.
#[derive(Debug, Clone, Copy)]
pub struct CoachMatchObservation {
    pub player_id: u32,
    pub effective_rating: f32,
    pub minutes_played: u16,
    pub is_starter: bool,
    pub match_importance: f32,
    pub is_cup: bool,
    pub is_derby: bool,
    pub is_continental: bool,
    pub goals: u16,
    pub assists: u16,
    pub errors_leading_to_goal: u16,
    pub yellow_cards: u8,
    pub red_cards: u8,
    pub team_won: bool,
    /// True when the player was substituted off well before full time
    /// for non-injury reasons. Read as a tactical-trust ding.
    pub was_substituted_early: bool,
    /// Player's natural position fit when starting — drives
    /// role_fit_confidence updates. 1.0 = natural slot, < 1.0 = pressed
    /// into an emergency role.
    pub role_fit: f32,
    /// Player's overall professionalism reading at observation time —
    /// stable across matches but exposed so the coach updates their
    /// internal read slowly.
    pub professionalism_signal: f32,
    pub date: NaiveDate,
}

impl CoachMatchObservation {
    /// True when this fixture qualifies as a "big match" — derby, cup
    /// tie, or continental game. Drives whether `big_match_trust` is
    /// updated at all.
    pub fn is_big_match(&self) -> bool {
        self.is_derby || self.is_cup || self.is_continental
    }
}

/// Stateless namespace owning the per-observation memory update math.
/// Bundles the EMA / streak / flag updates together so the formulas
/// stay in one place and `CoachMemoryStore` reads as orchestration.
pub struct MemoryEngine;

impl MemoryEngine {
    /// Apply `obs` to `record` in place. Personality shapes the
    /// reaction strength.
    pub fn apply(record: &mut CoachMemory, obs: &CoachMatchObservation, profile: &CoachProfile) {
        let rating = obs.effective_rating.clamp(1.0, 10.0);

        // Personality-shaped EMA coefficients. A recency-biased coach
        // updates the recent EMA faster; a high-judging-accuracy coach
        // softens single-match swings.
        let recency_scale: f32 = (1.0 + (profile.recency_bias - 0.5) * 0.6).clamp(0.6, 1.4);
        let acc_dampen: f32 = (1.0 - profile.judging_accuracy * 0.3).clamp(0.7, 1.0);
        // Review-window boost: first impressions form fast. With few
        // observations on record (a new manager assessing his squad, or
        // a new signing being assessed), each match moves the read
        // harder; by the sixth observation the coach settles into his
        // normal update rate. Continuous taper, no window cliff.
        let early_boost: f32 = 1.0 + ((6.0 - record.matches_observed as f32) / 6.0).max(0.0) * 0.6;
        let recent_alpha =
            (RECENT_FORM_ALPHA * recency_scale * acc_dampen * early_boost).clamp(0.20, 0.85);
        let long_alpha = (LONG_FORM_ALPHA * early_boost).min(0.45);

        // Seed or merge the EMAs. First match seeds both to the rating
        // so the baseline isn't anchored to a meaningless zero.
        if record.matches_observed == 0 {
            record.recent_rating_ema = rating;
            record.long_form_rating = rating;
            record.professionalism_read = obs.professionalism_signal.clamp(0.0, 1.0);
            record.role_fit_confidence = obs.role_fit.clamp(0.0, 1.0).max(0.5);
        } else {
            record.recent_rating_ema =
                record.recent_rating_ema * (1.0 - recent_alpha) + rating * recent_alpha;
            record.long_form_rating =
                record.long_form_rating * (1.0 - long_alpha) + rating * long_alpha;
            record.professionalism_read = record.professionalism_read
                * (1.0 - PROFESSIONALISM_ALPHA)
                + obs.professionalism_signal.clamp(0.0, 1.0) * PROFESSIONALISM_ALPHA;
        }

        // Trust delta — rating vs the coach's standing expectation.
        // Scaled by personality: negativity-biased coaches feel a
        // negative delta more sharply than a positive one.
        let expectation = record.long_form_rating.max(0.0);
        let raw_delta = rating - expectation;
        let weighted_delta = if raw_delta < 0.0 {
            raw_delta * (1.0 + profile.negativity_bias * 0.5)
        } else {
            raw_delta * (1.0 - profile.negativity_bias * 0.2)
        };
        record.trust_delta = weighted_delta;

        // Streak counters.
        if rating < POOR_RATING_THRESHOLD {
            record.poor_match_streak = record.poor_match_streak.saturating_add(1);
            record.strong_match_streak = 0;
        } else if rating >= STRONG_RATING_THRESHOLD {
            record.strong_match_streak = record.strong_match_streak.saturating_add(1);
            record.poor_match_streak = 0;
        } else {
            record.poor_match_streak = record.poor_match_streak.saturating_sub(1);
            record.strong_match_streak = record.strong_match_streak.saturating_sub(1);
        }

        // Sliding-window low/high counts.
        let low_bit: u8 = if rating < LOW_RATING_BAR { 1 } else { 0 };
        let high_bit: u8 = if rating >= HIGH_RATING_BAR { 1 } else { 0 };
        record.low_window_mask = ((record.low_window_mask << 1) | low_bit) & RECENT_MASK;
        record.high_window_mask = ((record.high_window_mask << 1) | high_bit) & RECENT_MASK;
        record.recent_low_rating_count = record.low_window_mask.count_ones() as u8;
        record.recent_high_rating_count = record.high_window_mask.count_ones() as u8;

        // Trust signal updates.
        Self::update_tactical_trust(record, obs, profile);
        if obs.is_big_match() && obs.is_starter {
            Self::update_big_match_trust(record, obs, profile);
        }
        Self::update_training_trust(record, obs);
        if obs.is_starter {
            let role_target = obs.role_fit.clamp(0.0, 1.0);
            record.role_fit_confidence =
                record.role_fit_confidence * (1.0 - ROLE_FIT_ALPHA) + role_target * ROLE_FIT_ALPHA;
        }

        // Flag updates.
        Self::update_flags(record, obs, rating);

        record.matches_observed = record.matches_observed.saturating_add(1);
        record.last_observed_date = Some(obs.date);
    }

    fn update_tactical_trust(
        record: &mut CoachMemory,
        obs: &CoachMatchObservation,
        profile: &CoachProfile,
    ) {
        // Build a 0..1 tactical signal from the observation. Errors and
        // early hooks pull down; clean discipline and full-90 minutes
        // pull up. The man-management dimension softens the impact —
        // a high-man-management coach distinguishes effort from result.
        let mut signal = 0.5;
        if obs.errors_leading_to_goal > 0 {
            signal -= 0.30 * (obs.errors_leading_to_goal as f32).min(2.0) / 2.0;
        }
        if obs.red_cards > 0 {
            signal -= 0.35;
        }
        if obs.yellow_cards > 0 {
            signal -= 0.05 * obs.yellow_cards as f32;
        }
        if obs.was_substituted_early && !obs.is_starter {
            // sub coming off again — minor signal
            signal -= 0.05;
        } else if obs.was_substituted_early && obs.is_starter {
            signal -= 0.10;
        }
        if obs.is_starter && obs.minutes_played >= 80 && obs.errors_leading_to_goal == 0 {
            signal += 0.10;
        }
        if obs.effective_rating >= 7.0 {
            signal += 0.08;
        } else if obs.effective_rating < 5.7 {
            signal -= 0.10;
        }
        let signal = signal.clamp(0.0, 1.0);

        // Asymmetric dampening: a high-man-management coach softens
        // downward swings more than upward ones — they distinguish
        // "had a bad night" from "is struggling" and don't snap-judge
        // a trusted player on a single match. Upward swings still
        // flow normally so trust can grow at the regular pace.
        let drop = signal < record.tactical_trust;
        let dampener = if drop {
            (1.0 - profile.man_management * 0.6).clamp(0.35, 1.0)
        } else {
            1.0
        };
        let alpha = (TRUST_ALPHA * dampener).clamp(0.02, TRUST_ALPHA);
        record.tactical_trust =
            (record.tactical_trust * (1.0 - alpha) + signal * alpha).clamp(0.0, 1.0);
    }

    fn update_big_match_trust(
        record: &mut CoachMemory,
        obs: &CoachMatchObservation,
        profile: &CoachProfile,
    ) {
        // Big-match trust is asymmetric. A great big-match showing
        // earns a meaningful chunk; a disaster costs a similar chunk.
        // Recency-biased coaches react harder either way.
        let signal = if obs.effective_rating >= 7.5 {
            0.85
        } else if obs.effective_rating >= 7.0 {
            0.70
        } else if obs.effective_rating < 5.5 {
            0.15
        } else if obs.effective_rating < 6.0 {
            0.30
        } else {
            0.50
        };
        let recency_scale = (1.0 + (profile.recency_bias - 0.5) * 0.4).clamp(0.7, 1.3);
        let alpha = (TRUST_ALPHA * 1.5 * recency_scale).clamp(0.05, 0.25);
        record.big_match_trust =
            (record.big_match_trust * (1.0 - alpha) + signal * alpha).clamp(0.0, 1.0);
    }

    fn update_training_trust(record: &mut CoachMemory, obs: &CoachMatchObservation) {
        // Drift training_trust toward the player's professionalism
        // signal — the coach's match-day read of "this player turns
        // up" cross-checks the training-tick read.
        let target = obs.professionalism_signal.clamp(0.0, 1.0);
        record.training_trust = (record.training_trust * (1.0 - PROFESSIONALISM_ALPHA * 2.0)
            + target * PROFESSIONALISM_ALPHA * 2.0)
            .clamp(0.0, 1.0);
    }

    fn update_flags(record: &mut CoachMemory, obs: &CoachMatchObservation, rating: f32) {
        // Sticky doubt: turns on after a costly error or red card;
        // turns off after the next clean appearance >= 6.5.
        if obs.errors_leading_to_goal > 0 || obs.red_cards > 0 {
            record.flags.insert(CoachMemoryFlags::STICKY_DOUBT);
        } else if rating >= 6.5 && obs.minutes_played >= 60 {
            record.flags.remove(CoachMemoryFlags::STICKY_DOUBT);
        }

        // Big-match flags.
        if obs.is_big_match() && obs.is_starter {
            if rating >= 7.2 {
                record.flags.insert(CoachMemoryFlags::BIG_MATCH_PROVEN);
                record.flags.remove(CoachMemoryFlags::BIG_MATCH_FAILED);
            } else if rating < 5.7 {
                record.flags.insert(CoachMemoryFlags::BIG_MATCH_FAILED);
                record.flags.remove(CoachMemoryFlags::BIG_MATCH_PROVEN);
            }
        }

        // Early-hook recent: turns on for an early-pull starter, decays
        // on the next clean 70+ minute showing.
        if obs.was_substituted_early && obs.is_starter {
            record.flags.insert(CoachMemoryFlags::EARLY_HOOK_RECENT);
        } else if obs.minutes_played >= 70 && rating >= 6.5 {
            record.flags.remove(CoachMemoryFlags::EARLY_HOOK_RECENT);
        }

        // Trusted-core: large `matches_observed` AND positive long_form.
        if record.matches_observed >= 15 && record.long_form_rating >= 6.9 {
            record.flags.insert(CoachMemoryFlags::TRUSTED_CORE);
        } else if record.long_form_rating > 0.0 && record.long_form_rating < 6.3 {
            record.flags.remove(CoachMemoryFlags::TRUSTED_CORE);
        }
    }

    fn decay_if_inactive(record: &mut CoachMemory, today: NaiveDate) {
        let Some(last) = record.last_observed_date else {
            return;
        };
        let days = (today - last).num_days();
        if days < INACTIVE_DECAY_DAYS {
            return;
        }
        let steps = (days / INACTIVE_DECAY_DAYS).min(8) as f32;
        let total_decay = (INACTIVE_DECAY_PER_STEP * steps).clamp(0.0, 0.95);

        // Pull recent EMA toward long-form baseline.
        if record.long_form_rating > 0.0 {
            record.recent_rating_ema = record.recent_rating_ema * (1.0 - total_decay)
                + record.long_form_rating * total_decay;
        }
        // Decay streaks.
        let streak_decay = (total_decay * 5.0) as u8;
        record.poor_match_streak = record.poor_match_streak.saturating_sub(streak_decay);
        record.strong_match_streak = record.strong_match_streak.saturating_sub(streak_decay);
        // Decay sliding windows by shifting out one bit per step.
        let shift = steps.min(5.0) as u8;
        record.low_window_mask >>= shift;
        record.high_window_mask >>= shift;
        record.recent_low_rating_count = record.low_window_mask.count_ones() as u8;
        record.recent_high_rating_count = record.high_window_mask.count_ones() as u8;
        // Pull trust EMAs toward neutral.
        let neutral_pull = (total_decay * 0.5).clamp(0.0, 0.5);
        record.tactical_trust = record.tactical_trust * (1.0 - neutral_pull) + 0.5 * neutral_pull;
        record.big_match_trust = record.big_match_trust * (1.0 - neutral_pull) + 0.5 * neutral_pull;
        // Early-hook fades out completely after a long break.
        if days > INACTIVE_DECAY_DAYS * 2 {
            record.flags.remove(CoachMemoryFlags::EARLY_HOOK_RECENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Staff;
    use crate::club::staff::CoachingStyle;
    use crate::club::staff::StaffStub;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// Test fixture struct — bundles a Staff builder so each test can
    /// override only the attribute that matters. Builds on top of
    /// `StaffStub::default()` so the tests don't replicate every Staff
    /// field literal each time.
    struct CoachFixture;

    impl CoachFixture {
        fn baseline() -> Staff {
            let mut staff = StaffStub::default();
            staff.id = 1;
            staff.staff_attributes.knowledge.judging_player_ability = 14;
            staff.staff_attributes.knowledge.judging_player_potential = 14;
            staff.staff_attributes.mental.man_management = 12;
            staff.staff_attributes.mental.motivating = 12;
            staff.staff_attributes.mental.adaptability = 10;
            staff.staff_attributes.mental.determination = 12;
            staff.staff_attributes.mental.discipline = 10;
            staff.staff_attributes.coaching.tactical = 12;
            staff.staff_attributes.coaching.technical = 12;
            staff.staff_attributes.coaching.fitness = 12;
            staff.staff_attributes.coaching.mental = 12;
            staff.staff_attributes.coaching.working_with_youngsters = 10;
            staff.staff_attributes.coaching.attacking = 12;
            staff.staff_attributes.coaching.defending = 12;
            staff.coaching_style = CoachingStyle::Democratic;
            staff
        }

        fn high_negativity() -> Staff {
            let mut s = Self::baseline();
            s.coaching_style = CoachingStyle::Authoritarian;
            s.staff_attributes.mental.discipline = 18;
            s.staff_attributes.mental.man_management = 5;
            s
        }

        fn high_man_management() -> Staff {
            let mut s = Self::baseline();
            s.coaching_style = CoachingStyle::Transformational;
            s.staff_attributes.mental.man_management = 18;
            s.staff_attributes.mental.motivating = 18;
            s
        }
    }

    /// Test observation builder — defaults to a routine clean league
    /// start, tests override only the field they care about.
    struct ObservationFixture;

    impl ObservationFixture {
        fn league_start(player_id: u32, rating: f32, date: NaiveDate) -> CoachMatchObservation {
            CoachMatchObservation {
                player_id,
                effective_rating: rating,
                minutes_played: 90,
                is_starter: true,
                match_importance: 0.7,
                is_cup: false,
                is_derby: false,
                is_continental: false,
                goals: 0,
                assists: 0,
                errors_leading_to_goal: 0,
                yellow_cards: 0,
                red_cards: 0,
                team_won: true,
                was_substituted_early: false,
                role_fit: 1.0,
                professionalism_signal: 0.7,
                date,
            }
        }

        fn big_match(player_id: u32, rating: f32, date: NaiveDate) -> CoachMatchObservation {
            let mut o = Self::league_start(player_id, rating, date);
            o.is_cup = true;
            o.match_importance = 0.95;
            o
        }
    }

    #[test]
    fn first_observation_seeds_emas_to_rating() {
        let staff = CoachFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let mut store = CoachMemoryStore::new();
        store.observe(
            &ObservationFixture::league_start(7, 7.0, d(2026, 1, 1)),
            &profile,
        );

        let mem = store.get(7).expect("record created");
        assert_eq!(mem.matches_observed, 1);
        assert!((mem.recent_rating_ema - 7.0).abs() < 1e-4);
        assert!((mem.long_form_rating - 7.0).abs() < 1e-4);
    }

    #[test]
    fn poor_streak_counter_increments() {
        let staff = CoachFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let mut store = CoachMemoryStore::new();
        for i in 0..3 {
            store.observe(
                &ObservationFixture::league_start(7, 5.0, d(2026, 1, 1 + i)),
                &profile,
            );
        }
        let mem = store.get(7).unwrap();
        assert!(mem.poor_match_streak >= 3, "{}", mem.poor_match_streak);
        assert_eq!(mem.strong_match_streak, 0);
    }

    #[test]
    fn high_negativity_coach_drops_recent_ema_faster() {
        let baseline = CoachFixture::baseline();
        let stern = CoachFixture::high_negativity();
        let bp = CoachProfile::from_staff(&baseline);
        let sp = CoachProfile::from_staff(&stern);

        // Both observe the same run: 4 strong matches followed by 1 bad one.
        let mut baseline_store = CoachMemoryStore::new();
        let mut stern_store = CoachMemoryStore::new();
        for i in 0..4 {
            baseline_store.observe(
                &ObservationFixture::league_start(7, 7.4, d(2026, 1, 1 + i)),
                &bp,
            );
            stern_store.observe(
                &ObservationFixture::league_start(7, 7.4, d(2026, 1, 1 + i)),
                &sp,
            );
        }
        baseline_store.observe(
            &ObservationFixture::league_start(7, 4.8, d(2026, 1, 6)),
            &bp,
        );
        stern_store.observe(
            &ObservationFixture::league_start(7, 4.8, d(2026, 1, 6)),
            &sp,
        );

        let baseline_mem = baseline_store.get(7).unwrap();
        let stern_mem = stern_store.get(7).unwrap();
        // The stern coach's recent EMA should sit lower than the
        // forgiving baseline coach's after the same poor match —
        // recency × (low man-management) accelerates the swing.
        assert!(
            stern_mem.recent_rating_ema < baseline_mem.recent_rating_ema,
            "stern={} baseline={}",
            stern_mem.recent_rating_ema,
            baseline_mem.recent_rating_ema
        );
    }

    #[test]
    fn man_management_softens_tactical_trust_swing() {
        let baseline = CoachFixture::baseline();
        let warm = CoachFixture::high_man_management();
        let bp = CoachProfile::from_staff(&baseline);
        let wp = CoachProfile::from_staff(&warm);

        // Build up matching positive tactical_trust on both, then both
        // see one match with an error. The warm coach's tactical_trust
        // should drop less.
        let mut bs = CoachMemoryStore::new();
        let mut ws = CoachMemoryStore::new();
        for i in 0..6 {
            bs.observe(
                &ObservationFixture::league_start(7, 7.0, d(2026, 1, 1 + i)),
                &bp,
            );
            ws.observe(
                &ObservationFixture::league_start(7, 7.0, d(2026, 1, 1 + i)),
                &wp,
            );
        }
        let mut error_obs = ObservationFixture::league_start(7, 5.5, d(2026, 1, 8));
        error_obs.errors_leading_to_goal = 1;
        bs.observe(&error_obs, &bp);
        ws.observe(&error_obs, &wp);

        let bm = bs.get(7).unwrap();
        let wm = ws.get(7).unwrap();
        assert!(
            wm.tactical_trust >= bm.tactical_trust,
            "warm={} baseline={}",
            wm.tactical_trust,
            bm.tactical_trust
        );
    }

    #[test]
    fn big_match_proven_flag_set_after_strong_cup_showing() {
        let staff = CoachFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let mut store = CoachMemoryStore::new();
        store.observe(
            &ObservationFixture::big_match(7, 7.5, d(2026, 1, 10)),
            &profile,
        );
        let mem = store.get(7).unwrap();
        assert!(mem.flags.contains(CoachMemoryFlags::BIG_MATCH_PROVEN));
    }

    #[test]
    fn inactive_decay_softens_old_streaks() {
        let staff = CoachFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let mut store = CoachMemoryStore::new();
        // Stack three poor matches.
        for i in 0..3 {
            store.observe(
                &ObservationFixture::league_start(7, 5.0, d(2026, 1, 1 + i)),
                &profile,
            );
        }
        let pre = store.get(7).unwrap().poor_match_streak;
        assert!(pre >= 3);

        // 90 days later, with no further observations, streaks should
        // soften.
        store.decay_inactive(d(2026, 4, 5));
        let post = store.get(7).unwrap().poor_match_streak;
        assert!(post < pre, "pre={} post={}", pre, post);
    }

    #[test]
    fn well_observed_gate_holds_through_first_three_matches() {
        let staff = CoachFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let mut store = CoachMemoryStore::new();
        for i in 0..3 {
            store.observe(
                &ObservationFixture::league_start(7, 5.0, d(2026, 1, 1 + i)),
                &profile,
            );
        }
        // 3 matches → form_pressure should still be zero (the
        // assessment layer should defer to existing scoring on first
        // impressions).
        assert_eq!(store.get(7).unwrap().form_pressure(), 0.0);
        store.observe(
            &ObservationFixture::league_start(7, 5.0, d(2026, 1, 4)),
            &profile,
        );
        // 4 matches → confident now.
        assert!(store.get(7).unwrap().form_pressure() > 0.0);
    }

    #[test]
    fn forget_drops_player_from_store() {
        let staff = CoachFixture::baseline();
        let profile = CoachProfile::from_staff(&staff);
        let mut store = CoachMemoryStore::new();
        store.observe(
            &ObservationFixture::league_start(7, 6.5, d(2026, 1, 1)),
            &profile,
        );
        assert!(store.get(7).is_some());
        store.forget(7);
        assert!(store.get(7).is_none());
    }
}
