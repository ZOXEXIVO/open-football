use crate::context::GlobalContext;
use crate::TeamType;
use super::Club;

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

        // 1. Player wages: annual salary / 12
        for team in &self.teams.teams {
            let annual_salary = team.get_annual_salary();
            let monthly_salary = annual_salary / 12;
            self.finance.push_salary(club_name, monthly_salary as i64);
        }

        // 2. Staff wages: coaching, medical, scouting staff
        for team in &self.teams.teams {
            let staff_annual: u32 = team.staffs.staffs.iter()
                .filter_map(|s| s.contract.as_ref())
                .map(|c| c.salary)
                .sum();
            let staff_monthly = staff_annual / 12;
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
        let main_team = self.teams.teams.iter().find(|t| t.team_type == TeamType::Main);
        if let Some(team) = main_team {
            // TV revenue (reputation-based, scaled by country TV multiplier)
            let tv_base: i64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 2_000_000,
                crate::ReputationLevel::Continental => 800_000,
                crate::ReputationLevel::National => 300_000,
                crate::ReputationLevel::Regional => 70_000,
                crate::ReputationLevel::Local => 20_000,
                crate::ReputationLevel::Amateur => 5_000,
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
                crate::ReputationLevel::Elite => 55.0,
                crate::ReputationLevel::Continental => 40.0,
                crate::ReputationLevel::National => 28.0,
                crate::ReputationLevel::Regional => 15.0,
                crate::ReputationLevel::Local => 8.0,
                crate::ReputationLevel::Amateur => 4.0,
            };
            let ticket_price = (ticket_base * price_level) as i64;
            let matchday_revenue = dynamic_attendance * ticket_price * 2;
            self.finance.balance.push_income_matchday(matchday_revenue);

            // Merchandising (reputation-based, scaled by sponsorship market AND price level)
            let merch_base: f64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 500_000.0,
                crate::ReputationLevel::Continental => 150_000.0,
                crate::ReputationLevel::National => 50_000.0,
                crate::ReputationLevel::Regional => 10_000.0,
                crate::ReputationLevel::Local => 2_000.0,
                crate::ReputationLevel::Amateur => 500.0,
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
                    crate::ReputationLevel::Elite => 500_000,
                    crate::ReputationLevel::Continental => 200_000,
                    crate::ReputationLevel::National => 80_000,
                    crate::ReputationLevel::Regional => 30_000,
                    crate::ReputationLevel::Local => 10_000,
                    crate::ReputationLevel::Amateur => 3_000,
                }
            } else {
                0
            };
            self.finance.balance.push_expense_facilities(balance_overhead + tier_overhead);
        }
    }

    /// Returns (recent_wins_ratio, league_position, total_teams) for the
    /// club's main team. Currently returns a neutral placeholder — the
    /// hook exists so that once team-level recent-form tracking and a
    /// league-table accessor are plumbed through `GlobalContext`, they can
    /// drop in here with zero change at the call site.
    fn compute_team_form_and_position(
        &self,
        _ctx: &GlobalContext<'_>,
    ) -> (f32, u16, u16) {
        (0.5, 10, 20)
    }
}
