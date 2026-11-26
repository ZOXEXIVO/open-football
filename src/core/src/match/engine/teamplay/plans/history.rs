//! **The plans as they stood a moment ago** — what a man who takes a new
//! job in late is still acting on.

use crate::r#match::engine::teamplay::plans::attack::AttackPlan;
use crate::r#match::engine::teamplay::plans::defence::DefensivePlan;

/// A side's last few plans, each with the tick it was made.
#[derive(Debug, Clone, Copy)]
pub struct PlanHistory {
    plans: [(u64, AttackPlan, DefensivePlan); Self::DEPTH],
    newest: usize,
}

impl PlanHistory {
    /// Enough refreshes to reach back past the longest job latency at the
    /// fastest cadence: six refreshes ten ticks apart.
    const DEPTH: usize = 6;

    pub const fn new() -> Self {
        PlanHistory {
            plans: [(0, AttackPlan::idle(), DefensivePlan::idle()); Self::DEPTH],
            newest: 0,
        }
    }

    pub fn record(&mut self, tick: u64, attack: &AttackPlan, defence: &DefensivePlan) {
        self.newest = (self.newest + 1) % Self::DEPTH;
        self.plans[self.newest] = (tick, *attack, *defence);
    }

    /// The plans as they stood at `tick`: the newest made by then, or the
    /// oldest kept.
    pub fn at(&self, tick: u64) -> (&AttackPlan, &DefensivePlan) {
        let index = (0..Self::DEPTH)
            .map(|back| (self.newest + Self::DEPTH - back) % Self::DEPTH)
            .find(|&i| self.plans[i].0 <= tick)
            .unwrap_or((self.newest + 1) % Self::DEPTH);
        let (_, attack, defence) = &self.plans[index];
        (attack, defence)
    }
}

impl Default for PlanHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::engine::adherence::Adherence;
    use crate::r#match::engine::teamplay::plans::defence::DefensiveDuty;
    use crate::r#match::engine::teamplay::plans::duties::DutyAssigner;

    const MAN: u32 = 7;

    /// The side's plan at `tick`: `MAN` holds a zone until tick 100 and
    /// covers from then on.
    fn plan_at(tick: u64) -> DefensivePlan {
        let mut defence = DefensivePlan::idle();
        defence.active = true;
        let duty = if tick >= 100 {
            DefensiveDuty::Cover
        } else {
            DefensiveDuty::HoldZone
        };
        DutyAssigner::push(&mut defence, MAN, duty);
        defence
    }

    /// The first tick at which a man with `adherence` acts on the cover,
    /// the plan refreshed every ten ticks as the match runs.
    fn takes_up_the_cover(adherence: Adherence) -> u64 {
        let mut history = PlanHistory::new();
        let latency = adherence.job_latency();
        (0..=300)
            .find(|&now| {
                if now % 10 == 0 {
                    history.record(now, &AttackPlan::idle(), &plan_at(now));
                }
                history.at(now.saturating_sub(latency)).1.duty_of(MAN) == DefensiveDuty::Cover
            })
            .expect("the cover reaches him")
    }

    #[test]
    fn a_man_who_follows_the_plan_less_takes_up_a_new_cover_later_within_the_bound() {
        let close = takes_up_the_cover(Adherence {
            place: 1.1,
            press: 1.0,
        });
        let loose = takes_up_the_cover(Adherence {
            place: 0.75,
            press: 1.0,
        });
        assert_eq!(close, 100, "at or above the population, at once");
        assert!(loose > close, "loose {loose}");
        assert!(
            loose - close <= Adherence::JOB_LATENCY + 10,
            "within the bound and one refresh: {loose}"
        );
    }

    #[test]
    fn the_history_reads_the_plan_made_by_the_moment_asked_for() {
        let mut history = PlanHistory::new();
        for tick in (60..=120).step_by(10) {
            history.record(tick, &AttackPlan::idle(), &plan_at(tick));
        }
        assert_eq!(history.at(99).1.duty_of(MAN), DefensiveDuty::HoldZone);
        assert_eq!(history.at(100).1.duty_of(MAN), DefensiveDuty::Cover);
        assert_eq!(
            history.at(0).1.duty_of(MAN),
            DefensiveDuty::HoldZone,
            "before anything kept, the oldest kept"
        );
    }
}
