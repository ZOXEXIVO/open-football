//! Shared expected-annual-package value helper.
//!
//! Happiness, acceptance scoring, and renewal sweetener tuning all need
//! to express "what is the player actually worth per year, all-in?" —
//! base salary plus the realistic expected value of bonuses and clauses.
//!
//! Without a single source of truth here, three slightly different
//! formulas drift apart: happiness might count a loyalty bonus at full
//! face, the AI might count it amortised, and acceptance might ignore
//! it entirely. This module fixes the drift.
//!
//! The two entry points are deliberately narrow:
//!   - [`package_inputs_from_proposal`] — un-signed package being
//!     evaluated for acceptance / renewal scoring.
//!   - [`package_inputs_from_contract`] — installed contract being
//!     evaluated by happiness or wage-envy passes.
//!
//! Both build a [`PackageInputs`] which feeds [`expected_annual_value`].

use crate::PlayerSquadStatus;
use crate::club::player::contract::contract::{ContractBonusType, PlayerClubContract};
use crate::club::player::mailbox::PlayerContractProposal;
use crate::club::player::player::Player;

/// Frequency-of-occurrence weights tuned for a typical season. Used to
/// translate per-event bonus values (paid per appearance / per goal /
/// per clean sheet) into an expected annual value.
///
/// Numbers reflect what a "typical first-team contributor" books, not
/// best-case stars. Acceptance / happiness scoring should not over-value
/// optimistic packages — a striker scoring 30 a season is rare, so the
/// goal bonus is weighted at 8 (decent striker median), not 30.
mod weights {
    pub const APPS_PER_SEASON_KEY_PLAYER: u32 = 38;
    pub const APPS_PER_SEASON_REGULAR: u32 = 28;
    pub const APPS_PER_SEASON_ROTATION: u32 = 18;
    pub const APPS_PER_SEASON_BACKUP: u32 = 8;

    pub const UNUSED_SUB_APPEARANCES: u32 = 12;

    pub const GOALS_FORWARD: u32 = 12;
    pub const GOALS_MIDFIELDER: u32 = 5;
    pub const GOALS_DEFENDER: u32 = 2;

    pub const CLEAN_SHEETS_GK: u32 = 12;
    pub const CLEAN_SHEETS_DEFENDER: u32 = 8;

    /// Probability the team triggers PromotionFee in any given season.
    /// 12% across all clubs / divisions; calibrated for promotion-tier
    /// clubs only is overkill for happiness/acceptance scoring.
    pub const PROMOTION_PROBABILITY: f32 = 0.12;
    pub const AVOID_RELEGATION_PROBABILITY: f32 = 0.30;
    pub const INTL_CAPS_PER_SEASON: u32 = 6;
}

/// Package shape passed to [`expected_annual_value`]. Carrier struct so
/// callers can fill in only the fields they have without juggling many
/// arguments.
#[derive(Debug, Clone)]
pub struct PackageInputs {
    pub base_salary: u32,
    pub years: u8,
    pub signing_bonus: u32,
    /// Set when the signing bonus has already been paid out by a
    /// previous monthly finance pass — the happiness model should stop
    /// counting it as outstanding compensation.
    pub signing_bonus_already_paid: bool,
    pub loyalty_bonus: u32,
    pub appearance_fee: u32,
    pub unused_sub_fee: u32,
    pub goal_bonus: u32,
    pub clean_sheet_bonus: u32,
    pub promotion_bonus: u32,
    pub avoid_relegation_bonus: u32,
    pub international_cap_bonus: u32,
    /// Release-clause value the player would consider attractive. Only
    /// counted when `release_clause_attractive` is true — defenders
    /// happily playing in the Bundesliga don't value an escape route.
    pub release_clause: u32,
    pub release_clause_attractive: bool,
    pub squad_status: PlayerSquadStatus,
    /// True if the player is a forward — drives goal-bonus expectation.
    pub is_forward: bool,
    pub is_midfielder: bool,
    pub is_defender: bool,
    pub is_goalkeeper: bool,
}

impl PackageInputs {
    fn empty(base_salary: u32, years: u8) -> Self {
        PackageInputs {
            base_salary,
            years,
            signing_bonus: 0,
            signing_bonus_already_paid: false,
            loyalty_bonus: 0,
            appearance_fee: 0,
            unused_sub_fee: 0,
            goal_bonus: 0,
            clean_sheet_bonus: 0,
            promotion_bonus: 0,
            avoid_relegation_bonus: 0,
            international_cap_bonus: 0,
            release_clause: 0,
            release_clause_attractive: false,
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            is_forward: false,
            is_midfielder: false,
            is_defender: false,
            is_goalkeeper: false,
        }
    }
}

/// Build inputs from a fresh proposal — un-installed package, signing
/// bonus has NOT been paid yet, so it's amortised over the contract
/// length. `player` supplies position + ambition for the goal/clause
/// weighting.
pub fn package_inputs_from_proposal(
    proposal: &PlayerContractProposal,
    player: &Player,
) -> PackageInputs {
    let pos = player.position();
    let ambition = player.attributes.ambition;
    PackageInputs {
        base_salary: proposal.salary,
        years: proposal.years.max(1),
        signing_bonus: proposal.signing_bonus,
        signing_bonus_already_paid: false,
        loyalty_bonus: proposal.loyalty_bonus,
        appearance_fee: proposal.appearance_fee.unwrap_or(0),
        unused_sub_fee: proposal.unused_sub_fee.unwrap_or(0),
        goal_bonus: proposal.goal_bonus.unwrap_or(0),
        clean_sheet_bonus: proposal.clean_sheet_bonus.unwrap_or(0),
        promotion_bonus: proposal.promotion_bonus.unwrap_or(0),
        avoid_relegation_bonus: proposal.avoid_relegation_bonus.unwrap_or(0),
        international_cap_bonus: proposal.international_cap_bonus.unwrap_or(0),
        release_clause: proposal.release_clause.unwrap_or(0),
        release_clause_attractive: ambition >= 13.0,
        squad_status: proposal.squad_status_promise.clone().unwrap_or_else(|| {
            player
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular)
        }),
        is_forward: pos.is_forward(),
        is_midfielder: pos.is_midfielder(),
        is_defender: pos.is_defender(),
        is_goalkeeper: pos.is_goalkeeper(),
    }
}

