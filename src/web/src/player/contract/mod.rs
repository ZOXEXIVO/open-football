pub mod routes;

use crate::player::{PlayerPageChrome, PlayerPageColors};
use crate::shared::slug::{PlayerPage, PlayerSlug};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::utils::FormattingUtils;
use core::{ContractBonusType, ContractClauseType, ContractType, Player, SimulatorData, Team};
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
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
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
    pub is_force_match_selection: bool,
    pub watch_key: String,
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
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
    /// Per-match fee the parent club pays the borrower for fielding the loanee
    /// (full for a start). `None` when the loan carries no match fee.
    pub match_fee: Option<String>,
    /// Percentage of the wage the borrowing club covers, e.g. "60%".
    pub wage_contribution: Option<String>,
    /// Minimum official appearances the borrower committed to give the loanee.
    pub min_appearances: Option<u16>,
}

pub struct BonusDto {
    pub bonus_type: String,
    pub value: String,
}

pub struct ClauseDto {
    pub clause_type: String,
    pub value: String,
}

pub struct PlayerContractPage;

impl PlayerContractPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayerContractRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let now = simulator_data.date.date();

        let (player, team_opt, canonical) = match PlayerSlug::resolve_page(
            simulator_data,
            &route_params.player_slug,
            &route_params.lang,
            "/contract",
        )? {
            PlayerPage::Found {
                player,
                team,
                canonical_slug,
            } => (player, team, canonical_slug),
            PlayerPage::Redirect(r) => return Ok(r),
        };

        let (contract, bonuses, clauses) =
            ContractDetail::build(player, team_opt, simulator_data, now, &i18n);
        let loan_contract = LoanDetailDto::build(player, simulator_data, &i18n);
        let chrome = PlayerPageChrome::build(
            simulator_data,
            i18n,
            player,
            team_opt,
            &route_params.lang,
            &canonical,
            "contract",
            PlayerPageColors::Grey,
        )?;

        Ok(PlayerContractTemplate {
            css_version: chrome.css_version,
            computer_name: chrome.computer_name,
            cpu_brand: chrome.cpu_brand,
            cores_count: chrome.cores_count,
            title: chrome.title,
            sub_title_prefix: chrome.sub_title_prefix,
            sub_title_suffix: chrome.sub_title_suffix,
            sub_title: chrome.sub_title,
            sub_title_link: chrome.sub_title_link,
            sub_title_flag: chrome.sub_title_flag,
            header_color: chrome.header_color,
            foreground_color: chrome.foreground_color,
            menu_sections: chrome.menu_sections,
            i18n: chrome.i18n,
            lang: chrome.lang,
            active_tab: chrome.active_tab,
            player_id: chrome.player_id,
            player_slug: chrome.player_slug,
            club_id: chrome.club_id,
            is_on_loan: chrome.is_on_loan,
            is_injured: chrome.is_injured,
            is_unhappy: chrome.is_unhappy,
            is_force_match_selection: chrome.is_force_match_selection,
            watch_key: chrome.watch_key,
            events_count: chrome.events_count,
            interested_clubs_count: chrome.interested_clubs_count,
            awards_count: chrome.awards_count,
            news_count: chrome.news_count,
            contract,
            loan_contract,
            bonuses,
            clauses,
        }
        .into_response())
    }
}

pub struct ContractDetail;

