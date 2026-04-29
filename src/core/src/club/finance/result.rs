use crate::club::finance::balance::DistressLevel;
use crate::club::{ClubSponsorshipContract, SponsorPerformance, SponsorRenewalContext};
use crate::simulator::SimulatorData;
use crate::ReputationLevel;
use log::debug;

pub struct ClubFinanceResult {
    pub club_id: u32,
    /// Club balance is deeply negative — emergency measures needed
    pub is_in_distress: bool,
    pub distress_level: DistressLevel,
    /// Number of sponsorship contracts that expired this month
    pub expired_sponsorships: u32,
}

impl ClubFinanceResult {
    pub fn new() -> Self {
        ClubFinanceResult {
            club_id: 0,
            is_in_distress: false,
            distress_level: DistressLevel::None,
            expired_sponsorships: 0,
        }
    }

    pub fn with_club(mut self, club_id: u32) -> Self {
        self.club_id = club_id;
        self
    }

    pub fn process(&self, data: &mut SimulatorData) {
        if self.club_id == 0 {
            return;
        }

        if self.is_in_distress {
            // Tighten budgets in proportion to distress severity. Even at
            // mild distress we cap the transfer chest; at severe/insolvency
            // we drag the wage budget too so the auto-renewal AI doesn't
            // keep handing out raises while the club is bleeding cash.
            let (transfer_factor, wage_factor) = match self.distress_level {
                DistressLevel::Distress => (0.40, 0.95),
                DistressLevel::Severe => (0.10, 0.85),
                DistressLevel::Insolvency => (0.0, 0.70),
                DistressLevel::None => (1.0, 1.0),
            };

            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };

            debug!(
                "Financial distress at {} — level={:?}, throttling budgets",
                club.name, self.distress_level
            );

            if let Some(ref mut budget) = club.finance.transfer_budget {
                budget.amount *= transfer_factor;
            }
            if let Some(ref mut budget) = club.finance.wage_budget {
                budget.amount *= wage_factor;
            }
        }

        if self.expired_sponsorships > 0 {
            // Read inputs first (immutable), then re-acquire the club
            // mutably to push the renewals — keeps `country_by_club` and
            // the mutable club borrow off each other.
            let date = data.date.date();
            let market_strength = data
                .country_by_club(self.club_id)
                .map(|c| c.economic_factors.sponsorship_market_strength)
                .unwrap_or(1.0);

            let club = match data.club(self.club_id) {
                Some(c) => c,
                None => return,
            };
            let reputation = club
                .teams
                .main()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);
            let performance = club
                .teams
                .main()
                .map(|team| {
                    let (wins, _draws, losses) = team.match_history.recent_results(8);
                    if wins >= 6 {
                        SponsorPerformance::Champion
                    } else if wins >= 4 {
                        SponsorPerformance::ContinentalQualifier
                    } else if losses >= 5 {
                        SponsorPerformance::Relegation
                    } else {
                        SponsorPerformance::MidTable
                    }
                })
                .unwrap_or(SponsorPerformance::MidTable);

            let renewal_ctx =
                SponsorRenewalContext::new(reputation, market_strength, performance);
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };
            for _ in 0..self.expired_sponsorships {
                if let Some(contract) = renewal_ctx.generate(date) {
                    club.finance.sponsorship.sponsorship_contracts.push(contract);
                }
            }

            debug!(
                "{} sponsorship(s) expired at {}, renewed at {:?} performance",
                self.expired_sponsorships, club.name, performance
            );
        }
        // Suppress unused-import warning if a feature path drops the type.
        let _ = std::marker::PhantomData::<ClubSponsorshipContract>;
    }
}
