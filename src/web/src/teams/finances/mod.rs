pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::teams::newspaper::NewspaperCounter;
use crate::teams::{TeamLookup, TeamNeighborhood};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::FormattingUtils;
use core::{Club, ClubFinances};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamFinancesGetRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/finances/index.html")]
pub struct TeamFinancesTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
    // Financial data
    pub balance: String,
    pub balance_positive: bool,
    pub transfer_budget: String,
    pub wage_budget: String,
    pub annual_wages: String,
    pub monthly_income: String,
    pub monthly_expenses: String,
    pub net_monthly: String,
    pub net_monthly_positive: bool,
    pub sponsors: Vec<SponsorDto>,
    pub history_entries: Vec<FinanceHistoryEntry>,
    // Income breakdown
    pub income_tv: String,
    pub income_tv_placement: String,
    pub income_matchday: String,
    pub income_sponsorship: String,
    pub income_merchandising: String,
    pub income_prize_money: String,
    pub income_cup_prize: String,
    pub income_continental_prize: String,
    pub income_parachute: String,
    pub income_owner_investment: String,
    pub income_loan_fees: String,
    // Expense breakdown
    pub expense_player_wages: String,
    pub expense_staff_wages: String,
    pub expense_facilities: String,
    pub expense_amortization: String,
    pub expense_debt_interest: String,
    pub expense_loan_fees: String,
    // Chart data (JSON-encoded for JS)
    pub chart_labels: String,
    pub chart_balances: String,
    pub chart_incomes: String,
    pub chart_expenses: String,
}

pub struct SponsorDto {
    pub name: String,
    pub annual_income: String,
}

pub struct FinanceHistoryEntry {
    pub month: String,
    pub balance: String,
    pub balance_positive: bool,
    pub income: String,
    pub expenses: String,
    pub net: String,
    pub net_positive: bool,
}

pub struct TeamFinancesPage;

impl TeamFinancesPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamFinancesGetRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let i18n = state.i18n.for_lang(&route_params.lang);

        let team = TeamLookup::by_slug(simulator_data, &route_params.team_slug)?;

        // Finances tab only available for senior teams that compete under their
        // own brand (Main, B, Second) — see TeamType::is_own_team.
        if !team.team_type.is_own_team() {
            return Err(ApiError::NotFound(
                "Finances not available for this team type".to_string(),
            ));
        }

        let club = simulator_data.club(team.club_id).ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", team.club_id))
        })?;

        Self::template(simulator_data, team, club, i18n, &route_params)
    }

    fn template(
        data: &core::SimulatorData,
        team: &core::Team,
        club: &Club,
        i18n: I18n,
        route_params: &TeamFinancesGetRequest,
    ) -> Result<TeamFinancesTemplate, ApiError> {
        let snapshot = FinanceSnapshot::of(club, &i18n);
        let league = team.league_id.and_then(|id| data.league(id));
        let current_path = format!("/{}/teams/{}/finances", route_params.lang, team.slug);
        let neighborhood = TeamNeighborhood::for_club(team.club_id, data, &i18n)?;
        let menu_sections = neighborhood.menu_sections(
            data,
            team.club_id,
            &i18n,
            &route_params.lang,
            &current_path,
        );
        let title = team.name.clone();
        let league_title = league
            .map(|l| views::Menus::league_display_name(l, &i18n, data))
            .unwrap_or_default();
        let figures = snapshot.figures;

        Ok(TeamFinancesTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            i18n,
            lang: route_params.lang.clone(),
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: league_title,
            sub_title_link: league
                .map(|l| LeagueAddress::of(data, l).url(&route_params.lang))
                .unwrap_or_default(),
            sub_title_flag: None,
            header_color: club.colors.background.clone(),
            foreground_color: club.colors.foreground.clone(),
            menu_sections,
            team_slug: team.slug.clone(),
            active_tab: "finances",
            show_finances_tab: true,
            show_academy_tab: team.team_type == core::TeamType::Main
                || team.team_type == core::TeamType::U18,
            newspaper_count: NewspaperCounter::count(data, team),
            balance: snapshot.balance,
            balance_positive: snapshot.balance_positive,
            transfer_budget: snapshot.transfer_budget,
            wage_budget: snapshot.wage_budget,
            annual_wages: snapshot.annual_wages,
            monthly_income: figures.monthly_income,
            monthly_expenses: figures.monthly_expenses,
            net_monthly: figures.net_monthly,
            net_monthly_positive: figures.net_monthly_positive,
            sponsors: snapshot.sponsors,
            history_entries: snapshot.history.entries,
            income_tv: figures.income_tv,
            income_tv_placement: figures.income_tv_placement,
            income_matchday: figures.income_matchday,
            income_sponsorship: figures.income_sponsorship,
            income_merchandising: figures.income_merchandising,
            income_prize_money: figures.income_prize_money,
            income_cup_prize: figures.income_cup_prize,
            income_continental_prize: figures.income_continental_prize,
            income_parachute: figures.income_parachute,
            income_owner_investment: figures.income_owner_investment,
            income_loan_fees: figures.income_loan_fees,
            expense_player_wages: figures.expense_player_wages,
            expense_staff_wages: figures.expense_staff_wages,
            expense_facilities: figures.expense_facilities,
            expense_amortization: figures.expense_amortization,
            expense_debt_interest: figures.expense_debt_interest,
            expense_loan_fees: figures.expense_loan_fees,
            chart_labels: snapshot.history.labels,
            chart_balances: snapshot.history.balances,
            chart_incomes: snapshot.history.incomes,
            chart_expenses: snapshot.history.expenses,
        })
    }
}

