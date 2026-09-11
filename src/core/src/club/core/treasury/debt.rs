use crate::Club;
use crate::club::DistressLevel;
use crate::club::finance::{AdministrationState, DebtProfile, DebtStanding};
use crate::club::news::affairs::ClubAffair;
use chrono::NaiveDate;
use log::debug;

/// What the club's books look like to its lenders this month.
pub(in crate::club::core) struct DebtServiceInputs {
    pub trailing_income: i64,
    pub avg_monthly_wages: i64,
    pub distress: DistressLevel,
    pub league_tier: u8,
    /// Completed-month snapshots the club actually has. Administration is a
    /// judgement about a trading record, so a world that has only just been
    /// generated has none to judge.
    pub funded_months: usize,
    pub date: NaiveDate,
}

impl Club {
    /// Service and, where necessary, resolve the club's debt.
    ///
    /// Walks the escalation ladder a real club meets: ordinary borrowing
    /// inside a facility, then owner funding, then emergency trading, then
    /// administration. The last rung is what guarantees termination — it
    /// writes the unpayable balance down to something the club can service
    /// in exchange for a points deduction and a year of embargo. Without a
    /// terminal state the balance is a pure divergent series.
    ///
    /// Three rungs of the ladder are also *dated events* rather than
    /// states, so each is filed in the club's diary at the line that
    /// performs it — see [`crate::club::news::affairs`]. The standing it
    /// leaves behind (a debt, an embargo) the press can read off the club
    /// whenever it likes; the day it changed, it cannot.
    pub(in crate::club::core) fn resolve_debt(
        &mut self,
        club_name: &str,
        inputs: DebtServiceInputs,
    ) {
        let DebtServiceInputs {
            trailing_income,
            avg_monthly_wages,
            distress,
            league_tier,
            funded_months,
            date,
        } = inputs;
        let administration = self.finance.debt.administration;
        let balance = self.finance.balance.balance;
        let standing = DebtProfile::classify(balance, trailing_income, distress, administration);
        self.finance.debt.standing = standing;

        // Interest on serviced borrowing only.
        let interest = self.finance.debt.monthly_interest(balance, trailing_income);
        if interest > 0 {
            self.finance.balance.push_expense_debt_interest(interest);
        }

        // The owner covers what he's willing to of the shortfall past the
        // facility. Booked as funding, not revenue — it must not flatter
        // the P&L or inflate next season's revenue-derived budgets.
        let appetite = self.board.ownership.injection_appetite();
        let injection = DebtProfile::owner_injection(
            self.finance.balance.balance,
            trailing_income,
            appetite,
            standing,
        );
        if injection > 0 {
            self.finance.balance.push_owner_investment(injection);
            self.affairs
                .record(ClubAffair::OwnerBailout { amount: injection }, date);
            debug!(
                "club: {}, finance: owner injected {} to cover a shortfall",
                club_name, injection
            );
        }

        // Tick down an administration already in force, or enter one.
        if let Some(mut state) = self.finance.debt.administration {
            if state.tick() {
                self.finance.debt.administration = None;
                self.finance.debt.standing = DebtProfile::classify(
                    self.finance.balance.balance,
                    trailing_income,
                    distress,
                    None,
                );
                self.affairs.record(ClubAffair::AdministrationExited, date);
                debug!("club: {}, finance: exited administration", club_name);
            } else {
                self.finance.debt.administration = Some(state);
            }
            return;
        }

        // Administration is a judgement about a club's trading record, so
        // it needs a real trading record. A world that has just been
        // generated must never put its indebted clubs straight into
        // administration on the strength of a seeded opening balance.
        const MIN_MONTHS_BEFORE_ADMINISTRATION: usize = 12;
        if funded_months < MIN_MONTHS_BEFORE_ADMINISTRATION {
            return;
        }

        let trailing_wages = avg_monthly_wages.saturating_mul(12);
        if DebtProfile::should_enter_administration(
            self.finance.balance.balance,
            trailing_income,
            trailing_wages,
            standing,
        ) {
            let written = DebtProfile::administration_write_down(
                self.finance.balance.balance,
                trailing_income,
            );
            if written > 0 {
                self.finance.balance.push_debt_write_down(written);
            }
            let state = AdministrationState::enter(league_tier);
            debug!(
                "club: {}, finance: entered administration — {} points deducted, {} written down",
                club_name, state.points_deduction, written
            );
            self.affairs.record(
                ClubAffair::AdministrationEntered {
                    points_deduction: state.points_deduction,
                },
                date,
            );
            self.finance.debt.administration = Some(state);
            self.finance.debt.standing = DebtStanding::Administration;
        }
    }
}