/// Build inputs from an installed contract — signing bonus is treated as
/// paid (happiness shouldn't keep counting a one-shot payment that
/// already hit the bank).
pub fn package_inputs_from_contract(
    contract: &PlayerClubContract,
    player: &Player,
) -> PackageInputs {
    let pos = player.position();
    let ambition = player.attributes.ambition;
    let mut inputs = PackageInputs::empty(contract.salary, 1);
    inputs.squad_status = contract.squad_status.clone();
    inputs.signing_bonus_already_paid = contract.signing_bonus_paid;
    inputs.is_forward = pos.is_forward();
    inputs.is_midfielder = pos.is_midfielder();
    inputs.is_defender = pos.is_defender();
    inputs.is_goalkeeper = pos.is_goalkeeper();
    inputs.release_clause_attractive = ambition >= 13.0;
    for bonus in &contract.bonuses {
        let v = bonus.value.max(0) as u32;
        match bonus.bonus_type {
            ContractBonusType::SigningBonus => inputs.signing_bonus = v,
            ContractBonusType::LoyaltyBonus => inputs.loyalty_bonus = v,
            ContractBonusType::AppearanceFee => inputs.appearance_fee = v,
            ContractBonusType::UnusedSubstitutionFee => inputs.unused_sub_fee = v,
            ContractBonusType::GoalFee => inputs.goal_bonus = v,
            ContractBonusType::CleanSheetFee => inputs.clean_sheet_bonus = v,
            ContractBonusType::PromotionFee => inputs.promotion_bonus = v,
            ContractBonusType::AvoidRelegationFee => inputs.avoid_relegation_bonus = v,
            ContractBonusType::InternationalCapFee => inputs.international_cap_bonus = v,
            // Inert bonus types — neither happiness nor acceptance should
            // value them until they have lifecycle effects.
            ContractBonusType::TeamOfTheYear | ContractBonusType::TopGoalscorer => {}
        }
    }
    inputs
}

/// Total expected annual value of the package — base salary plus the
/// probability/frequency-weighted value of every bonus, plus a small
/// option-style value for an attractive release clause.
pub fn expected_annual_value(p: &PackageInputs) -> u32 {
    let years = p.years.max(1) as u32;
    let signing_amortised = if p.signing_bonus_already_paid {
        0
    } else {
        p.signing_bonus / years
    };

    let apps_per_season = match p.squad_status {
        PlayerSquadStatus::KeyPlayer => weights::APPS_PER_SEASON_KEY_PLAYER,
        PlayerSquadStatus::FirstTeamRegular => weights::APPS_PER_SEASON_REGULAR,
        PlayerSquadStatus::FirstTeamSquadRotation | PlayerSquadStatus::HotProspectForTheFuture => {
            weights::APPS_PER_SEASON_ROTATION
        }
        PlayerSquadStatus::MainBackupPlayer | PlayerSquadStatus::DecentYoungster => {
            weights::APPS_PER_SEASON_BACKUP
        }
        _ => weights::APPS_PER_SEASON_BACKUP,
    };

    let appearance_value = (p.appearance_fee as u64 * apps_per_season as u64) as u32;
    let unused_sub_value =
        (p.unused_sub_fee as u64 * weights::UNUSED_SUB_APPEARANCES as u64) as u32;

    let expected_goals = if p.is_forward {
        weights::GOALS_FORWARD
    } else if p.is_midfielder {
        weights::GOALS_MIDFIELDER
    } else if p.is_defender {
        weights::GOALS_DEFENDER
    } else {
        0
    };
    let goal_value = (p.goal_bonus as u64 * expected_goals as u64) as u32;

    let expected_cs = if p.is_goalkeeper {
        weights::CLEAN_SHEETS_GK
    } else if p.is_defender {
        weights::CLEAN_SHEETS_DEFENDER
    } else {
        0
    };
    let clean_sheet_value = (p.clean_sheet_bonus as u64 * expected_cs as u64) as u32;

    // Probability-weighted seasonal bonuses.
    let promotion_value = ((p.promotion_bonus as f32) * weights::PROMOTION_PROBABILITY) as u32;
    let avoid_rel_value =
        ((p.avoid_relegation_bonus as f32) * weights::AVOID_RELEGATION_PROBABILITY) as u32;
    let intl_value =
        (p.international_cap_bonus as u64 * weights::INTL_CAPS_PER_SEASON as u64) as u32;

    // Release-clause "option value" — only counted for ambitious /
    // high-leverage players, capped at 5% of the clause threshold (this
    // is an annualised willingness-to-pay, not a real cash payout).
    let release_value = if p.release_clause_attractive {
        ((p.release_clause as f32) * 0.05) as u32
    } else {
        0
    };

    p.base_salary
        .saturating_add(p.loyalty_bonus)
        .saturating_add(signing_amortised)
        .saturating_add(appearance_value)
        .saturating_add(unused_sub_value)
        .saturating_add(goal_value)
        .saturating_add(clean_sheet_value)
        .saturating_add(promotion_value)
        .saturating_add(avoid_rel_value)
        .saturating_add(intl_value)
        .saturating_add(release_value)
}
