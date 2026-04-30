use crate::ReputationLevel;
use crate::utils::FloatUtils;
use chrono::{Datelike, NaiveDate};

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

    /// Drop expired contracts and replace each one with a freshly negotiated
    /// deal sized by the club's reputation, market and recent performance.
    /// Returns `(expired_count, renewed_count)` for telemetry.
    pub fn renew_expired(&mut self, date: NaiveDate, ctx: &SponsorRenewalContext) -> (u32, u32) {
        let expired = self.drop_expired(date);
        let mut renewed = 0u32;
        for _ in 0..expired {
            if let Some(contract) = ctx.generate(date) {
                self.sponsorship_contracts.push(contract);
                renewed += 1;
            }
        }
        (expired, renewed)
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
    fn renew_expired_replaces_each_dropped_contract() {
        let mut s = ClubSponsorship::new(vec![
            ClubSponsorshipContract::new("Old A".to_string(), 100, d(2026, 1, 1)),
            ClubSponsorshipContract::new("Old B".to_string(), 200, d(2026, 1, 1)),
            ClubSponsorshipContract::new("Active".to_string(), 300, d(2030, 1, 1)),
        ]);

        let ctx = SponsorRenewalContext::new(
            ReputationLevel::National,
            1.0,
            SponsorPerformance::MidTable,
        );

        let (expired, renewed) = s.renew_expired(d(2026, 6, 1), &ctx);
        assert_eq!(expired, 2);
        assert_eq!(renewed, 2);
        assert_eq!(s.sponsorship_contracts.len(), 3);
    }

    #[test]
    fn renew_skips_when_no_expirations() {
        let mut s = ClubSponsorship::new(vec![ClubSponsorshipContract::new(
            "Active".to_string(),
            500,
            d(2030, 1, 1),
        )]);
        let ctx = SponsorRenewalContext::new(
            ReputationLevel::Regional,
            0.6,
            SponsorPerformance::MidTable,
        );
        let (expired, renewed) = s.renew_expired(d(2026, 6, 1), &ctx);
        assert_eq!(expired, 0);
        assert_eq!(renewed, 0);
        assert_eq!(s.sponsorship_contracts.len(), 1);
    }
}
