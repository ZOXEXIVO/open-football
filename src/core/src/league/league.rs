use crate::context::{GlobalContext, SimulationContext};
use crate::league::{
    LeagueDynamics, LeagueMilestones, LeagueRegulations,
    LeagueResult, LeagueStatistics, LeagueTable, LeagueTableRow,
    MatchStorage, Schedule, ScheduleItem,
};
use crate::{Club, Team};
use chrono::{Datelike, NaiveDate};
use log::debug;

#[derive(Debug, Clone)]
pub struct League {
    pub id: u32,
    pub name: String,
    pub slug: String,
    pub country_id: u32,
    pub schedule: Schedule,
    pub table: LeagueTable,
    pub settings: LeagueSettings,
    pub matches: MatchStorage,
    pub reputation: u16,
    pub final_table: Option<Vec<LeagueTableRow>>,
    pub dynamics: LeagueDynamics,
    pub regulations: LeagueRegulations,
    pub statistics: LeagueStatistics,
    pub milestones: LeagueMilestones,
    pub friendly: bool,
    pub is_cup: bool,
    pub financials: LeagueFinancials,
}

#[derive(Debug, Clone, Default)]
pub struct LeagueFinancials {
    pub prize_pool: i64,
    pub tv_deal_total: i64,
}

impl LeagueFinancials {
    pub fn from_reputation_and_tier(reputation: u16, tier: u8, country_reputation: u16) -> Self {
        let rep_factor = (reputation as f64 / 10000.0).clamp(0.0, 1.0);
        let country_factor = (country_reputation as f64 / 10000.0).clamp(0.0, 1.0);

        let tier_factor = match tier {
            1 => 1.0,
            2 => 0.30,
            _ => 0.10,
        };

        let base_prize = 200_000_000.0;
        let base_tv = 500_000_000.0;

        let country_market = country_factor * country_factor;
        let scale = rep_factor * rep_factor * rep_factor * tier_factor * country_market;

        LeagueFinancials {
            prize_pool: (base_prize * scale) as i64,
            tv_deal_total: (base_tv * scale) as i64,
        }
    }
}

impl League {
    pub fn new(
        id: u32,
        name: String,
        slug: String,
        country_id: u32,
        reputation: u16,
        settings: LeagueSettings,
        friendly: bool,
    ) -> Self {
        let financials = LeagueFinancials::default();

        League {
            id,
            name,
            slug,
            country_id,
            schedule: Schedule::default(),
            table: LeagueTable::default(),
            matches: MatchStorage::new(),
            settings,
            reputation,
            final_table: None,
            dynamics: LeagueDynamics::new(),
            regulations: LeagueRegulations::new(),
            statistics: LeagueStatistics::new(),
            milestones: LeagueMilestones::new(),
            friendly,
            is_cup: false,
            financials,
        }
    }

    pub fn simulate(&mut self, clubs: &[Club], ctx: GlobalContext<'_>) -> LeagueResult {
        let league_name = self.name.clone();
        let current_date = ctx.simulation.date.date();

        debug!("⚽ Simulating league: {} (Reputation: {})", league_name, self.reputation);

        self.prepare_matchday(&ctx, clubs);

        let table_result = self.table.simulate(&ctx);

        let league_teams: Vec<u32> = clubs
            .iter()
            .flat_map(|c| c.teams.with_league(self.id))
            .collect();

        let mut schedule_result = self.schedule.simulate(
            &self.settings,
            ctx.with_league(self.id, String::from(&self.slug), &league_teams, self.reputation),
        );

        let new_season_started = schedule_result.generated
            && self.table.rows.iter().any(|r| r.played > 0);

        if schedule_result.generated {
            self.table = LeagueTable::new(&league_teams);
            self.matches = MatchStorage::new();
            debug!("📊 League table reset for new season: {}", self.name);
        }

        if schedule_result.is_match_scheduled() {
            let match_results = self.play_scheduled_matches(
                &mut schedule_result.scheduled_matches,
                clubs,
                &ctx,
                self.friendly,
            );

            self.process_match_day_results(&match_results, clubs, &ctx, current_date);

            let mut result = LeagueResult::with_match_result(self.id, table_result, match_results);
            result.new_season_started = new_season_started;
            return result;
        }

        self.process_non_matchday(clubs, &ctx);

        let mut result = LeagueResult::new(self.id, table_result);
        result.new_season_started = new_season_started;
        result
    }

    #[allow(dead_code)]
    fn get_team<'c>(&self, clubs: &'c [Club], id: u32) -> &'c Team {
        clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .find(|team| team.id == id)
            .unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DayMonthPeriod {
    pub from_day: u8,
    pub from_month: u8,
    pub to_day: u8,
    pub to_month: u8,
}

impl DayMonthPeriod {
    pub fn new(from_day: u8, from_month: u8, to_day: u8, to_month: u8) -> Self {
        DayMonthPeriod { from_day, from_month, to_day, to_month }
    }
}

#[derive(Debug, Clone)]
pub struct LeagueSettings {
    pub season_starting_half: DayMonthPeriod,
    pub season_ending_half: DayMonthPeriod,
    pub tier: u8,
    pub promotion_spots: u8,
    pub relegation_spots: u8,
    pub league_group: Option<LeagueGroup>,
}

/// Identifies a league as one group within a larger competition.
#[derive(Debug, Clone)]
pub struct LeagueGroup {
    pub name: String,
    pub competition: String,
    pub total_groups: u8,
}

impl LeagueSettings {
    pub fn is_time_for_new_schedule(&self, context: &SimulationContext) -> bool {
        let season_starting_date = &self.season_starting_half;
        let date = context.date.date();
        (NaiveDate::day(&date) as u8) == season_starting_date.from_day
            && (date.month() as u8) == season_starting_date.from_month
    }
}

// Schedule extensions for enhanced functionality
impl Schedule {
    pub fn get_matches_in_next_days(&self, from_date: NaiveDate, days: i64) -> Vec<&ScheduleItem> {
        let end_date = from_date + chrono::Duration::days(days);

        self.tours.iter()
            .flat_map(|t| &t.items)
            .filter(|item| {
                let item_date = item.date.date();
                item_date >= from_date && item_date <= end_date && item.result.is_none()
            })
            .collect()
    }

    pub fn get_matches_for_team_in_days(&self, team_id: u32, from_date: NaiveDate, days: i64) -> Vec<&ScheduleItem> {
        let end_date = from_date + chrono::Duration::days(days);

        self.tours.iter()
            .flat_map(|t| &t.items)
            .filter(|item| {
                let item_date = item.date.date();
                (item.home_team_id == team_id || item.away_team_id == team_id) &&
                    item_date >= from_date &&
                    item_date <= end_date &&
                    item.result.is_none()
            })
            .collect()
    }
}
