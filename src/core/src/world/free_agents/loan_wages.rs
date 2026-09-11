use crate::world::SimulatorData;
use rayon::prelude::*;
use std::collections::HashMap;

/// The half of a loanee's wages the borrower never agreed to pay.
///
/// Per-club monthly finance bills the borrower for the loan contract;
/// the parent club still owes the residual share of its primary
/// contract for the duration of the loan. Settled at the world level
/// because parent and borrower may live in different countries — a
/// per-country pass cannot see them both.
pub struct LoanWageSettlement;

impl LoanWageSettlement {
    /// Residual = `(parent_salary - loan_salary).max(0) / 12`. When
    /// `loan_wage_contribution_pct` is recorded it implies the loan
    /// salary is already a percentage of the parent salary, so the
    /// residual arithmetic is correct without a separate pct path.
    /// Negative residuals are clamped to zero so we never accidentally
    /// credit the parent.
    fn owed_by_parent(data: &SimulatorData) -> HashMap<u32, i64> {
        // Read pass: the world-wide walk parallelises across countries,
        // and the per-parent merge happens serially below, so no borrow
        // is held across the credit pass.
        let entries: Vec<(u32, i64)> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map_iter(|country| {
                country.clubs.iter().flat_map(|club| {
                    club.teams.teams.iter().flat_map(|team| {
                        team.players.players.iter().filter_map(|player| {
                            let loan = player.contract_loan.as_ref()?;
                            let parent_id = loan.loan_from_club_id?;
                            let parent_annual = player.contract.as_ref()?.salary;
                            let residual_annual = parent_annual.saturating_sub(loan.salary);
                            let monthly = (residual_annual / 12) as i64;
                            (monthly > 0).then_some((parent_id, monthly))
                        })
                    })
                })
            })
            .collect();

        let mut owed: HashMap<u32, i64> = HashMap::new();
        for (parent_id, monthly) in entries {
            *owed.entry(parent_id).or_insert(0) += monthly;
        }
        owed
    }

    /// Charge each parent club its month's residual once.
    pub fn settle(data: &mut SimulatorData) {
        for (parent_id, amount) in Self::owed_by_parent(data) {
            if let Some(club) = data.club_mut(parent_id) {
                club.finance.balance.push_expense_player_wages(amount);
            }
        }
    }
}
