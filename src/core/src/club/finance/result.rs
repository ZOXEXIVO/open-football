use crate::ReputationLevel;
use crate::club::finance::balance::DistressLevel;
use crate::club::{ClubSponsorship, SponsorPerformance, SponsorRenewalContext};
use crate::league::result::LeagueProcessAccess;
use log::debug;

pub struct ClubFinanceResult {
    pub club_id: u32,
    /// Club balance is deeply negative — emergency measures needed
    pub is_in_distress: bool,
    pub distress_level: DistressLevel,
    /// Number of sponsorship contracts that expired this month
    pub expired_sponsorships: u32,
    /// True on the month-beginning tick — the result-stage reconciles the
    /// sponsorship book (renewals + top-up toward the portfolio target)
    /// only on this cadence.
    pub is_month_start: bool,
}

impl ClubFinanceResult {
    pub fn new() -> Self {
        ClubFinanceResult {
            club_id: 0,
            is_in_distress: false,
            distress_level: DistressLevel::None,
            expired_sponsorships: 0,
            is_month_start: false,
        }
    }

    pub fn with_club(mut self, club_id: u32) -> Self {
        self.club_id = club_id;
        self
    }

    pub fn process<D: LeagueProcessAccess>(&self, data: &mut D) {
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

        if self.is_month_start {
            // Monthly sponsorship-book reconciliation. Expired deals were
            // already dropped in the finance simulate pass; here the
            // commercial department signs replacements — and, when the
            // book sits below the reputation-tier portfolio target (a
            // freshly promoted club, or a legacy save from before clubs
            // carried a full book), lands at most one additional deal per
            // month so the ramp-up looks like business development, not a
            // windfall. A club whose reputation has fallen signs nothing
            // and the book shrinks by natural expiry toward the smaller
            // target.
            //
            // Read inputs first (immutable), then re-acquire the club
            // mutably to push the new deals — keeps the country read and
            // the mutable club borrow off each other.
            let date = data.date().date();
            let market_strength = data.sponsorship_market_strength_for(self.club_id);

            let club = match data.club(self.club_id) {
                Some(c) => c,
                None => return,
            };
            let reputation = club
                .teams
                .main()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);

            let current = club.finance.sponsorship.sponsorship_contracts.len();
            let target = ClubSponsorship::target_portfolio_size(reputation);
            let deals_to_sign =
                ClubSponsorship::deals_to_sign(current, target, self.expired_sponsorships);
            if deals_to_sign == 0 {
                return;
            }

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

            let renewal_ctx = SponsorRenewalContext::new(reputation, market_strength, performance);
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };
            for _ in 0..deals_to_sign {
                if let Some(contract) = renewal_ctx.generate(date) {
                    club.finance
                        .sponsorship
                        .sponsorship_contracts
                        .push(contract);
                }
            }

            debug!(
                "{}: signed {} sponsorship deal(s) ({} expired, book {}/{}) at {:?} performance",
                club.name,
                deals_to_sign,
                self.expired_sponsorships,
                club.finance.sponsorship.sponsorship_contracts.len(),
                target,
                performance
            );
        }
    }
}
