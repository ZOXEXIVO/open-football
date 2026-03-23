pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::{ContractBonusType, ContractClauseType, ContractType, Player, PlayerSquadStatus, SimulatorData};
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerContractRequest {
    pub lang: String,
    pub player_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/contract/index.html")]
pub struct PlayerContractTemplate {
    pub css_version: &'static str,
    pub hostname: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub contract: Option<ContractDetailDto>,
    pub loan_contract: Option<LoanDetailDto>,
    pub bonuses: Vec<BonusDto>,
    pub clauses: Vec<ClauseDto>,
}

pub struct ContractDetailDto {
    pub club_name: String,
    pub club_slug: String,
    pub contract_type: String,
    pub squad_status: String,
    pub shirt_number: Option<u8>,
    pub salary: String,
    pub salary_annual: String,
    pub started: String,
    pub expiration: String,
    pub years_remaining: String,
    pub is_transfer_listed: bool,
    pub transfer_status: String,
}

pub struct LoanDetailDto {
    pub loan_type: String,
    pub from_club_name: String,
    pub from_club_slug: String,
    pub to_club_name: String,
    pub to_club_slug: String,
    pub salary: String,
    pub expiration: String,
}

pub struct BonusDto {
    pub bonus_type: String,
    pub value: String,
}

pub struct ClauseDto {
    pub clause_type: String,
    pub value: String,
}

pub async fn player_contract_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerContractRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let now = simulator_data.date.date();

    let active = simulator_data.player_with_team(route_params.player_id);
    let player = if let Some((p, _)) = active {
        p
    } else if let Some(p) = simulator_data.retired_player(route_params.player_id) {
        p
    } else {
        return Err(ApiError::NotFound(format!("Player with ID {} not found", route_params.player_id)));
    };
    let team_opt = active.map(|(_, t)| t);

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let title = format!("{} {}", player.full_name.display_first_name(), player.full_name.display_last_name());

    let (contract, bonuses, clauses) = build_contract_detail(player, team_opt, simulator_data, now);
    let loan_contract = build_loan_detail(player, simulator_data);

    Ok(PlayerContractTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        hostname: &crate::common::default_handler::HOSTNAME,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt.map(|t| t.name.clone()).unwrap_or_else(|| "Retired".to_string()),
        sub_title_link: team_opt.map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug)).unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt.and_then(|t| simulator_data.club(t.club_id).map(|c| c.colors.background.clone())).unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt.and_then(|t| simulator_data.club(t.club_id).map(|c| c.colors.foreground.clone())).unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
            views::team_menu(&mp, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main)
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "contract",
        player_id: player.id,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        contract,
        loan_contract,
        bonuses,
        clauses,
    })
}

