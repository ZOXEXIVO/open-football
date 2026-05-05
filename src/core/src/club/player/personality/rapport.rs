//! Coach ↔ Player rapport — a thing FM only implies but never shows.
//!
//! Every player tracks a rapport score with each coach who has trained
//! them recently. High rapport amplifies training gains and team-talk
//! impact; low rapport means the coach has lost the player.
//!
//! Why this is "beyond FM":
//! - Rapport is **explicit and persistent**. You can see exactly which
//!   players vibe with which coaches.
//! - It **decays without contact** — a coach that isn't actively training
//!   a player drifts back toward neutral over months.
//! - It **asymmetrically responds** to good/bad events: a bad team-talk
//!   costs more rapport than a good one earns, mirroring real dressing
//!   rooms where trust is hard to build and easy to lose.
//!
//! The struct lives on `Player` so we don't have to store an N×M matrix
//! in a central place. Typical players have 1–3 rapport entries (head
//! coach, assistant, specialist).

use chrono::NaiveDate;

/// Min rapport clamp (coach has lost the player).
pub const RAPPORT_MIN: i16 = -50;
/// Max rapport clamp (coach has the player's total trust).
pub const RAPPORT_MAX: i16 = 100;

#[derive(Debug, Clone)]
pub struct CoachRapport {
    pub coach_id: u32,
    /// Rapport score, -50 to +100. 0 = neutral new relationship.
    pub score: i16,
    /// Last day this rapport was touched — used for decay.
    pub last_touched: NaiveDate,
    /// Cumulative days trained together. Long-term pairings unlock higher
    /// effectiveness ceilings even at the same nominal score.
    pub shared_days: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerRapport {
    pub coaches: Vec<CoachRapport>,
}

impl PlayerRapport {
    pub fn new() -> Self {
        Self {
            coaches: Vec::new(),
        }
    }

    /// Return the rapport entry (creating it if missing).
    fn touch(&mut self, coach_id: u32, now: NaiveDate) -> &mut CoachRapport {
        if let Some(idx) = self.coaches.iter().position(|c| c.coach_id == coach_id) {
            &mut self.coaches[idx]
        } else {
            self.coaches.push(CoachRapport {
                coach_id,
                score: 0,
                last_touched: now,
                shared_days: 0,
            });
            self.coaches.last_mut().unwrap()
        }
    }

    /// Record one shared training day; small positive drift + shared day
    /// count. Called weekly during the training tick.
    pub fn accrue_training_day(&mut self, coach_id: u32, now: NaiveDate, days: u8) {
        let entry = self.touch(coach_id, now);
        entry.shared_days = entry.shared_days.saturating_add(days as u32);
        entry.last_touched = now;
        // Slow positive drift — maxes out around +20 without other events.
        let drift = if entry.score < 20 { 1 } else { 0 };
        entry.score = (entry.score + drift).clamp(RAPPORT_MIN, RAPPORT_MAX);
    }

    /// Good event (praise, MotM after a talk). Bounded positive movement.
    pub fn on_positive(&mut self, coach_id: u32, now: NaiveDate, amount: i16) {
        let entry = self.touch(coach_id, now);
        entry.last_touched = now;
        entry.score = (entry.score + amount).clamp(RAPPORT_MIN, RAPPORT_MAX);
    }

    /// Bad event (criticism, benching after a confidence-sapping week).
    /// Asymmetry: bad events hit 1.5× harder than good ones to mirror the
    /// "hard to build, easy to lose" rule of real dressing rooms.
    pub fn on_negative(&mut self, coach_id: u32, now: NaiveDate, amount: i16) {
        let entry = self.touch(coach_id, now);
        entry.last_touched = now;
        let hit = amount.saturating_mul(3) / 2;
        entry.score = (entry.score - hit).clamp(RAPPORT_MIN, RAPPORT_MAX);
    }