impl ContractDetail {
    fn build(
        player: &Player,
        team_opt: Option<&Team>,
        data: &SimulatorData,
        now: chrono::NaiveDate,
        i18n: &I18n,
    ) -> (Option<ContractDetailDto>, Vec<BonusDto>, Vec<ClauseDto>) {
        let contract = match &player.contract {
            Some(c) => c,
            None => return (None, Vec::new(), Vec::new()),
        };

        let (club_name, club_slug) = Self::club_identity(team_opt, data, i18n);
        let contract_type = Self::contract_type_key(&contract.contract_type);
        let squad_status = match contract.squad_status.i18n_key() {
            "" => "-",
            key => i18n.t(key),
        };
        let transfer_status = Self::transfer_status(contract, i18n);
        let years_remaining = Self::years_remaining(contract.expiration, now, i18n);
        let bonuses = Self::bonuses(contract, i18n);
        let clauses = Self::clauses(contract, i18n);

        let detail = ContractDetailDto {
            club_name,
            club_slug,
            contract_type: i18n.t(contract_type).to_string(),
            squad_status: squad_status.to_string(),
            shirt_number: contract.shirt_number,
            salary: FormattingUtils::format_money(contract.salary as f64 / 52.0),
            salary_annual: FormattingUtils::format_money(contract.salary as f64),
            started: contract
                .started
                .map(|d| d.format("%d.%m.%Y").to_string())
                .unwrap_or_else(|| "-".to_string()),
            expiration: contract.expiration.format("%d.%m.%Y").to_string(),
            years_remaining,
            is_transfer_listed: contract.is_transfer_listed,
            transfer_status,
        };

        (Some(detail), bonuses, clauses)
    }

    fn club_identity(
        team_opt: Option<&Team>,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> (String, String) {
        team_opt
            .and_then(|t| data.club(t.club_id))
            .map(|club| {
                let slug = club
                    .teams
                    .teams
                    .first()
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();
                (club.name.clone(), slug)
            })
            .unwrap_or_else(|| (i18n.t("unknown").to_string(), String::new()))
    }

    fn contract_type_key(contract_type: &ContractType) -> &'static str {
        match contract_type {
            ContractType::FullTime => "contract_type_full_time",
            ContractType::PartTime => "contract_type_part_time",
            ContractType::Amateur => "contract_type_amateur",
            ContractType::Youth => "contract_type_youth",
            ContractType::NonContract => "contract_type_non_contract",
            ContractType::Loan => "contract_type_loan",
        }
    }

    fn transfer_status(contract: &core::PlayerClubContract, i18n: &I18n) -> String {
        if !contract.is_transfer_listed {
            return String::new();
        }
        match &contract.transfer_status {
            Some(core::PlayerTransferStatus::TransferListed) => {
                i18n.t("player_status_listed").to_string()
            }
            Some(core::PlayerTransferStatus::LoadListed) => {
                i18n.t("player_status_loan_listed").to_string()
            }
            Some(core::PlayerTransferStatus::TransferAndLoadListed) => {
                i18n.t("transfer_and_loan_listed").to_string()
            }
            None => i18n.t("player_status_listed").to_string(),
        }
    }

    fn years_remaining(
        expiration: chrono::NaiveDate,
        now: chrono::NaiveDate,
        i18n: &I18n,
    ) -> String {
        let days_remaining = (expiration - now).num_days();
        if days_remaining > 365 {
            let years = days_remaining / 365;
            let months = (days_remaining % 365) / 30;
            if months > 0 {
                format!(
                    "{} {} {} {}",
                    years,
                    i18n.t("unit_yr"),
                    months,
                    i18n.t("unit_mo")
                )
            } else {
                format!("{} {}", years, i18n.t("unit_yr"))
            }
        } else if days_remaining > 30 {
            format!("{} {}", days_remaining / 30, i18n.t("unit_mo"))
        } else if days_remaining > 0 {
            format!("{} {}", days_remaining, i18n.t("unit_days"))
        } else {
            i18n.t("expired").to_string()
        }
    }

    fn bonuses(contract: &core::PlayerClubContract, i18n: &I18n) -> Vec<BonusDto> {
        contract
            .bonuses
            .iter()
            .map(|b| BonusDto {
                bonus_type: i18n.t(Self::bonus_type_key(&b.bonus_type)).to_string(),
                value: FormattingUtils::format_money(b.value as f64),
            })
            .collect()
    }

