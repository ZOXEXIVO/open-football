//! Board promises to the manager — the commitments made on appointment or
//! at a board meeting ("you'll get a striker in January", "stay up and the
//! budget grows"). Each promise has a deadline and trust consequences:
//! kept promises build manager trust, broken ones erode it. The strategy
//! component score reads outstanding/kept/broken promises.

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseType {
    TransferBudget,
    FacilityImprovement,
    YouthMinutes,
    ContinentalQualification,
    Survival,
    TitleChallenge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromiseStatus {
    #[default]
    Active,
    Kept,
    Broken,
}

#[derive(Debug, Clone)]
pub struct BoardPromise {
    pub promise_type: PromiseType,
    pub created_at: NaiveDate,
    pub due_date: NaiveDate,
    pub status: PromiseStatus,
    /// Manager-trust points awarded when the promise is kept.
    pub trust_delta_on_success: i8,
    /// Manager-trust points lost when the promise is broken.
    pub trust_delta_on_failure: i8,
}

impl BoardPromise {
    pub fn new(promise_type: PromiseType, created_at: NaiveDate, due_date: NaiveDate) -> Self {
        // Weightier promises swing trust harder.
        let (success, failure) = match promise_type {
            PromiseType::TitleChallenge => (10, -12),
            PromiseType::ContinentalQualification => (8, -9),
            PromiseType::Survival => (7, -10),
            PromiseType::TransferBudget => (5, -8),
            PromiseType::FacilityImprovement => (4, -6),
            PromiseType::YouthMinutes => (4, -5),
        };
        BoardPromise {
            promise_type,
            created_at,
            due_date,
            status: PromiseStatus::Active,
            trust_delta_on_success: success,
            trust_delta_on_failure: failure,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, PromiseStatus::Active)
    }

    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        self.is_active() && today >= self.due_date
    }
}

/// A small registry of the board's live promises. Wraps the vec so the
/// resolution / trust-bookkeeping logic lives next to the data instead of
/// leaking into the board's `simulate`.
#[derive(Debug, Clone, Default)]
pub struct PromiseLedger {
    promises: Vec<BoardPromise>,
}

impl PromiseLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, promise: BoardPromise) {
        self.promises.push(promise);
    }

    pub fn active(&self) -> impl Iterator<Item = &BoardPromise> {
        self.promises.iter().filter(|p| p.is_active())
    }

    pub fn active_count(&self) -> usize {
        self.active().count()
    }

    pub fn has_active(&self, kind: PromiseType) -> bool {
        self.promises
            .iter()
            .any(|p| p.is_active() && p.promise_type == kind)
    }

    /// Mark the first active promise of `kind` as kept; returns the trust
    /// reward if one was resolved.
    pub fn fulfil(&mut self, kind: PromiseType) -> Option<i8> {
        for p in self.promises.iter_mut() {
            if p.is_active() && p.promise_type == kind {
                p.status = PromiseStatus::Kept;
                return Some(p.trust_delta_on_success);
            }
        }
        None
    }

    /// Resolve every promise whose deadline has passed without being kept,
    /// summing the trust penalty. Returns total (negative) trust delta.
    pub fn break_overdue(&mut self, today: NaiveDate) -> i32 {
        let mut penalty = 0i32;
        for p in self.promises.iter_mut() {
            if p.is_overdue(today) {
                p.status = PromiseStatus::Broken;
                penalty += p.trust_delta_on_failure as i32;
            }
        }
        penalty
    }

    /// Net strategy-score contribution from promise track record:
    /// kept promises lift, broken ones drag. Active promises are neutral.
    pub fn track_record_score(&self) -> f32 {
        let mut score = 0.0f32;
        for p in &self.promises {
            match p.status {
                PromiseStatus::Kept => score += 4.0,
                PromiseStatus::Broken => score -= 6.0,
                PromiseStatus::Active => {}
            }
        }
        score.clamp(-30.0, 30.0)
    }

    /// Drop resolved promises older than `keep` so the ledger doesn't grow
    /// unbounded across seasons. Keeps all active promises regardless.
    pub fn prune(&mut self, today: NaiveDate, keep_days: i64) {
        self.promises
            .retain(|p| p.is_active() || (today - p.created_at).num_days() < keep_days);
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
    fn fulfilling_promise_rewards_trust() {
        let mut ledger = PromiseLedger::new();
        ledger.add(BoardPromise::new(
            PromiseType::TransferBudget,
            d(2025, 7, 1),
            d(2026, 1, 1),
        ));
        let reward = ledger.fulfil(PromiseType::TransferBudget);
        assert_eq!(reward, Some(5));
        assert_eq!(ledger.active_count(), 0);
    }

    #[test]
    fn overdue_promise_breaks_and_penalises() {
        let mut ledger = PromiseLedger::new();
        ledger.add(BoardPromise::new(
            PromiseType::Survival,
            d(2025, 7, 1),
            d(2026, 5, 1),
        ));
        let penalty = ledger.break_overdue(d(2026, 5, 2));
        assert_eq!(penalty, -10);
        assert!(ledger.track_record_score() < 0.0);
    }

    #[test]
    fn active_promise_not_broken_before_due() {
        let mut ledger = PromiseLedger::new();
        ledger.add(BoardPromise::new(
            PromiseType::Survival,
            d(2025, 7, 1),
            d(2026, 5, 1),
        ));
        assert_eq!(ledger.break_overdue(d(2026, 1, 1)), 0);
        assert_eq!(ledger.active_count(), 1);
    }
}