    /// Raw rapport score with a coach, or 0 when no entry exists. Used by
    /// callers that need the score itself (e.g. success-chance modifiers
    /// for manager talks) rather than a derived multiplier.
    pub fn score(&self, coach_id: u32) -> i16 {
        self.coaches
            .iter()
            .find(|c| c.coach_id == coach_id)
            .map(|c| c.score)
            .unwrap_or(0)
    }

    /// Apply monthly decay — unused coach pairings drift toward 0.
    pub fn decay(&mut self, now: NaiveDate) {
        self.coaches.retain_mut(|entry| {
            let days_since = (now - entry.last_touched).num_days();
            if days_since < 21 {
                return true;
            }
            // Drift toward 0 by 1 per month of inactivity; drop entries that
            // have returned to neutral after long inactivity.
            if entry.score > 0 {
                entry.score -= 1;
            } else if entry.score < 0 {
                entry.score += 1;
            }
            entry.score != 0 || days_since < 180
        });
    }

    /// Rapport-derived training multiplier for this coach.
    /// Neutral rapport = 1.0, strong positive = up to 1.15, broken = 0.85.
    pub fn training_multiplier(&self, coach_id: u32) -> f32 {
        let score = self
            .coaches
            .iter()
            .find(|c| c.coach_id == coach_id)
            .map(|c| c.score)
            .unwrap_or(0);
        // Map [-50..100] → [0.85..1.15], with a small long-term-pair bonus
        // after 200+ shared days.
        let base = 1.0 + (score as f32 / 100.0) * 0.15;
        let long_term = self
            .coaches
            .iter()
            .find(|c| c.coach_id == coach_id)
            .map(|c| if c.shared_days > 200 { 0.02 } else { 0.0 })
            .unwrap_or(0.0);
        (base + long_term).clamp(0.85, 1.20)
    }

    /// Team-talk delivery multiplier. Low rapport blunts praise and
    /// amplifies criticism ("why should I listen to you?").
    pub fn talk_reception_multiplier(&self, coach_id: u32, positive_tone: bool) -> f32 {
        let score = self
            .coaches
            .iter()
            .find(|c| c.coach_id == coach_id)
            .map(|c| c.score)
            .unwrap_or(0);
        if positive_tone {
            // Praise from a trusted coach lands harder
            (1.0 + (score as f32 / 100.0) * 0.4).clamp(0.6, 1.4)
        } else {
            // Criticism from a trusted coach is fair; from an untrusted
            // one, it's inflammatory.
            if score >= 0 {
                (1.0 - (score as f32 / 100.0) * 0.3).clamp(0.7, 1.0)
            } else {
                (1.0 - (score as f32 / 100.0) * 0.5).clamp(1.0, 1.5)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
    }

    #[test]
    fn praise_lands_harder_with_high_rapport() {
        let neutral = PlayerRapport::new();
        let mut trusted = PlayerRapport::new();
        trusted.on_positive(42, d(), 60);
        let neutral_mult = neutral.talk_reception_multiplier(42, true);
        let trusted_mult = trusted.talk_reception_multiplier(42, true);
        assert!(
            trusted_mult > neutral_mult,
            "trusted coach praise should land harder ({} vs {})",
            trusted_mult,
            neutral_mult
        );
    }

    #[test]
    fn criticism_hurts_more_with_low_rapport() {
        let neutral = PlayerRapport::new();
        let mut broken = PlayerRapport::new();
        broken.on_negative(42, d(), 30); // ramp lands at -45
        let neutral_mult = neutral.talk_reception_multiplier(42, false);
        let broken_mult = broken.talk_reception_multiplier(42, false);
        assert!(
            broken_mult > neutral_mult,
            "untrusted coach criticism should hurt more ({} vs {})",
            broken_mult,
            neutral_mult
        );
    }

    #[test]
    fn score_getter_returns_zero_when_absent() {
        let r = PlayerRapport::new();
        assert_eq!(r.score(99), 0);
    }
}
