/// Board cash-deployment policy. A club keeps a working reserve of
/// [`Self::RESERVE_REVENUE_MULTIPLE`] × trailing annual revenue; cash
/// beyond that steadily leaves the club as infrastructure programmes and
/// owner distributions. This is the sim's only wealth-proportional flow —
/// every other expense scales with income or squad ability, so without it
/// balances only ever go up.
pub struct ExcessCashDeployment;

impl ExcessCashDeployment {
    /// Working reserve kept as a multiple of trailing annual revenue.
    /// 1.5× ≈ a season and a half of income — a healthy war chest, far
    /// below the 20×+ multiples hoarding produced.
    const RESERVE_REVENUE_MULTIPLE: f64 = 1.5;
    /// Absolute reserve floor so a tiny club is never drained toward zero.
    const RESERVE_FLOOR: i64 = 5_000_000;
    /// Fraction of the excess deployed per month (~46%/year): large piles
    /// unwind over a few seasons rather than vanishing overnight.
    const MONTHLY_RATE: f64 = 0.05;
    /// A full trailing year of completed-month snapshots is required
    /// before the policy runs, so a freshly generated world doesn't sweep
    /// DB-seeded balances before the club has any revenue evidence.
    const MIN_FUNDED_MONTHS: usize = 12;

    /// Cash to deploy this month. Zero when history is too shallow, the
    /// balance sits within the reserve, or the club is in the red.
    pub fn amount(balance: i64, trailing_annual_income: i64, funded_months: usize) -> i64 {
        if funded_months < Self::MIN_FUNDED_MONTHS {
            return 0;
        }
        let reserve = ((trailing_annual_income.max(0) as f64 * Self::RESERVE_REVENUE_MULTIPLE)
            as i64)
            .max(Self::RESERVE_FLOOR);
        let excess = balance - reserve;
        if excess <= 0 {
            return 0;
        }
        (excess as f64 * Self::MONTHLY_RATE) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::ExcessCashDeployment;

    #[test]
    fn no_deployment_without_a_full_year_of_history() {
        // A freshly generated club with a DB-seeded war chest must not be
        // swept before it has a year of revenue evidence.
        assert_eq!(ExcessCashDeployment::amount(450_000_000, 0, 0), 0);
        assert_eq!(ExcessCashDeployment::amount(450_000_000, 5_000_000, 11), 0);
    }

    #[test]
    fn balance_within_reserve_is_untouched() {
        // Reserve = 1.5 × 20M = 30M; a 25M balance sits inside it.
        assert_eq!(ExcessCashDeployment::amount(25_000_000, 20_000_000, 12), 0);
        // Negative balances obviously deploy nothing.
        assert_eq!(ExcessCashDeployment::amount(-5_000_000, 20_000_000, 12), 0);
    }

    #[test]
    fn reserve_floor_protects_tiny_clubs() {
        // Trailing income 1M → 1.5× would be 1.5M, but the 5M floor wins:
        // a 4M balance is fully protected.
        assert_eq!(ExcessCashDeployment::amount(4_000_000, 1_000_000, 12), 0);
        // 6M balance → 1M excess over the floor → 5% deployed.
        assert_eq!(
            ExcessCashDeployment::amount(6_000_000, 1_000_000, 12),
            50_000
        );
    }

    #[test]
    fn hoarded_small_club_pile_unwinds() {
        // The reported case: a Regional club sitting on 450M with ~5M/yr
        // revenue. Reserve = 7.5M, excess = 442.5M → ~22.1M leaves this
        // month; the pile converges to the reserve over a few seasons.
        let monthly = ExcessCashDeployment::amount(450_000_000, 5_000_000, 12);
        assert_eq!(monthly, 22_125_000);
    }

    #[test]
    fn healthy_elite_war_chest_is_kept() {
        // An Elite club with 250M revenue keeps up to 375M cash — only
        // genuine excess above that unwinds.
        assert_eq!(
            ExcessCashDeployment::amount(300_000_000, 250_000_000, 12),
            0
        );
        let monthly = ExcessCashDeployment::amount(500_000_000, 250_000_000, 12);
        assert_eq!(monthly, (125_000_000f64 * 0.05) as i64);
    }
}
