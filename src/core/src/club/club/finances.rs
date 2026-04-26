use crate::context::GlobalContext;
use crate::{ContractBonusType, ReputationLevel};
use super::Club;
use chrono::Datelike;

/// Country price level: scales ticket prices, merchandising etc. by local economy.
/// England 1.5, Colombia 0.4, default 1.0.
fn get_price_level(ctx: &GlobalContext<'_>) -> f64 {
    ctx.country.as_ref()
        .map(|c| c.price_level as f64)
        .unwrap_or(1.0)
}

impl Club {
    pub(super) fn process_monthly_finances(&mut self, ctx: GlobalContext<'_>) {
        let club_name = ctx.club.as_ref().expect("no club found").name;
        let date = ctx.simulation.date.date();

        // Economic factors from country
        let tv_multiplier = ctx.country.as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let attendance_factor = ctx.country.as_ref()
            .map(|c| c.stadium_attendance_factor)
            .unwrap_or(1.0);
        let sponsorship_strength = ctx.country.as_ref()
            .map(|c| c.sponsorship_market_strength)
            .unwrap_or(1.0);

        // 1. Player wages: annual salary / 12. `Team::get_annual_salary`
        // returns *only* player wages (loan-aware: borrowers bill the
        // loan contract, not the parent contract).
        for team in self.teams.iter() {
            let annual_salary = team.get_annual_salary();
            let monthly_salary = annual_salary / 12;
            self.finance.push_salary(club_name, monthly_salary as i64);
        }

        // 1b. Lump-sum bonuses owed this month: signing bonus on freshly
        // signed contracts, loyalty bonus on each contract anniversary
        // year. Mutates the contract's `signing_bonus_paid` / per-year
        // memos so a re-run of this pass cannot double-charge.
        let bonus_payout = settle_lump_sum_bonuses(self, date);
        if bonus_payout > 0 {
            self.finance
                .balance
                .push_expense_player_wages(bonus_payout);
        }

        // 2. Staff wages: coaching, medical, scouting staff
        for team in self.teams.iter() {
            let staff_monthly = team.staffs.get_annual_salary() / 12;
            if staff_monthly > 0 {
                self.finance.balance.push_expense_staff_wages(staff_monthly as i64);
            }
        }

        // 3. Sponsorship income
        let sponsorship_income: i64 = self.finance.sponsorship
            .get_sponsorship_incomes(date)
            .iter()
            .map(|c| (c.wage / 12) as i64)
            .sum();
        if sponsorship_income > 0 {
            self.finance.balance.push_income_sponsorship(sponsorship_income);
        }

        // 4. TV, matchday, merchandising, facility costs — from main team reputation
        let main_team = self.teams.main();
        if let Some(team) = main_team {
            // TV revenue (reputation-based, scaled by country TV multiplier)
            let tv_base: i64 = match team.reputation.level() {
                ReputationLevel::Elite => 2_000_000,
                ReputationLevel::Continental => 800_000,
                ReputationLevel::National => 300_000,
                ReputationLevel::Regional => 70_000,
                ReputationLevel::Local => 20_000,
                ReputationLevel::Amateur => 5_000,
            };
            let tv_revenue = (tv_base as f64 * tv_multiplier as f64) as i64;
            self.finance.balance.push_income_tv(tv_revenue);

            // Matchday revenue (dynamic attendance × ticket price scaled by country economy)
            let price_level = get_price_level(&ctx);
            let base_attendance = self.facilities.average_attendance as f64;

            // Form + table position modifier. Reads recent stats from the
            // main team: wins over the last few games and current position.
            let (recent_wins_ratio, league_pos, total_teams) =
                self.compute_team_form_and_position(&ctx);
            let form_mult = self.facilities.dynamic_attendance_multiplier(
                recent_wins_ratio,
                league_pos,
                total_teams,
            ) as f64;

            let dynamic_attendance =
                (base_attendance * attendance_factor as f64 * form_mult) as i64;
            let ticket_base: f64 = match team.reputation.level() {
                ReputationLevel::Elite => 55.0,
                ReputationLevel::Continental => 40.0,
                ReputationLevel::National => 28.0,
                ReputationLevel::Regional => 15.0,
                ReputationLevel::Local => 8.0,
                ReputationLevel::Amateur => 4.0,
            };
            let ticket_price = (ticket_base * price_level) as i64;
            let matchday_revenue = dynamic_attendance * ticket_price * 2;
            self.finance.balance.push_income_matchday(matchday_revenue);

            // Merchandising (reputation-based, scaled by sponsorship market AND price level)
            let merch_base: f64 = match team.reputation.level() {
                ReputationLevel::Elite => 500_000.0,
                ReputationLevel::Continental => 150_000.0,
                ReputationLevel::National => 50_000.0,
                ReputationLevel::Regional => 10_000.0,
                ReputationLevel::Local => 2_000.0,
                ReputationLevel::Amateur => 500.0,
            };
            let merch_revenue = (merch_base * sponsorship_strength as f64 * price_level) as i64;
            self.finance.balance.push_income_merchandising(merch_revenue);
        }

        // 5. Facility maintenance costs
        let facility_cost: i64 = (
            self.facilities.training.to_rating() as i64 +
            self.facilities.youth.to_rating() as i64 +
            self.facilities.academy.to_rating() as i64
        ) * 5_000;
        self.finance.balance.push_expense_facilities(facility_cost);

        // 6. Operating overhead: administration, taxes, community, infrastructure
        // Scales with both balance and revenue to prevent infinite wealth accumulation.
        // Wealthy clubs have higher overhead (better facilities, more staff, legal, etc.)
        let balance = self.finance.balance.balance;
        if balance > 1_000_000 {
            // Progressive tax-like overhead: 0.3% of balance per month (~3.6% annually)
            // Plus a flat overhead based on club tier
            let balance_overhead = (balance as f64 * 0.003) as i64;
            let tier_overhead: i64 = if let Some(team) = main_team {
                match team.reputation.level() {
                    ReputationLevel::Elite => 500_000,
                    ReputationLevel::Continental => 200_000,
                    ReputationLevel::National => 80_000,
                    ReputationLevel::Regional => 30_000,
                    ReputationLevel::Local => 10_000,
                    ReputationLevel::Amateur => 3_000,
                }
            } else {
                0
            };
            self.finance.balance.push_expense_facilities(balance_overhead + tier_overhead);
        }
    }

