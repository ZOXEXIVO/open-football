use crate::ReputationLevel;
use crate::utils::FloatUtils;
use chrono::{Datelike, Duration, NaiveDate};

#[derive(Debug, Clone)]
pub struct ClubSponsorship {
    pub sponsorship_contracts: Vec<ClubSponsorshipContract>,
}

impl ClubSponsorship {
    pub fn new(contracts: Vec<ClubSponsorshipContract>) -> Self {
        ClubSponsorship {
            sponsorship_contracts: contracts,
        }
    }

    /// How many simultaneous sponsorship deals a club of this stature
    /// sustains — kit supplier + shirt sponsor + secondary partners at the
    /// top, a single local backer at the bottom. Commercial revenue is a
    /// pillar of real club finance (roughly 40% of an elite club's income),
    /// so the portfolio target is what keeps that pillar standing: the
    /// monthly finance result signs new deals until the book reaches this
    /// size and lets it shrink back by non-renewal when reputation falls.
    pub fn target_portfolio_size(reputation: ReputationLevel) -> usize {
        match reputation {
            ReputationLevel::Elite => 3,
            ReputationLevel::Continental => 3,
            ReputationLevel::National => 2,
            ReputationLevel::Regional => 2,
            ReputationLevel::Local => 1,
            ReputationLevel::Amateur => 1,
        }
    }

    fn drop_expired(&mut self, date: NaiveDate) -> u32 {
        let before = self.sponsorship_contracts.len();
        self.sponsorship_contracts
            .retain(|contract| !contract.is_expired(date));
        (before - self.sponsorship_contracts.len()) as u32
    }

    /// Drop expired contracts and report how many were removed. Used by
    /// the monthly finance pass so the result-stage can synthesise
    /// replacements at the right pace and value.
    pub fn remove_expired(&mut self, date: NaiveDate) -> u32 {
        self.drop_expired(date)
    }

    pub fn get_sponsorship_incomes<'s>(&mut self, date: NaiveDate) -> &[ClubSponsorshipContract] {
        self.drop_expired(date);

        &self.sponsorship_contracts
    }

    /// Monthly signing policy: every deal that just expired is replaced
    /// immediately (a club does not let its shirt run blank), and when the
    /// book still sits below the portfolio target the commercial
    /// department lands at most ONE additional deal — ramping a promoted
    /// club (or a legacy save from before clubs carried a full book) up
    /// over a few months instead of granting an instant windfall. A club
    /// at or above target signs nothing extra, so a shrunken reputation
    /// deflates the book by natural expiry.
    pub fn deals_to_sign(current: usize, target: usize, expired: u32) -> usize {
        target.saturating_sub(current).min(expired as usize + 1)
    }
}

/// Inputs that drive a freshly generated sponsorship contract: the club's
/// reputation tier, its country sponsorship market, and a coarse view of
/// recent on-pitch performance. Centralised so the renewal pass and the
/// initial database load can both build contracts the same way.
#[derive(Debug, Clone, Copy)]
pub struct SponsorRenewalContext {
    pub reputation: ReputationLevel,
    pub market_strength: f32,
    pub performance: SponsorPerformance,
}

#[derive(Debug, Clone, Copy)]
pub enum SponsorPerformance {
    /// Champion or top-2 finish — most lucrative renewals.
    Champion,
    /// Continental qualification.
    ContinentalQualifier,
    /// Mid-table — neutral baseline.
    MidTable,
    /// In or near the relegation zone — sponsors discount the deal.
    Relegation,
}

impl SponsorPerformance {
    pub fn multiplier(self) -> f32 {
        match self {
            SponsorPerformance::Champion => 1.25,
            SponsorPerformance::ContinentalQualifier => 1.15,
            SponsorPerformance::MidTable => 1.00,
            SponsorPerformance::Relegation => 0.75,
        }
    }
}

impl SponsorRenewalContext {
    pub fn new(
        reputation: ReputationLevel,
        market_strength: f32,
        performance: SponsorPerformance,
    ) -> Self {
        SponsorRenewalContext {
            reputation,
            market_strength,
            performance,
        }
    }

    fn annual_base(reputation: ReputationLevel) -> f64 {
        match reputation {
            ReputationLevel::Elite => 45_000_000.0,
            ReputationLevel::Continental => 14_000_000.0,
            ReputationLevel::National => 3_500_000.0,
            ReputationLevel::Regional => 750_000.0,
            ReputationLevel::Local => 120_000.0,
            ReputationLevel::Amateur => 20_000.0,
        }
    }

    /// Build a club's day-one sponsorship book: one contract per portfolio
    /// slot for the club's stature, each with a random remaining term and
    /// an extra 0-11 month stagger so the whole book doesn't come up for
    /// renewal in the same month. Used by the database generator when a
    /// world is created — the monthly renewal pass keeps the book at the
    /// target size from then on.
    pub fn generate_initial_portfolio(&self, date: NaiveDate) -> Vec<ClubSponsorshipContract> {
        let slots = ClubSponsorship::target_portfolio_size(self.reputation);
        (0..slots)
            .filter_map(|_| {
                let mut contract = self.generate(date)?;
                let stagger_days = FloatUtils::random(0.0, 330.0) as i64;
                contract.expiration += Duration::days(stagger_days);
                Some(contract)
            })
            .collect()
    }

