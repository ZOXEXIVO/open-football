pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::common::slug::{resolve_player_page, PlayerPage};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::utils::FormattingUtils;
use core::{ContractBonusType, ContractClauseType, ContractType, Player, PlayerSquadStatus, PlayerStatusType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerContractRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/contract/index.html")]
pub struct PlayerContractTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub player_slug: String,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
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
) -> ApiResult<Response> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let now = simulator_data.date.date();

    let (player, team_opt, canonical) = match resolve_player_page(
        simulator_data,
        &route_params.player_slug,
        &route_params.lang,
        "/contract",
    )? {
        PlayerPage::Found { player, team, canonical_slug } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let title = format!("{} {}", player.full_name.display_first_name(), player.full_name.display_last_name());

    let (contract, bonuses, clauses) = build_contract_detail(player, team_opt, simulator_data, now, &i18n);
    let loan_contract = build_loan_detail(player, simulator_data, &i18n);

    Ok(PlayerContractTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt.map(|t| t.name.clone()).unwrap_or_else(|| {
            if player.retired {
                i18n.t("retired").to_string()
            } else {
                i18n.t("free_agent").to_string()
            }
        }),
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
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        contract,
        loan_contract,
        bonuses,
        clauses,
    }.into_response())
}

fn build_contract_detail(
    player: &Player,
    team_opt: Option<&core::Team>,
    data: &SimulatorData,
    now: chrono::NaiveDate,
    i18n: &I18n,
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
        ContractType::FullTime => i18n.t("contract_type_full_time"),
        ContractType::PartTime => i18n.t("contract_type_part_time"),
        ContractType::Amateur => i18n.t("contract_type_amateur"),
        ContractType::Youth => i18n.t("contract_type_youth"),
        ContractType::NonContract => i18n.t("contract_type_non_contract"),
        ContractType::Loan => i18n.t("contract_type_loan"),
    };

    let squad_status = match contract.squad_status {
        PlayerSquadStatus::KeyPlayer => i18n.t("squad_key_player"),
        PlayerSquadStatus::FirstTeamRegular => i18n.t("squad_first_team_regular"),
        PlayerSquadStatus::FirstTeamSquadRotation => i18n.t("squad_rotation"),
        PlayerSquadStatus::MainBackupPlayer => i18n.t("squad_backup_player"),
        PlayerSquadStatus::HotProspectForTheFuture => i18n.t("squad_hot_prospect"),
        PlayerSquadStatus::DecentYoungster => i18n.t("squad_decent_youngster"),
        PlayerSquadStatus::NotNeeded => i18n.t("squad_not_needed"),
        _ => "-",
    };

    let transfer_status = if contract.is_transfer_listed {
        match &contract.transfer_status {
            Some(core::PlayerTransferStatus::TransferListed) => i18n.t("player_status_listed").to_string(),
            Some(core::PlayerTransferStatus::LoadListed) => i18n.t("player_status_loan_listed").to_string(),
            Some(core::PlayerTransferStatus::TransferAndLoadListed) => i18n.t("transfer_and_loan_listed").to_string(),
            None => i18n.t("player_status_listed").to_string(),
        }
    } else {
        String::new()
    };

    let days_remaining = (contract.expiration - now).num_days();
    let years_remaining = if days_remaining > 365 {
        let years = days_remaining / 365;
        let months = (days_remaining % 365) / 30;
        if months > 0 {
            format!("{} {} {} {}", years, i18n.t("unit_yr"), months, i18n.t("unit_mo"))
        } else {
            format!("{} {}", years, i18n.t("unit_yr"))
        }
    } else if days_remaining > 30 {
        format!("{} {}", days_remaining / 30, i18n.t("unit_mo"))
    } else if days_remaining > 0 {
        format!("{} {}", days_remaining, i18n.t("unit_days"))
    } else {
        i18n.t("expired").to_string()
    };

    let bonuses: Vec<BonusDto> = contract.bonuses.iter().map(|b| {
        BonusDto {
            bonus_type: match b.bonus_type {
                ContractBonusType::AppearanceFee => i18n.t("bonus_appearance_fee"),
                ContractBonusType::GoalFee => i18n.t("bonus_goal"),
                ContractBonusType::CleanSheetFee => i18n.t("bonus_clean_sheet"),
                ContractBonusType::TeamOfTheYear => i18n.t("bonus_team_of_year"),
                ContractBonusType::TopGoalscorer => i18n.t("bonus_top_goalscorer"),
                ContractBonusType::PromotionFee => i18n.t("bonus_promotion"),
                ContractBonusType::AvoidRelegationFee => i18n.t("bonus_avoid_relegation"),
                ContractBonusType::InternationalCapFee => i18n.t("bonus_international_cap"),
                ContractBonusType::UnusedSubstitutionFee => i18n.t("bonus_unused_sub"),
                ContractBonusType::SigningBonus => i18n.t("bonus_signing"),
                ContractBonusType::LoyaltyBonus => i18n.t("bonus_loyalty"),
            }.to_string(),
            value: FormattingUtils::format_money(b.value as f64),
        }
    }).collect();

    let clauses: Vec<ClauseDto> = contract.clauses.iter().map(|c| {
        ClauseDto {
            clause_type: match c.bonus_type {
                ContractClauseType::MinimumFeeRelease => i18n.t("clause_min_fee_release"),
                ContractClauseType::RelegationFeeRelease => i18n.t("clause_relegation_release"),
                ContractClauseType::NonPromotionRelease => i18n.t("clause_non_promotion_release"),
                ContractClauseType::YearlyWageRise => i18n.t("clause_yearly_wage_rise"),
                ContractClauseType::PromotionWageIncrease => i18n.t("clause_promotion_wage_increase"),
                ContractClauseType::RelegationWageDecrease => i18n.t("clause_relegation_wage_decrease"),
                ContractClauseType::StaffJobRelease => i18n.t("clause_staff_job_release"),
                ContractClauseType::SellOnFee => i18n.t("clause_sell_on_fee"),
                ContractClauseType::SellOnFeeProfit => i18n.t("clause_sell_on_fee_profit"),
                ContractClauseType::SeasonalLandmarkGoalBonus => i18n.t("clause_landmark_goal_bonus"),
                ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason => i18n.t("clause_1yr_extension_games"),
                ContractClauseType::MatchHighestEarner => i18n.t("clause_match_highest_earner"),
                ContractClauseType::WageAfterReachingClubCareerLeagueGames => i18n.t("clause_wage_club_games"),
                ContractClauseType::TopDivisionPromotionWageRise => i18n.t("clause_top_div_promotion_rise"),
                ContractClauseType::TopDivisionRelegationWageDrop => i18n.t("clause_top_div_relegation_drop"),
                ContractClauseType::MinimumFeeReleaseToForeignClubs => i18n.t("clause_min_fee_foreign"),
                ContractClauseType::MinimumFeeReleaseToHigherDivisionClubs => i18n.t("clause_min_fee_higher_div"),
                ContractClauseType::MinimumFeeReleaseToDomesticClubs => i18n.t("clause_min_fee_domestic"),
                ContractClauseType::WageAfterReachingInternationalCaps => i18n.t("clause_wage_intl_caps"),
                ContractClauseType::OptionalContractExtensionByClub => i18n.t("clause_optional_extension"),
            }.to_string(),
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

fn build_loan_detail(player: &Player, data: &SimulatorData, i18n: &I18n) -> Option<LoanDetailDto> {
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
        i18n.t("loan_in")
    } else {
        i18n.t("loan_out")
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
