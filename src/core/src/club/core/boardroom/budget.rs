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
        // The mandate INCLUDING everything the board has moved this season.
        // Reading the raw seasonal figure meant a cut or a grant survived
        // only until the next monthly recompute rebuilt the budget from it.
        let mandate_transfer = targets.adjusted_transfer_budget() as f64;
        let mandate_wage = targets.adjusted_wage_budget() as f64;

        // Distress throttles the chest hard but never to exactly zero —
        // even a struggling club can do free-transfer and loan business,
        // and a zero budget is what froze the market world-wide. An
        // embargo is a state, not a permanent penalty: it lifts when the
        // club leaves emergency measures or exits administration.
        let (_, wage_factor) = self.finance.distress_level.budget_factors();
        // What the season has done to the money is part of the figure: a
        // fee spent in August is still spent in September, and the share
        // of a sale the club recycled is still there to spend.
        let transfer_amount = self.finance.transfer_money(mandate_transfer);

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

#[cfg(test)]
mod tests {
    use crate::academy::ClubAcademy;
    use crate::club::board::SeasonTargets;
    use crate::shared::Location;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, TeamCollection,
    };

    const MANDATE: i32 = 60_000_000;

    fn club() -> Club {
        let mut club = Club::new(
            1,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(200_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(Vec::new()),
            ClubFacilities::default(),
        );
        club.board.season_targets = Some(SeasonTargets {
            transfer_budget: MANDATE,
            ..SeasonTargets::default()
        });
        club.recompute_budgets();
        club
    }

    fn available(club: &Club) -> f64 {
        club.finance.transfer_budget.as_ref().unwrap().amount
    }

    #[test]
    fn a_fee_spent_in_august_is_still_spent_in_september() {
        let mut club = club();
        assert!(club.finance.register_transfer_purchase(25_000_000.0, 4));
        assert_eq!(available(&club), 35_000_000.0);

        club.recompute_budgets();
        assert_eq!(
            available(&club),
            35_000_000.0,
            "the monthly recompute handed the fee back"
        );
    }

    #[test]
    fn recycled_sale_proceeds_outlive_the_turn_of_the_month() {
        let mut club = club();
        club.finance.add_transfer_income_at(80_000_000.0, 0.5);
        assert_eq!(available(&club), 100_000_000.0);

        club.recompute_budgets();
        assert_eq!(
            available(&club),
            100_000_000.0,
            "the monthly recompute erased the recycled 40M"
        );
    }

    #[test]
    fn a_new_seasons_budget_is_not_moved_by_the_season_before() {
        let mut club = club();
        club.finance.add_transfer_income_at(80_000_000.0, 0.5);
        assert!(club.finance.register_transfer_purchase(70_000_000.0, 4));
        assert_eq!(available(&club), 30_000_000.0);

        // The season turns and the board sets its budget again.
        club.finance.season_fees.reset();
        club.board.season_targets = Some(SeasonTargets {
            transfer_budget: 45_000_000,
            ..SeasonTargets::default()
        });
        club.recompute_budgets();
        assert_eq!(available(&club), 45_000_000.0);
    }

    #[test]
    fn a_fee_set_aside_and_handed_back_leaves_the_figure_where_it_was() {
        let mut club = club();
        assert!(club.finance.reserve_transfer_budget(20_000_000.0));
        club.recompute_budgets();
        assert_eq!(available(&club), 40_000_000.0, "set aside across the month");

        club.finance.refund_transfer_budget(20_000_000.0);
        club.recompute_budgets();
        assert_eq!(available(&club), 60_000_000.0);
    }

    #[test]
    fn distress_throttles_what_may_be_spent_and_not_what_has_been() {
        use crate::club::finance::DistressLevel;

        let mut club = club();
        club.finance.add_transfer_income_at(40_000_000.0, 0.5);
        assert!(club.finance.register_transfer_purchase(10_000_000.0, 4));
        assert_eq!(available(&club), 70_000_000.0);

        club.finance.distress_level = DistressLevel::Distress;
        club.recompute_budgets();
        // (60 + 20) at six tenths, less the ten already spent.
        assert_eq!(available(&club), 38_000_000.0);
        club.recompute_budgets();
        assert_eq!(available(&club), 38_000_000.0, "and the same again next month");
    }
}