    fn bonus_type_key(bonus_type: &ContractBonusType) -> &'static str {
        match bonus_type {
            ContractBonusType::AppearanceFee => "bonus_appearance_fee",
            ContractBonusType::GoalFee => "bonus_goal",
            ContractBonusType::CleanSheetFee => "bonus_clean_sheet",
            ContractBonusType::TeamOfTheYear => "bonus_team_of_year",
            ContractBonusType::TopGoalscorer => "bonus_top_goalscorer",
            ContractBonusType::PromotionFee => "bonus_promotion",
            ContractBonusType::AvoidRelegationFee => "bonus_avoid_relegation",
            ContractBonusType::InternationalCapFee => "bonus_international_cap",
            ContractBonusType::UnusedSubstitutionFee => "bonus_unused_sub",
            ContractBonusType::SigningBonus => "bonus_signing",
            ContractBonusType::LoyaltyBonus => "bonus_loyalty",
        }
    }

    fn clauses(contract: &core::PlayerClubContract, i18n: &I18n) -> Vec<ClauseDto> {
        contract
            .clauses
            .iter()
            .map(|c| ClauseDto {
                clause_type: i18n.t(Self::clause_type_key(&c.bonus_type)).to_string(),
                value: ClauseDto::format_value(&c.bonus_type, c.value),
            })
            .collect()
    }

    fn clause_type_key(clause_type: &ContractClauseType) -> &'static str {
        match clause_type {
            ContractClauseType::MinimumFeeRelease => "clause_min_fee_release",
            ContractClauseType::RelegationFeeRelease => "clause_relegation_release",
            ContractClauseType::NonPromotionRelease => "clause_non_promotion_release",
            ContractClauseType::YearlyWageRise => "clause_yearly_wage_rise",
            ContractClauseType::PromotionWageIncrease => "clause_promotion_wage_increase",
            ContractClauseType::RelegationWageDecrease => "clause_relegation_wage_decrease",
            ContractClauseType::StaffJobRelease => "clause_staff_job_release",
            ContractClauseType::SellOnFee => "clause_sell_on_fee",
            ContractClauseType::SellOnFeeProfit => "clause_sell_on_fee_profit",
            ContractClauseType::SeasonalLandmarkGoalBonus => "clause_landmark_goal_bonus",
            ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason => {
                "clause_1yr_extension_games"
            }
            ContractClauseType::MatchHighestEarner => "clause_match_highest_earner",
            ContractClauseType::WageAfterReachingClubCareerLeagueGames => "clause_wage_club_games",
            ContractClauseType::TopDivisionPromotionWageRise => "clause_top_div_promotion_rise",
            ContractClauseType::TopDivisionRelegationWageDrop => "clause_top_div_relegation_drop",
            ContractClauseType::MinimumFeeReleaseToForeignClubs => "clause_min_fee_foreign",
            ContractClauseType::MinimumFeeReleaseToHigherDivisionClubs => {
                "clause_min_fee_higher_div"
            }
            ContractClauseType::MinimumFeeReleaseToDomesticClubs => "clause_min_fee_domestic",
            ContractClauseType::WageAfterReachingInternationalCaps => "clause_wage_intl_caps",
            ContractClauseType::OptionalContractExtensionByClub => "clause_optional_extension",
        }
    }
}

impl LoanDetailDto {
    fn build(player: &Player, data: &SimulatorData, i18n: &I18n) -> Option<LoanDetailDto> {
        let loan = player.contract_loan.as_ref()?;

        let (from_name, from_slug) = loan
            .loan_from_club_id
            .and_then(|id| data.club(id))
            .map(|club| {
                let slug = club
                    .teams
                    .teams
                    .first()
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();
                (club.name.clone(), slug)
            })
            .unwrap_or_default();

        let (to_name, to_slug) = loan
            .loan_to_club_id
            .and_then(|id| data.club(id))
            .map(|club| {
                let slug = club
                    .teams
                    .teams
                    .first()
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();
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
            match_fee: loan
                .loan_match_fee
                .filter(|fee| *fee > 0)
                .map(|fee| FormattingUtils::format_money(fee as f64)),
            wage_contribution: loan.loan_wage_contribution_pct.map(|pct| format!("{pct}%")),
            min_appearances: loan.loan_min_appearances,
        })
    }
}

impl ClauseDto {
    fn format_value(clause_type: &ContractClauseType, value: i32) -> String {
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
}
