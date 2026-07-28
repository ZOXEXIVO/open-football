use crate::ReputationLevel;
use crate::club::finance::balance::DistressLevel;
use crate::club::news::affairs::ClubAffair;
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
            // Record the severity and let the board's seasonal recompute
            // size the budgets from revenue.
            //
            // This used to apply the throttle by *multiplying* the stored
            // budgets every month:
            //
            //     budget.amount *= transfer_factor;  // 0.0 at insolvency
            //     budget.amount *= wage_factor;      // 0.70 at insolvency
            //
            // Both bugs were fatal. The factors compounded — 0.70^12 ≈ 0.01,
            // so a season under distress erased the wage budget regardless of
            // how the club was actually trading — and a transfer factor of
            // exactly 0.0 zeroed the chest permanently, because no later
            // multiplication can lift a value off zero. Every club in the
            // world ended up showing a $0 transfer budget, which meant no
            // club could sign anyone, squads decayed, results decayed, and
            // the distress that triggered it deepened. The throttle was the
            // trap, not the cure.
            //
            // Worse, it never reduced a single actual wage: contracts kept
            // paying in full. Budget pressure now flows through
            // `Club::recompute_budgets`, which reads the distress level and
            // the debt standing and derives fresh figures from revenue.
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };

            debug!(
                "Financial distress at {} — level={:?}, budgets will be resized from revenue",
                club.name, self.distress_level
            );

            club.finance.distress_level = self.distress_level;
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
                // Deals ran out and the commercial department could not
                // replace them: the club's reputation has fallen and the
                // book is shrinking toward a smaller target. That is a
                // dated fact and the only place it is visible, so it is
                // filed here rather than inferred from a shorter list.
                if self.expired_sponsorships > 0 {
                    if let Some(club) = data.club_mut(self.club_id) {
                        club.affairs.record(
                            ClubAffair::SponsorshipLost {
                                count: self.expired_sponsorships,
                            },
                            date,
                        );
                    }
                }
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
            // The best of the month's deals is what the paper prints:
            // "signed three partners" is a line nobody reads, and the
            // one with the biggest number on it is the story.
            let mut headline_value = 0i64;
            for _ in 0..deals_to_sign {
                if let Some(contract) = renewal_ctx.generate(date) {
                    headline_value = headline_value.max(contract.wage as i64);
                    club.finance
                        .sponsorship
                        .sponsorship_contracts
                        .push(contract);
                }
            }
            if headline_value > 0 {
                club.affairs.record(
                    ClubAffair::SponsorSigned {
                        annual_value: headline_value,
                    },
                    date,
                );
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
