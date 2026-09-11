//! Turning the board's seasonal mandate into this month's live budgets.

use crate::Club;
use crate::shared::{Currency, CurrencyValue};

impl Club {
    /// Re-derive the live transfer and wage budgets from the board's
    /// seasonal mandate and the club's current financial standing.
    ///
    /// Idempotent by construction: every figure is computed from
    /// `board.season_targets`, never by scaling last month's value. That
    /// distinction is the whole point. The previous implementation applied
    /// distress by multiplying the stored budget in place each month, so
    /// the throttle compounded away to nothing over a season and, at an
    /// insolvency factor of exactly 0.0, latched the transfer budget at
    /// zero permanently — no subsequent multiplication can recover a value
    /// from zero. Recomputing from the mandate means a club that trades its
    /// way out of trouble gets its budget back the month it does so.
    pub(in crate::club::core) fn recompute_budgets(&mut self) {
        let Some(targets) = self.board.season_targets.as_ref() else {
            return;
        };
        let mandate_transfer = targets.transfer_budget.max(0) as f64;
        let mandate_wage = targets.wage_budget.max(0) as f64;

        // Distress throttles the chest hard but never to exactly zero —
        // even a struggling club can do free-transfer and loan business,
        // and a zero budget is what froze the market world-wide.
        let (transfer_factor, wage_factor) = self.finance.distress_level.budget_factors();

        // An embargo is a state, not a permanent penalty: it lifts when the
        // club leaves emergency measures or exits administration.
        let standing = self.finance.debt.standing;
        let transfer_amount = if standing.blocks_transfer_spending() {
            0.0
        } else {
            mandate_transfer * transfer_factor
        };

        self.finance.transfer_budget = Some(CurrencyValue {
            amount: transfer_amount,
            currency: Currency::Usd,
        });
        self.finance.wage_budget = Some(CurrencyValue {
            amount: mandate_wage * wage_factor,
            currency: Currency::Usd,
        });
    }
}