struct FinanceSnapshot {
    balance: String,
    balance_positive: bool,
    transfer_budget: String,
    wage_budget: String,
    annual_wages: String,
    figures: MonthlyFigures,
    sponsors: Vec<SponsorDto>,
    history: FinanceHistoryView,
}

impl FinanceSnapshot {
    fn of(club: &Club, i18n: &I18n) -> Self {
        let finance = &club.finance;

        // Current balance
        let balance_positive = finance.balance.balance >= 0;
        let balance = Money::compact(finance.balance.balance);

        // Budgets
        let transfer_budget = finance
            .transfer_budget
            .as_ref()
            .map(|b| FormattingUtils::format_money(b.amount))
            .unwrap_or_else(|| i18n.t("fin_not_set").to_string());

        let wage_budget = finance
            .wage_budget
            .as_ref()
            .map(|b| FormattingUtils::format_money(b.amount))
            .unwrap_or_else(|| i18n.t("fin_not_set").to_string());

        // Annual wages (sum across all teams in the club)
        let total_annual_wages: u32 = club.teams.teams.iter().map(|t| t.get_annual_salary()).sum();
        let annual_wages = Money::compact(total_annual_wages as i64);

        let latest = Self::latest_balance(finance);
        let figures = MonthlyFigures::of(latest);

        Self {
            balance,
            balance_positive,
            transfer_budget,
            wage_budget,
            annual_wages,
            figures,
            sponsors: Self::sponsors(club),
            history: FinanceHistoryView::of(finance, i18n),
        }
    }

    fn latest_balance(finance: &ClubFinances) -> &core::ClubFinancialBalance {
        // The latest completed month, not the in-progress one.
        finance
            .history
            .iter()
            .next()
            .map(|(_, bal)| bal)
            .unwrap_or(&finance.balance)
    }

    fn sponsors(club: &Club) -> Vec<SponsorDto> {
        club.finance
            .sponsorship
            .sponsorship_contracts
            .iter()
            .map(|c| SponsorDto {
                name: c.sponsor_name.clone(),
                annual_income: Money::compact(c.wage as i64),
            })
            .collect()
    }
}

struct MonthlyFigures {
    monthly_income: String,
    monthly_expenses: String,
    net_monthly: String,
    net_monthly_positive: bool,
    income_tv: String,
    income_tv_placement: String,
    income_matchday: String,
    income_sponsorship: String,
    income_merchandising: String,
    income_prize_money: String,
    income_cup_prize: String,
    income_continental_prize: String,
    income_parachute: String,
    income_owner_investment: String,
    income_loan_fees: String,
    expense_player_wages: String,
    expense_staff_wages: String,
    expense_facilities: String,
    expense_amortization: String,
    expense_debt_interest: String,
    expense_loan_fees: String,
}