    /// Returns (recent_wins_ratio, league_position, total_teams) for the
    /// club's main team. Form comes from the last ~5 matches in the team's
    /// `match_history`; league position rides through `ClubContext` —
    /// which the country simulation populates from the live table.
    fn compute_team_form_and_position(
        &self,
        ctx: &GlobalContext<'_>,
    ) -> (f32, u16, u16) {
        let wins_ratio = self
            .teams
            .main()
            .map(|team| team.match_history.recent_wins_ratio(5))
            .unwrap_or(0.5);

        let (position, total) = ctx
            .club
            .as_ref()
            .map(|c| (c.league_position as u16, c.league_size as u16))
            .map(|(p, t)| if p == 0 || t == 0 { (10, 20) } else { (p, t) })
            .unwrap_or((10, 20));

        (wins_ratio, position, total)
    }
}

/// Pay every lump-sum bonus owed to a player on this monthly tick. Walks
/// the club's player contracts and, for each one:
///   - SigningBonus pays once on the first finance pass after acceptance.
///     Mutates `signing_bonus_paid = true` so subsequent passes skip it.
///   - LoyaltyBonus pays once per calendar year — the contract's
///     `last_loyalty_paid_year` memo prevents same-year double pay.
///   - InternationalCapFee pays per cap gained since the last pass.
///     Tracked via `last_intl_caps_paid` so the difference is the new caps.
///
/// Returns the total expense to charge to the club this month.
fn settle_lump_sum_bonuses(club: &mut Club, date: chrono::NaiveDate) -> i64 {
    let year = date.year();
    let mut total: i64 = 0;
    for team in club.teams.teams.iter_mut() {
        for player in team.players.players.iter_mut() {
            // Cap-tracking baseline lives on the player; caps cumulative
            // count is `player.player_attributes.international_apps`.
            let current_caps = player.player_attributes.international_apps;
            let baseline_caps = player.last_intl_caps_paid;

            if let Some(contract) = player.contract.as_mut() {
                for bonus in &contract.bonuses {
                    if bonus.value <= 0 {
                        continue;
                    }
                    match bonus.bonus_type {
                        ContractBonusType::SigningBonus => {
                            if !contract.signing_bonus_paid {
                                total += bonus.value as i64;
                            }
                        }
                        ContractBonusType::LoyaltyBonus => {
                            // Pay only on or after the contract anniversary
                            // and at most once per calendar year. Year of
                            // signing pays nothing — it's the signing bonus.
                            if let Some(started) = contract.started {
                                if year > started.year()
                                    && contract.last_loyalty_paid_year != Some(year)
                                {
                                    total += bonus.value as i64;
                                }
                            }
                        }
                        ContractBonusType::InternationalCapFee => {
                            let new_caps =
                                current_caps.saturating_sub(baseline_caps) as i64;
                            if new_caps > 0 {
                                total += bonus.value as i64 * new_caps;
                            }
                        }
                        _ => {}
                    }
                }
                // Memo updates AFTER the bonus value scan so a re-entrant
                // call within the same month is a no-op.
                if !contract.signing_bonus_paid
                    && contract
                        .bonuses
                        .iter()
                        .any(|b| matches!(b.bonus_type, ContractBonusType::SigningBonus))
                {
                    contract.signing_bonus_paid = true;
                }
                if let Some(started) = contract.started {
                    if year > started.year()
                        && contract.last_loyalty_paid_year != Some(year)
                        && contract
                            .bonuses
                            .iter()
                            .any(|b| matches!(b.bonus_type, ContractBonusType::LoyaltyBonus))
                    {
                        contract.last_loyalty_paid_year = Some(year);
                    }
                }
            }
            // Update international-caps baseline on the player so the next
            // pass only counts further caps. Done outside the contract
            // borrow.
            if current_caps > baseline_caps {
                player.last_intl_caps_paid = current_caps;
            }
        }
    }
    total
}
