use crate::Club;
use crate::club::classify_distress;
use crate::club::finance::{RevenueInputs, RevenueModel};
use crate::context::GlobalContext;
use chrono::NaiveDate;
use log::debug;

use super::debt::DebtServiceInputs;
use super::surplus::ExcessCashDeployment;

/// The local economy as the club's books see it. Every figure defaults to
/// 1.0, so a club simulated without a country behind it keeps trading at
/// world-average prices instead of earning nothing.
struct CountryEconomy {
    tv_market: f32,
    sponsorship_market: f32,
    attendance_factor: f32,
    /// Scales ticket prices and commercial income by the local economy.
    /// England 1.5, Colombia 0.4.
    price_level: f32,
}

impl CountryEconomy {
    fn of(ctx: &GlobalContext<'_>) -> Self {
        let country = ctx.country.as_ref();
        CountryEconomy {
            tv_market: country.map(|c| c.tv_revenue_multiplier).unwrap_or(1.0),
            sponsorship_market: country
                .map(|c| c.sponsorship_market_strength)
                .unwrap_or(1.0),
            attendance_factor: country.map(|c| c.stadium_attendance_factor).unwrap_or(1.0),
            price_level: country.map(|c| c.price_level).unwrap_or(1.0),
        }
    }
}

impl Club {
    pub(in crate::club::core) fn process_monthly_finances(&mut self, ctx: GlobalContext<'_>) {
        let club_name = ctx.club.as_ref().expect("no club found").name;
        let date = ctx.simulation.date.date();
        let economy = CountryEconomy::of(&ctx);

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
        let bonus_payout = self.settle_lump_sum_bonuses(date);
        if bonus_payout > 0 {
            self.finance.balance.push_expense_player_wages(bonus_payout);
        }

        // 2. Staff wages: coaching, medical, scouting staff
        for team in self.teams.iter() {
            let staff_monthly = team.staffs.get_annual_salary() / 12;
            if staff_monthly > 0 {
                self.finance
                    .balance
                    .push_expense_staff_wages(staff_monthly as i64);
            }
        }

        // 3. Sponsorship income
        let sponsorship_income: i64 = self
            .finance
            .sponsorship
            .get_sponsorship_incomes(date)
            .iter()
            .map(|c| (c.wage / 12) as i64)
            .sum();
        if sponsorship_income > 0 {
            self.finance
                .balance
                .push_income_sponsorship(sponsorship_income);
        }

        // 4. Broadcast, matchday and commercial income.
        //
        // Every line is continuous in the club's own inputs — see
        // `finance::revenue`. The old model looked each one up from a
        // six-bucket reputation ladder, so a single tier slip cut broadcast
        // income 60% and commercial income 70% overnight while the wage
        // bill, fixed by contracts already signed, didn't move at all.
        let reputation_score = self
            .teams
            .main()
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);
        let has_main_team = self.teams.main().is_some();

        if has_main_team {
            let (recent_wins_ratio, league_pos, total_teams) =
                self.compute_team_form_and_position(&ctx);
            let league_tier = ctx
                .club
                .as_ref()
                .map(|c| c.main_league_tier.max(1))
                .unwrap_or(1);

            let inputs = RevenueInputs {
                reputation_score,
                league_tier,
                league_position: league_pos,
                league_size: total_teams,
                tv_market: economy.tv_market,
                sponsorship_market: economy.sponsorship_market,
                attendance_factor: economy.attendance_factor,
                price_level: economy.price_level,
                stadium_capacity: self.facilities.capacity_or_estimate(reputation_score),
                recent_wins_ratio,
                home_matches: self.finance.take_home_match_count(),
                parachute: self.finance.parachute,
            };

            // Supporters turning up is the only part of the gate that moves
            // with results; the ground itself does not shrink.
            let form_mult = self.facilities.dynamic_attendance_multiplier(
                recent_wins_ratio,
                league_pos,
                total_teams,
            );
            let revenue = RevenueModel::monthly(&inputs, form_mult);

            if revenue.broadcast_base > 0 {
                self.finance.balance.push_income_tv(revenue.broadcast_base);
            }
            if revenue.broadcast_merit > 0 {
                self.finance
                    .balance
                    .push_income_tv_placement(revenue.broadcast_merit);
            }
            if revenue.parachute > 0 {
                self.finance
                    .balance
                    .push_income_parachute(revenue.parachute);
            }
            if revenue.matchday > 0 {
                self.finance.balance.push_income_matchday(revenue.matchday);
                // Keep the club's observed average gate honest — it feeds
                // the board's expansion case and the UI.
                self.facilities.average_attendance = revenue.attendance;
            }
            if revenue.commercial > 0 {
                self.finance
                    .balance
                    .push_income_merchandising(revenue.commercial);
            }
        }

        // 5. Amortization: each outstanding transfer purchase contributes
        // its monthly slice as a P&L expense. Cash already left the
        // balance at the upfront purchase, so this only hits `outcome` and
        // the categorised `expense_amortization` bucket.
        self.finance.tick_amortization();