    pub fn generate(&self, date: NaiveDate) -> Option<ClubSponsorshipContract> {
        let market = self.market_strength.max(0.05) as f64;
        let perf = self.performance.multiplier() as f64;
        let randomness = FloatUtils::random(0.85, 1.15) as f64;
        let annual = (Self::annual_base(self.reputation) * market * perf * randomness).max(0.0);
        if annual < 1.0 {
            return None;
        }

        // Duration weighted toward 2–3 years; allow 1–4.
        let roll = FloatUtils::random(0.0, 1.0);
        let years = if roll < 0.10 {
            1
        } else if roll < 0.45 {
            2
        } else if roll < 0.85 {
            3
        } else {
            4
        };
        let expiration = NaiveDate::from_ymd_opt(date.year() + years, date.month(), date.day())
            .or_else(|| NaiveDate::from_ymd_opt(date.year() + years, date.month(), 28))
            .unwrap_or(date);

        let name = generate_sponsor_name(self.reputation);
        Some(ClubSponsorshipContract::new_with_start(
            name,
            annual as i32,
            date,
            expiration,
        ))
    }
}

fn generate_sponsor_name(reputation: ReputationLevel) -> String {
    let pool: &[&str] = match reputation {
        ReputationLevel::Elite | ReputationLevel::Continental => &[
            "Atlas Capital",
            "Meridian Group",
            "Aurora Holdings",
            "Vanguard Industries",
            "Helios Bank",
        ],
        ReputationLevel::National => &[
            "Northwind Insurance",
            "Civic Energy",
            "Riverstone Logistics",
            "Beacon Telecom",
        ],
        _ => &[
            "Local Dairy Co.",
            "Westside Builders",
            "Hometown Print",
            "Greenfield Foods",
        ],
    };
    let idx = FloatUtils::random(0.0, pool.len() as f32) as usize;
    pool.get(idx).copied().unwrap_or("Sponsor").to_string()
}

#[derive(Debug, Clone)]
pub struct ClubSponsorshipContract {
    pub sponsor_name: String,
    pub wage: i32,
    pub started: Option<NaiveDate>,
    expiration: NaiveDate,
}

impl ClubSponsorshipContract {
    pub fn new(sponsor_name: String, wage: i32, expiration: NaiveDate) -> Self {
        ClubSponsorshipContract {
            sponsor_name,
            wage,
            started: None,
            expiration,
        }
    }

    pub fn new_with_start(
        sponsor_name: String,
        wage: i32,
        started: NaiveDate,
        expiration: NaiveDate,
    ) -> Self {
        ClubSponsorshipContract {
            sponsor_name,
            wage,
            started: Some(started),
            expiration,
        }
    }

    pub fn expiration(&self) -> NaiveDate {
        self.expiration
    }

    pub fn is_expired(&self, date: NaiveDate) -> bool {
        date >= self.expiration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn initial_portfolio_fills_every_slot_for_the_tier() {
        for (rep, market) in [
            (ReputationLevel::Elite, 0.9),
            (ReputationLevel::Continental, 0.7),
            (ReputationLevel::National, 0.5),
            (ReputationLevel::Regional, 0.3),
            (ReputationLevel::Local, 0.2),
        ] {
            let ctx = SponsorRenewalContext::new(rep, market, SponsorPerformance::MidTable);
            let book = ctx.generate_initial_portfolio(d(2026, 7, 1));
            assert_eq!(
                book.len(),
                ClubSponsorship::target_portfolio_size(rep),
                "tier {rep:?} must start with a full book"
            );
            for contract in &book {
                assert!(contract.wage > 0, "tier {rep:?} deal must carry value");
                assert!(
                    !contract.is_expired(d(2026, 7, 1)),
                    "seeded deal must be live on day one"
                );
            }
        }
    }

    #[test]
    fn initial_portfolio_staggers_renewal_dates() {
        // With 3 slots, random 1-4 year terms and a 0-330 day stagger, all
        // three contracts landing on the identical expiry date means the
        // stagger logic is broken (chance is negligible otherwise).
        let ctx =
            SponsorRenewalContext::new(ReputationLevel::Elite, 1.0, SponsorPerformance::MidTable);
        let book = ctx.generate_initial_portfolio(d(2026, 7, 1));
        let first = book[0].expiration();
        assert!(
            book.iter().any(|c| c.expiration() != first),
            "expirations must not all coincide"
        );
    }

    #[test]
    fn deals_to_sign_replaces_expired_and_ramps_toward_target() {
        // All expired deals are replaced at once (book at target).
        assert_eq!(ClubSponsorship::deals_to_sign(1, 3, 2), 2);
        // Below target with no expiry: at most one business-development
        // deal per month.
        assert_eq!(ClubSponsorship::deals_to_sign(0, 3, 0), 1);
        assert_eq!(ClubSponsorship::deals_to_sign(2, 3, 0), 1);
        // At or above target: nothing new — the book shrinks by expiry.
        assert_eq!(ClubSponsorship::deals_to_sign(3, 3, 0), 0);
        assert_eq!(ClubSponsorship::deals_to_sign(3, 2, 1), 0);
        // Reputation fell while a deal expired: replacement is not renewed
        // beyond the smaller target.
        assert_eq!(ClubSponsorship::deals_to_sign(1, 1, 2), 0);
    }
}