fn build_contract_detail(
    player: &Player,
    team_opt: Option<&core::Team>,
    data: &SimulatorData,
    now: chrono::NaiveDate,
) -> (Option<ContractDetailDto>, Vec<BonusDto>, Vec<ClauseDto>) {
    let contract = match &player.contract {
        Some(c) => c,
        None => return (None, Vec::new(), Vec::new()),
    };

    let (club_name, club_slug) = team_opt
        .and_then(|t| data.club(t.club_id))
        .map(|club| {
            let slug = club.teams.teams.first()
                .map(|t| t.slug.clone())
                .unwrap_or_default();
            (club.name.clone(), slug)
        })
        .unwrap_or_else(|| ("Unknown".to_string(), String::new()));

    let contract_type = match contract.contract_type {
        ContractType::FullTime => "Full Time",
        ContractType::PartTime => "Part Time",
        ContractType::Amateur => "Amateur",
        ContractType::Youth => "Youth",
        ContractType::NonContract => "Non-Contract",
        ContractType::Loan => "Loan",
    };

    let squad_status = match contract.squad_status {
        PlayerSquadStatus::KeyPlayer => "Key Player",
        PlayerSquadStatus::FirstTeamRegular => "First Team Regular",
        PlayerSquadStatus::FirstTeamSquadRotation => "Squad Rotation",
        PlayerSquadStatus::MainBackupPlayer => "Backup Player",
        PlayerSquadStatus::HotProspectForTheFuture => "Hot Prospect",
        PlayerSquadStatus::DecentYoungster => "Decent Youngster",
        PlayerSquadStatus::NotNeeded => "Not Needed",
        _ => "-",
    };

    let transfer_status = if contract.is_transfer_listed {
        match &contract.transfer_status {
            Some(core::PlayerTransferStatus::TransferListed) => "Transfer Listed".to_string(),
            Some(core::PlayerTransferStatus::LoadListed) => "Loan Listed".to_string(),
            Some(core::PlayerTransferStatus::TransferAndLoadListed) => "Transfer & Loan Listed".to_string(),
            None => "Transfer Listed".to_string(),
        }
    } else {
        String::new()
    };

    let days_remaining = (contract.expiration - now).num_days();
    let years_remaining = if days_remaining > 365 {
        let years = days_remaining / 365;
        let months = (days_remaining % 365) / 30;
        if months > 0 {
            format!("{} yr {} mo", years, months)
        } else {
            format!("{} yr", years)
        }
    } else if days_remaining > 30 {
        format!("{} mo", days_remaining / 30)
    } else if days_remaining > 0 {
        format!("{} days", days_remaining)
    } else {
        "Expired".to_string()
    };

    let bonuses: Vec<BonusDto> = contract.bonuses.iter().map(|b| {
        BonusDto {
            bonus_type: match b.bonus_type {
                ContractBonusType::AppearanceFee => "Appearance Fee".to_string(),
                ContractBonusType::GoalFee => "Goal Bonus".to_string(),
                ContractBonusType::CleanSheetFee => "Clean Sheet Bonus".to_string(),
                ContractBonusType::TeamOfTheYear => "Team of the Year".to_string(),
                ContractBonusType::TopGoalscorer => "Top Goalscorer".to_string(),
                ContractBonusType::PromotionFee => "Promotion Bonus".to_string(),
                ContractBonusType::AvoidRelegationFee => "Avoid Relegation Bonus".to_string(),
                ContractBonusType::InternationalCapFee => "International Cap Fee".to_string(),
                ContractBonusType::UnusedSubstitutionFee => "Unused Sub Fee".to_string(),
            },
            value: FormattingUtils::format_money(b.value as f64),
        }
    }).collect();

    let clauses: Vec<ClauseDto> = contract.clauses.iter().map(|c| {
        ClauseDto {
            clause_type: match c.bonus_type {
                ContractClauseType::MinimumFeeRelease => "Minimum Fee Release".to_string(),
                ContractClauseType::RelegationFeeRelease => "Relegation Release".to_string(),
                ContractClauseType::NonPromotionRelease => "Non-Promotion Release".to_string(),
                ContractClauseType::YearlyWageRise => "Yearly Wage Rise".to_string(),
                ContractClauseType::PromotionWageIncrease => "Promotion Wage Rise".to_string(),
                ContractClauseType::RelegationWageDecrease => "Relegation Wage Cut".to_string(),
                ContractClauseType::StaffJobRelease => "Staff Job Release".to_string(),
                ContractClauseType::SellOnFee => "Sell-on Fee".to_string(),
                ContractClauseType::SellOnFeeProfit => "Sell-on Fee (Profit)".to_string(),
                ContractClauseType::SeasonalLandmarkGoalBonus => "Landmark Goal Bonus".to_string(),
                ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason => "1yr Extension (Games)".to_string(),
                ContractClauseType::MatchHighestEarner => "Match Highest Earner".to_string(),
                ContractClauseType::WageAfterReachingClubCareerLeagueGames => "Wage Rise (Club Games)".to_string(),
                ContractClauseType::TopDivisionPromotionWageRise => "Top Div Promotion Rise".to_string(),
                ContractClauseType::TopDivisionRelegationWageDrop => "Top Div Relegation Drop".to_string(),
                ContractClauseType::MinimumFeeReleaseToForeignClubs => "Min Fee (Foreign Clubs)".to_string(),
                ContractClauseType::MinimumFeeReleaseToHigherDivisionClubs => "Min Fee (Higher Div)".to_string(),
                ContractClauseType::MinimumFeeReleaseToDomesticClubs => "Min Fee (Domestic)".to_string(),
                ContractClauseType::WageAfterReachingInternationalCaps => "Wage Rise (Int'l Caps)".to_string(),
                ContractClauseType::OptionalContractExtensionByClub => "Optional Extension".to_string(),
            },
            value: format_clause_value(&c.bonus_type, c.value),
        }
    }).collect();

    let detail = ContractDetailDto {
        club_name,
        club_slug,
        contract_type: contract_type.to_string(),
        squad_status: squad_status.to_string(),
        shirt_number: contract.shirt_number,
        salary: FormattingUtils::format_money(contract.salary as f64 / 52.0),
        salary_annual: FormattingUtils::format_money(contract.salary as f64),
        started: contract.started.map(|d| d.format("%d.%m.%Y").to_string()).unwrap_or_else(|| "-".to_string()),
        expiration: contract.expiration.format("%d.%m.%Y").to_string(),
        years_remaining,
        is_transfer_listed: contract.is_transfer_listed,
        transfer_status,
    };

    (Some(detail), bonuses, clauses)
}

fn build_loan_detail(player: &Player, data: &SimulatorData) -> Option<LoanDetailDto> {
    let loan = player.contract_loan.as_ref()?;

    let (from_name, from_slug) = loan.loan_from_club_id
        .and_then(|id| data.club(id))
        .map(|club| {
            let slug = club.teams.teams.first().map(|t| t.slug.clone()).unwrap_or_default();
            (club.name.clone(), slug)
        })
        .unwrap_or_default();

    let (to_name, to_slug) = loan.loan_to_club_id
        .and_then(|id| data.club(id))
        .map(|club| {
            let slug = club.teams.teams.first().map(|t| t.slug.clone()).unwrap_or_default();
            (club.name.clone(), slug)
        })
        .unwrap_or_default();

    let loan_type = if loan.loan_from_club_id.is_some() {
        "Loan In"
    } else {
        "Loan Out"
    };

    Some(LoanDetailDto {
        loan_type: loan_type.to_string(),
        from_club_name: from_name,
        from_club_slug: from_slug,
        to_club_name: to_name,
        to_club_slug: to_slug,
        salary: FormattingUtils::format_money(loan.salary as f64),
        expiration: loan.expiration.format("%d.%m.%Y").to_string(),
    })
}

fn format_clause_value(clause_type: &ContractClauseType, value: i32) -> String {
    match clause_type {
        ContractClauseType::YearlyWageRise
        | ContractClauseType::PromotionWageIncrease
        | ContractClauseType::RelegationWageDecrease
        | ContractClauseType::TopDivisionPromotionWageRise
        | ContractClauseType::TopDivisionRelegationWageDrop
        | ContractClauseType::SellOnFee
        | ContractClauseType::SellOnFeeProfit => format!("{}%", value),
        _ => FormattingUtils::format_money(value as f64),
    }
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
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