        // 6. Facility maintenance costs
        let facility_cost: i64 = (self.facilities.training.to_rating() as i64
            + self.facilities.youth.to_rating() as i64
            + self.facilities.academy.to_rating() as i64)
            * 5_000;
        self.finance.balance.push_expense_facilities(facility_cost);

        // 7. Operating overhead: administration, taxes, community,
        // marketing, infrastructure. A share of revenue (the way real
        // clubs' SG&A actually scales) plus a fixed institutional floor
        // that doesn't shrink in a lean year. The floor is continuous in
        // reputation rather than a tier lookup, so a club sliding down the
        // table doesn't get an overnight cost cut it hasn't earned.
        let monthly_income = self.finance.balance.income;
        let overhead = if has_main_team {
            RevenueModel::operating_overhead(monthly_income, reputation_score)
        } else {
            0
        };
        if overhead > 0 {
            self.finance.balance.push_expense_facilities(overhead);
        }

        // 8. Debt service and resolution.
        //
        // Interest is charged only on borrowing inside the club's agreed
        // facility, at a single-digit annual rate. The old model charged
        // 0.6-1.5% *per month* on the entire negative balance and added it
        // straight back onto that balance — an uncapped compounding term
        // that drove clubs to nine-figure and then ten-figure debts with no
        // mechanism anywhere to stop or resolve it.
        let funded_months = self.finance.monthly_history_depth(date);
        // Sizing the club's borrowing facility off a trailing year it hasn't
        // lived yet would read every DB-seeded debt as a bankruptcy on the
        // first tick of a new world. The estimate annualises whatever months
        // exist; the month just billed backstops the very first tick, when
        // history is still empty.
        let trailing_income = self
            .finance
            .estimated_annual_income(date)
            .max(monthly_income.max(0).saturating_mul(12));
        let avg_wages = self.finance.trailing_avg_monthly_wages(date);
        let distress = classify_distress(self.finance.balance.balance, avg_wages);
        self.finance.distress_level = distress;

        let league_tier = ctx
            .club
            .as_ref()
            .map(|c| c.main_league_tier.max(1))
            .unwrap_or(1);
        self.resolve_debt(
            club_name,
            DebtServiceInputs {
                trailing_income,
                avg_monthly_wages: avg_wages,
                distress,
                league_tier,
                funded_months,
                date,
            },
        );

        // 9. Excess-cash deployment. Nothing else in the sim scales with
        // accumulated wealth — budgets come from trailing free cash flow,
        // wages from ability/reputation — so without this the balance is a
        // monotone accumulator and a decade of small operating surpluses
        // plus a few player sales parks nine-figure cash at a Regional
        // club. Real boards don't hoard: cash beyond a working reserve
        // goes into stadium/training programmes and owner distributions.
        // Booked as a pure cash outflow (like the upfront leg of a
        // transfer purchase): capital deployment is not an operating
        // expense, so P&L, FFP maths and budget projections stay clean.
        let deployment = ExcessCashDeployment::amount(
            self.finance.balance.balance,
            trailing_income,
            funded_months,
        );
        if deployment > 0 {
            self.finance.balance.push_cash_outflow(deployment);
            debug!(
                "club: {}, finance: excess-cash deployment of {} (balance {}, trailing income {})",
                club_name, deployment, self.finance.balance.balance, trailing_income
            );
        }
    }

    /// Returns (recent_wins_ratio, league_position, total_teams) for the
    /// club's main team. Form comes from the last ~5 matches in the team's
    /// `match_history`; league position rides through `ClubContext` —
    /// which the country simulation populates from the live table.
    fn compute_team_form_and_position(&self, ctx: &GlobalContext<'_>) -> (f32, u16, u16) {
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

    /// A year's income this club can be judged on today.
    ///
    /// The trailing estimate as soon as any month has closed; before that,
    /// [`RevenueModel::projected_annual`] on the club's own standing. A
    /// world's first tick has no finance history at all, and a zero
    /// denominator is what made every cash-positive club read as an
    /// owner-funded one (`ClubBenefactor::signal`).
    pub(in crate::club::core) fn projected_annual_income(
        &self,
        ctx: &GlobalContext<'_>,
        league_tier: u8,
        date: NaiveDate,
    ) -> i64 {
        let trailing = self.finance.estimated_annual_income(date);
        if trailing > 0 {
            return trailing;
        }
        let Some(main) = self.teams.main() else {
            return 0;
        };
        let reputation_score = main.reputation.overall_score();
        let (recent_wins_ratio, league_position, league_size) =
            self.compute_team_form_and_position(ctx);
        let economy = CountryEconomy::of(ctx);
        let inputs = RevenueInputs {
            reputation_score,
            league_tier: league_tier.max(1),
            league_position,
            league_size,
            tv_market: economy.tv_market,
            sponsorship_market: economy.sponsorship_market,
            attendance_factor: economy.attendance_factor,
            price_level: economy.price_level,
            stadium_capacity: self.facilities.capacity_or_estimate(reputation_score),
            recent_wins_ratio,
            home_matches: 0, // replaced by the projection's own cadence
            parachute: self.finance.parachute,
        };
        RevenueModel::projected_annual(&inputs)
    }
}
