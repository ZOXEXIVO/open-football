pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::FormattingUtils;
use core::SimulatorData;
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
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
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
    pub income_matchday: String,
    pub income_sponsorship: String,
    pub income_merchandising: String,
    pub income_prize_money: String,
    // Expense breakdown
    pub expense_player_wages: String,
    pub expense_staff_wages: String,
    pub expense_facilities: String,
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

pub async fn team_finances_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamFinancesGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let team_id = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    // Finances tab only available for Main and B teams
    if team.team_type != core::TeamType::Main && team.team_type != core::TeamType::B {
        return Err(ApiError::NotFound("Finances not available for this team type".to_string()));
    }

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let club = simulator_data
        .club(team.club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", team.club_id)))?;

    let finance = &club.finance;

    // Current balance
    let balance_positive = finance.balance.balance >= 0;
    let balance = format_currency(finance.balance.balance as i64);

    // Budgets
    let transfer_budget = finance.transfer_budget.as_ref()
        .map(|b| FormattingUtils::format_money(b.amount))
        .unwrap_or_else(|| i18n.t("fin_not_set").to_string());

    let wage_budget = finance.wage_budget.as_ref()
        .map(|b| FormattingUtils::format_money(b.amount))
        .unwrap_or_else(|| i18n.t("fin_not_set").to_string());

    // Annual wages (sum across all teams in the club)
    let total_annual_wages: u32 = club.teams.teams.iter()
        .map(|t| t.get_annual_salary())
        .sum();
    let annual_wages = format_currency(total_annual_wages as i64);

    // Monthly income/expenses (use latest completed month from history, not in-progress month)
    let latest_bal = finance.history.iter().next()
        .map(|(_, bal)| bal)
        .unwrap_or(&finance.balance);
    let monthly_income_val = latest_bal.income;
    let monthly_expenses_val = latest_bal.outcome;
    let monthly_income = format_currency(monthly_income_val);
    let monthly_expenses = format_currency(monthly_expenses_val);
    let net = monthly_income_val - monthly_expenses_val;
    let net_monthly_positive = net >= 0;
    let net_monthly = format_currency(net);

    // Income/expense category breakdown
    let income_tv = format_currency(latest_bal.income_tv);
    let income_matchday = format_currency(latest_bal.income_matchday);
    let income_sponsorship = format_currency(latest_bal.income_sponsorship);
    let income_merchandising = format_currency(latest_bal.income_merchandising);
    let income_prize_money = format_currency(latest_bal.income_prize_money);
    let expense_player_wages = format_currency(latest_bal.expense_player_wages);
    let expense_staff_wages = format_currency(latest_bal.expense_staff_wages);
    let expense_facilities = format_currency(latest_bal.expense_facilities);

    // Sponsorship
    let sponsors: Vec<SponsorDto> = club.finance.sponsorship.sponsorship_contracts.iter()
        .map(|c| SponsorDto {
            name: c.sponsor_name.clone(),
            annual_income: format_currency(c.wage as i64),
        })
        .collect();

    // History (most recent first, take last 12 months)
    let history_items: Vec<_> = finance.history.iter().take(12).collect();

    let mut chart_labels: Vec<String> = Vec::new();
    let mut chart_balances: Vec<i64> = Vec::new();
    let mut chart_incomes: Vec<i64> = Vec::new();
    let mut chart_expenses: Vec<i64> = Vec::new();

    let history_entries: Vec<FinanceHistoryEntry> = history_items.iter().map(|(date, bal)| {
        let month_str = format!("{}/{}", date.format("%m"), date.format("%y"));
        chart_labels.push(month_str.clone());
        chart_balances.push(bal.balance as i64);
        chart_incomes.push(bal.income as i64);
        chart_expenses.push(bal.outcome as i64);

        let net_val = bal.income - bal.outcome;
        FinanceHistoryEntry {
            month: format!("{}", date.format("%b %Y")),
            balance: format_currency(bal.balance as i64),
            balance_positive: bal.balance >= 0,
            income: format_currency(bal.income as i64),
            expenses: format_currency(bal.outcome as i64),
            net: format_currency(net_val as i64),
            net_positive: net_val >= 0,
        }
    }).collect();

    // Reverse chart data so it goes oldest -> newest for the chart
    chart_labels.reverse();
    chart_balances.reverse();
    chart_incomes.reverse();
    chart_expenses.reverse();

    let (neighbor_teams, country_leagues) = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/finances", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main);
    let title = team.name.clone();
    let league_title = league.map(|l| views::league_display_name(l, &i18n, simulator_data)).unwrap_or_default();

    Ok(TeamFinancesTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: club.colors.background.clone(),
        foreground_color: club.colors.foreground.clone(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "finances",
        show_finances_tab: true,
        show_academy_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::U18,
        balance,
        balance_positive,
        transfer_budget,
        wage_budget,
        annual_wages,
        monthly_income,
        monthly_expenses,
        net_monthly,
        net_monthly_positive,
        sponsors,
        history_entries,
        income_tv,
        income_matchday,
        income_sponsorship,
        income_merchandising,
        income_prize_money,
        expense_player_wages,
        expense_staff_wages,
        expense_facilities,
        chart_labels: serde_json::to_string(&chart_labels).unwrap_or_default(),
        chart_balances: serde_json::to_string(&chart_balances).unwrap_or_default(),
        chart_incomes: serde_json::to_string(&chart_incomes).unwrap_or_default(),
        chart_expenses: serde_json::to_string(&chart_expenses).unwrap_or_default(),
    })
}

fn format_currency(amount: i64) -> String {
    if amount.abs() >= 1_000_000 {
        format!("${:.1}M", amount as f64 / 1_000_000.0)
    } else if amount.abs() >= 1_000 {
        format!("${:.0}K", amount as f64 / 1_000.0)
    } else {
        format!("${}", amount)
    }
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            (format!("{}  |  {}", club_name, i18n.t(team.team_type.as_i18n_key())), team.slug.clone(), team.reputation.world)
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    let mut country_leagues: Vec<(u32, String, String)> = data
        .country_by_club(club_id)
        .map(|country| {
            country.leagues.leagues.iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);

    Ok((
        teams.into_iter().map(|(name, slug, _)| (name, slug)).collect(),
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}
