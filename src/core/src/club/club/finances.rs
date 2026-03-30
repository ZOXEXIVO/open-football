use crate::context::GlobalContext;
use crate::TeamType;
use super::Club;

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

            // Matchday revenue (dynamic attendance)
            let base_attendance = self.facilities.average_attendance as f64;
            let dynamic_attendance = (base_attendance * attendance_factor as f64) as i64;
            let ticket_price: i64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 55,
                crate::ReputationLevel::Continental => 40,
                crate::ReputationLevel::National => 28,
                crate::ReputationLevel::Regional => 15,
                crate::ReputationLevel::Local => 8,
                crate::ReputationLevel::Amateur => 4,
            };
            let matchday_revenue = dynamic_attendance * ticket_price * 2;
            self.finance.balance.push_income_matchday(matchday_revenue);

            // Merchandising (reputation-based, scaled by sponsorship market)
            let merch_base: i64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 500_000,
                crate::ReputationLevel::Continental => 150_000,
                crate::ReputationLevel::National => 50_000,
                crate::ReputationLevel::Regional => 10_000,
                crate::ReputationLevel::Local => 2_000,
                crate::ReputationLevel::Amateur => 500,
            };
            let merch_revenue = (merch_base as f64 * sponsorship_strength as f64) as i64;
            self.finance.balance.push_income_merchandising(merch_revenue);
        }

        // 5. Facility maintenance costs
        let facility_cost: i64 = (
            self.facilities.training.to_rating() as i64 +
            self.facilities.youth.to_rating() as i64 +
            self.facilities.academy.to_rating() as i64
        ) * 5_000;
        self.finance.balance.push_expense_facilities(facility_cost);
    }
}