impl MonthlyFigures {
    fn of(latest: &core::ClubFinancialBalance) -> Self {
        let monthly_income_val = latest.income;
        let monthly_expenses_val = latest.outcome;
        let net = monthly_income_val - monthly_expenses_val;

        // Generic prize_money already includes cup + continental, so subtract
        // them here to avoid double-counting in the UI.
        let other_prize =
            (latest.income_prize_money - latest.income_cup_prize - latest.income_continental_prize)
                .max(0);

        Self {
            monthly_income: Money::compact(monthly_income_val),
            monthly_expenses: Money::compact(monthly_expenses_val),
            net_monthly: Money::compact(net),
            net_monthly_positive: net >= 0,
            income_tv: Money::compact(latest.income_tv),
            income_tv_placement: Money::compact(latest.income_tv_placement),
            income_matchday: Money::compact(latest.income_matchday),
            income_sponsorship: Money::compact(latest.income_sponsorship),
            income_merchandising: Money::compact(latest.income_merchandising),
            income_prize_money: Money::compact(other_prize),
            income_cup_prize: Money::compact(latest.income_cup_prize),
            income_continental_prize: Money::compact(latest.income_continental_prize),
            income_parachute: Money::compact(latest.income_parachute),
            income_owner_investment: Money::compact(latest.income_owner_investment),
            income_loan_fees: Money::compact(latest.income_loan_fees),
            expense_player_wages: Money::compact(latest.expense_player_wages),
            expense_staff_wages: Money::compact(latest.expense_staff_wages),
            expense_facilities: Money::compact(latest.expense_facilities),
            expense_amortization: Money::compact(latest.expense_amortization),
            expense_debt_interest: Money::compact(latest.expense_debt_interest),
            expense_loan_fees: Money::compact(latest.expense_loan_fees),
        }
    }
}

struct FinanceHistoryView {
    entries: Vec<FinanceHistoryEntry>,
    labels: String,
    balances: String,
    incomes: String,
    expenses: String,
}

impl FinanceHistoryView {
    fn of(finance: &ClubFinances, i18n: &I18n) -> Self {
        // History (most recent first, take last 12 months)
        let history_items: Vec<_> = finance.history.iter().take(12).collect();

        let mut chart_labels: Vec<String> = Vec::new();
        let mut chart_balances: Vec<i64> = Vec::new();
        let mut chart_incomes: Vec<i64> = Vec::new();
        let mut chart_expenses: Vec<i64> = Vec::new();

        let entries: Vec<FinanceHistoryEntry> = history_items
            .iter()
            .map(|(date, bal)| {
                let month_str = format!("{}/{}", date.format("%m"), date.format("%y"));
                chart_labels.push(month_str.clone());
                chart_balances.push(bal.balance);
                chart_incomes.push(bal.income);
                chart_expenses.push(bal.outcome);

                let net_val = bal.income - bal.outcome;
                FinanceHistoryEntry {
                    month: i18n.format_month_year(*date),
                    balance: Money::compact(bal.balance),
                    balance_positive: bal.balance >= 0,
                    income: Money::compact(bal.income),
                    expenses: Money::compact(bal.outcome),
                    net: Money::compact(net_val),
                    net_positive: net_val >= 0,
                }
            })
            .collect();

        // Reverse chart data so it goes oldest -> newest for the chart
        chart_labels.reverse();
        chart_balances.reverse();
        chart_incomes.reverse();
        chart_expenses.reverse();

        Self {
            entries,
            labels: serde_json::to_string(&chart_labels).unwrap_or_default(),
            balances: serde_json::to_string(&chart_balances).unwrap_or_default(),
            incomes: serde_json::to_string(&chart_incomes).unwrap_or_default(),
            expenses: serde_json::to_string(&chart_expenses).unwrap_or_default(),
        }
    }
}

struct Money;

impl Money {
    fn compact(amount: i64) -> String {
        if amount.abs() >= 1_000_000 {
            format!("${:.1}M", amount as f64 / 1_000_000.0)
        } else if amount.abs() >= 1_000 {
            format!("${:.0}K", amount as f64 / 1_000.0)
        } else {
            format!("${}", amount)
        }
    }
}
