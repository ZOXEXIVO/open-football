//! What it costs to end a manager's deal early.
//!
//! Dismissal used to be free. The contract vanished with the man, no money
//! moved anywhere, and a board could work through four coaches in three
//! seasons without its accounts ever showing it. In the real game the
//! pay-off is the main brake on boardroom churn — it is why a club with two
//! years left on a big deal and nothing in the bank thinks twice, and why a
//! sovereign-wealth owner does not.
//!
//! Two numbers, both already on the table: what remains of the contract,
//! and how much of it this owner settles ([`OwnershipType::severance_share`]).

use chrono::NaiveDate;

/// The bill for a dismissal.
pub struct Severance;

impl Severance {
    /// Months of salary a settlement never falls below, however little of
    /// the contract is left. Nobody is dismissed for nothing.
    pub const MIN_MONTHS: f64 = 3.0;

    /// What the club owes for tearing up a deal that runs to `expires`.
    ///
    /// `share` is the owner's settlement rate. The floor applies after it:
    /// a hard-nosed owner negotiates the percentage down, not the notice
    /// period away.
    pub fn owed(annual_salary: u32, expires: NaiveDate, today: NaiveDate, share: f64) -> i64 {
        if annual_salary == 0 {
            return 0;
        }
        let salary = annual_salary as f64;
        let months_left = ((expires - today).num_days() as f64 / 30.0).max(0.0);
        let settled = salary * (months_left / 12.0) * share.clamp(0.0, 1.0);
        let floor = salary * (Self::MIN_MONTHS / 12.0);
        settled.max(floor).round() as i64
    }

    /// The same figure expressed as months of salary — what the board reads
    /// when it weighs whether it can afford to act.
    pub fn months_of_salary(annual_salary: u32, expires: NaiveDate, today: NaiveDate) -> f64 {
        if annual_salary == 0 {
            return 0.0;
        }
        ((expires - today).num_days() as f64 / 30.0)
            .max(0.0)
            .max(Self::MIN_MONTHS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, 6, 1).unwrap()
    }

    /// Two years left on a million a year, at a local owner's 60%.
    #[test]
    fn a_long_deal_torn_up_costs_most_of_what_is_left() {
        let owed = Severance::owed(1_000_000, day(2032), day(2030), 0.60);
        // ≈ 1M x 24.3/12 x 0.6
        assert!(
            (1_180_000..=1_250_000).contains(&owed),
            "two years at 60% should be about 1.2M, got {owed}"
        );
    }

    /// A sovereign owner pays the whole thing.
    #[test]
    fn a_deep_pocket_settles_in_full() {
        let full = Severance::owed(1_000_000, day(2032), day(2030), 1.00);
        let thrifty = Severance::owed(1_000_000, day(2032), day(2030), 0.60);
        assert!(full > thrifty);
        assert!((1_980_000..=2_060_000).contains(&full), "{full}");
    }

    /// A deal in its last weeks still costs the notice period — nobody is
    /// dismissed for nothing.
    #[test]
    fn a_deal_about_to_expire_still_costs_the_notice() {
        let owed = Severance::owed(1_200_000, day(2030), day(2030), 0.60);
        assert_eq!(owed, 300_000, "three months of a 1.2M salary");
    }

    /// An expired or missing contract owes nothing at all.
    #[test]
    fn nothing_is_owed_on_nothing() {
        assert_eq!(Severance::owed(0, day(2032), day(2030), 1.0), 0);
        // Already past its end: the floor still applies, because the man is
        // being told to go rather than allowed to run down.
        assert_eq!(
            Severance::owed(1_200_000, day(2029), day(2030), 0.60),
            300_000
        );
    }
}
