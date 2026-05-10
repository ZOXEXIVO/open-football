use super::SimulatorData;
use rayon::prelude::*;
use std::collections::HashMap;

/// Walk every player in the world, find loaned-in players, and bill
/// the parent club for its residual share of the primary contract that
/// the borrower's loan contract didn't cover. Runs once per calendar
/// month from `simulate_with`.
///
/// Residual = `(parent_salary - loan_salary).max(0) / 12`. When
/// `loan_wage_contribution_pct` is recorded it implies the loan salary
/// is already a percentage of the parent salary, so the residual
/// arithmetic is correct without a separate pct path. Negative
/// residuals (borrower paying more than the parent contract — should
/// not happen in practice) are clamped to zero so we never accidentally
/// credit the parent.
pub(super) fn settle_parent_residual_loan_wages(data: &mut SimulatorData) {
    // Pass 1 (read): collect (parent_club_id, monthly_residual) entries
    // so we don't hold borrows across the credit pass. The world-wide
    // walk parallelises across countries — every player is read-only
    // here, the merge into the HashMap happens serially below.
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
                        let parent_contract = player.contract.as_ref()?;
                        let parent_annual = parent_contract.salary;
                        let borrower_annual = loan.salary;
                        let residual_annual = parent_annual.saturating_sub(borrower_annual);
                        if residual_annual == 0 {
                            return None;
                        }
                        let monthly = (residual_annual / 12) as i64;
                        if monthly > 0 {
                            Some((parent_id, monthly))
                        } else {
                            None
                        }
                    })
                })
            })
        })
        .collect();

    let mut owed_by_parent: HashMap<u32, i64> = HashMap::new();
    for (parent_id, monthly) in entries {
        *owed_by_parent.entry(parent_id).or_insert(0) += monthly;
    }

    // Pass 2 (write): charge each parent club once.
    for (parent_id, amount) in owed_by_parent {
        if let Some(club) = data.club_mut(parent_id) {
            club.finance.balance.push_expense_player_wages(amount);
        }
    }
}
