//! Side effects of a permanent transfer / free signing / loan landing
//! on the player. These methods own the legal-state mutation: contract
//! install, status reset, signing-plan stage, sell-on bookkeeping.
//!
//! The *social* fallout (other players reacting to a teammate
//! leaving, dream-move collapse, bid rejection) lives in
//! [`super::transfer_social`].

use chrono::{Duration, NaiveDate};

use super::types::{LoanCompletion, TransferCompletion};
use crate::club::player::adaptation::PendingSigning;
use crate::club::player::calculators::WageCalculator;
use crate::club::player::contract::contract::{
    is_inert_bonus, is_inert_clause, ContractBonus, ContractClause, ContractClauseType,
};
use crate::club::player::load::PlayerLoad;
use crate::club::player::player::{Player, SellOnObligation};
use crate::club::PlayerClubContract;
use crate::{ContractBonusType, Person, PlayerHappiness, PlayerPlan, PlayerStatusType};

impl Player {
    /// React to a completed permanent transfer. Resets stats history,
    /// clears transient statuses and happiness, installs a fresh contract
    /// and signing plan, and stages a pending signing so the next sim
    /// tick can emit the shock / role-fit / promise events.
    pub fn complete_transfer(&mut self, t: TransferCompletion<'_>) {
        let previous_salary = self.contract.as_ref().map(|c| c.salary);
        self.on_transfer(t.from, t.to, t.fee, t.date);
        self.sold_from = Some((t.selling_club_id, t.fee));
        self.reset_on_club_change();
        self.install_permanent_contract(
            t.date,
            t.to.reputation,
            t.buying_league_reputation,
            t.agreed_wage,
        );
        self.plan = Some(PlayerPlan::from_signing(self.age(t.date), t.fee, t.date));
        if let Some(pct) = t.record_sell_on {
            if pct > 0.0 && self.sell_on_obligations.len() < 3 {
                self.sell_on_obligations.push(SellOnObligation {
                    beneficiary_club_id: t.selling_club_id,
                    percentage: pct,
                });
            }
        }
        self.pending_signing = Some(PendingSigning {
            previous_salary,
            fee: t.fee,
            is_loan: false,
            destination_club_id: t.buying_club_id,
        });
    }

    /// React to a completed free-agent signing — the no-source-club mirror
    /// of `complete_transfer`. History flows through `on_free_agent_signing`
    /// (no synthetic "Free Agent" career row); there is no `sold_from` and
    /// no sell-on because there is no selling club. Contract install,
    /// signing plan, and pending-signing settlement are identical to the
    /// paid-transfer path so the new club gets the same shock / role-fit /
    /// promise events. Fee is always 0 for a free signing.
    pub fn complete_free_agent_signing(
        &mut self,
        to: &crate::TeamInfo,
        date: NaiveDate,
        buying_club_id: u32,
        buying_league_reputation: u16,
        agreed_wage: Option<u32>,
    ) {
        let previous_salary = self.contract.as_ref().map(|c| c.salary);
        self.on_free_agent_signing(to, date);
        self.reset_on_club_change();
        self.install_permanent_contract(date, to.reputation, buying_league_reputation, agreed_wage);
        self.plan = Some(PlayerPlan::from_signing(self.age(date), 0.0, date));
        self.pending_signing = Some(PendingSigning {
            previous_salary,
            fee: 0.0,
            is_loan: false,
            destination_club_id: buying_club_id,
        });
    }

    /// Take and return all active sell-on obligations, clearing the list on
    /// the player. Used by execution to route money to past beneficiaries
    /// before crediting the current seller.
    pub fn drain_sell_on_obligations(&mut self) -> Vec<SellOnObligation> {
        std::mem::take(&mut self.sell_on_obligations)
    }

    /// React to a completed loan. The parent contract is preserved; the
    /// borrowing club's contract is installed as `contract_loan`. We also
    /// annotate the parent contract's `loan_to_club_id` so downstream
    /// queries (UI, match-day loaned-in collector) can locate the borrower
    /// directly from the parent-side contract without digging into
    /// `contract_loan`.
    pub fn complete_loan(&mut self, l: LoanCompletion<'_>) {
        let borrowing_id = l.borrowing_club_id;
        self.on_loan(l.from, l.to, l.loan_fee, l.date);
        self.reset_on_club_change();
        if let Some(parent) = self.contract.as_mut() {
            parent.loan_to_club_id = Some(borrowing_id);
        }
        self.contract_loan = Some(l.loan_contract);
        self.pending_signing = Some(PendingSigning {
            previous_salary: None,
            fee: l.loan_fee,
            is_loan: true,
            destination_club_id: borrowing_id,
        });
    }

    fn reset_on_club_change(&mut self) {
        const TRANSIENT: [PlayerStatusType; 10] = [
            PlayerStatusType::Lst,
            PlayerStatusType::Loa,
            PlayerStatusType::Frt,
            PlayerStatusType::Req,
            PlayerStatusType::Unh,
            PlayerStatusType::Trn,
            PlayerStatusType::Bid,
            PlayerStatusType::Wnt,
            PlayerStatusType::Sct,
            PlayerStatusType::Enq,
        ];
        for s in TRANSIENT {
            self.statuses.remove(s);
        }
        self.happiness = PlayerHappiness::new();
        self.load = PlayerLoad::new();
        // Force-selection is the previous manager's pin — the new
        // club's manager hasn't expressed a preference yet, so the
        // flag mustn't survive the move.
        self.is_force_match_selection = false;
    }

    /// Install a fresh permanent contract on this player at the buying club.
    ///
    /// This is the canonical contract-installation policy used by both the
    /// AI transfer pipeline (via `complete_transfer`) and the manual web
    /// UI. The single source of truth for two decisions:
    ///
    ///  - **Length:** age-banded (5y under 24, 4y under 28, 3y under 32,
    ///    otherwise 2y). Younger players get longer deals.
    ///  - **Salary:** `agreed_wage` if `Some` (for AI deals where the
    ///    negotiation already settled on a number); otherwise computed
    ///    via `WageCalculator::expected_annual_wage` from the player's
    ///    profile and the buying club's reputation.
    ///
    /// Inputs `buying_club_reputation` and `buying_league_reputation` are
    /// raw 0–10000 reputation values for the club's main team and its
    /// league. The wage calculator normalises them internally.
    ///
    /// Side effects: `self.contract` is set to a fresh
    /// `PlayerClubContract` with `squad_status = NotYetSet`; callers that
    /// know the destination roster should update `squad_status`
    /// afterwards. `self.contract_loan` is cleared to drop any prior
    /// borrowing-club contract.
    pub fn install_permanent_contract(
        &mut self,
        date: NaiveDate,
        buying_club_reputation: u16,
        buying_league_reputation: u16,
        agreed_wage: Option<u32>,
    ) {
        let age = self.age(date);
        let years = if age < 24 {
            5
        } else if age < 28 {
            4
        } else if age < 32 {
            3
        } else {
            2
        };
        let expiry = date
            .checked_add_signed(Duration::days(years * 365))
            .unwrap_or(date);
        let salary = agreed_wage.unwrap_or_else(|| {
            let club_score = (buying_club_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
            WageCalculator::expected_annual_wage(self, age, club_score, buying_league_reputation)
        });
        let mut contract = PlayerClubContract::new(salary, expiry);
        // Install a profile-appropriate set of bonuses + clauses so
        // transfer-completed contracts feel like the same market as
        // renewals. Without this every transfer signs a bare
        // salary/years deal and never pays a goal/clean-sheet/loyalty
        // bonus.
        install_transfer_package(&mut contract, self, age, buying_club_reputation);
        // Anchor international-cap baseline so any cap bonus added to
        // this fresh contract pays only on FUTURE caps, not on the
        // ones the player accumulated before the transfer.
        if contract
            .bonuses
            .iter()
            .any(|b| matches!(b.bonus_type, ContractBonusType::InternationalCapFee))
        {
            self.last_intl_caps_paid = self.player_attributes.international_apps;
        }
        self.contract = Some(contract);
        self.contract_loan = None;
    }

    /// Extend the parent contract (if needed) so it doesn't expire while the
    /// player is out on loan — used by the loan pipeline before shipping the
    /// player to the borrower.
    pub fn ensure_contract_covers_loan_end(&mut self, loan_end: NaiveDate) {
        let min_expiry = loan_end
            .checked_add_signed(Duration::days(365))
            .unwrap_or(loan_end);
        if let Some(ref mut contract) = self.contract {
            if contract.expiration < min_expiry {
                contract.expiration = min_expiry;
            }
        }
    }
}

// ============================================================
// File-private helpers — contract decoration policy
// ============================================================

/// Decorate a freshly-installed transfer contract with profile-shaped
/// bonuses and clauses. Mirrors the renewal AI's `decorate_proposal` so
/// the transfer market and renewal market install the same shape of
/// deal — without this every transfer signs a bare salary/years deal.
///
/// Profile axes:
///   - **Star** (rep ≥ 5000 or ability ≥ 150): signing + loyalty
///     bonuses, position-based goal/clean-sheet bonus, release clause.
///   - **YoungProspect** (age ≤ 23 + potential ≥ 130): yearly wage
///     rise, optional extension, apps-threshold bump, release clause
///     for ambitious ones.
///   - **Veteran** (age ≥ 31): short-term flavour — loyalty bonus,
///     appearance fee, no release clause.
///   - **Backup / Standard**: small appearance + unused-sub fee.
///
/// Relegation-risk clubs (low buying-club reputation) attach a
/// relegation wage-drop clause so the cost-side trims if the club goes
/// down. Top clubs don't need it.
fn install_transfer_package(
    contract: &mut PlayerClubContract,
    player: &Player,
    age: u8,
    buying_club_reputation: u16,
) {
    let salary = contract.salary;
    let pos = player.position();
    let rep = player.player_attributes.current_reputation;
    let ability = player.player_attributes.current_ability;
    let potential = player.player_attributes.potential_ability;
    let ambition = player.attributes.ambition;
    let loyalty = player.attributes.loyalty;
    let club_rep_score = (buying_club_reputation as f32 / 10_000.0).clamp(0.0, 1.0);

    let is_star = rep > 5000 || ability >= 150;
    let is_prospect = age <= 23 && potential >= 130;
    let is_veteran = age >= 31;

    if is_star {
        // Signing + loyalty + position bonus + (large) release clause.
        contract.bonuses.push(ContractBonus::new(
            ((salary as f32) * 0.30) as i32,
            ContractBonusType::SigningBonus,
        ));
        contract.bonuses.push(ContractBonus::new(
            ((salary as f32) * 0.15) as i32,
            ContractBonusType::LoyaltyBonus,
        ));
        if pos.is_forward() || pos.is_midfielder() {
            contract.bonuses.push(ContractBonus::new(
                ((salary as f32) * 0.012) as i32,
                ContractBonusType::GoalFee,
            ));
        } else if pos.is_goalkeeper() || pos.is_defender() {
            contract.bonuses.push(ContractBonus::new(
                ((salary as f32) * 0.012) as i32,
                ContractBonusType::CleanSheetFee,
            ));
        }
        contract.bonuses.push(ContractBonus::new(
            ((salary as f32) * 0.01) as i32,
            ContractBonusType::AppearanceFee,
        ));
        let release_value = release_clause_value(ability, rep, 1.4);
        contract.clauses.push(ContractClause::new(
            release_value as i32,
            ContractClauseType::MinimumFeeRelease,
        ));
    } else if is_prospect {
        // Long progression — yearly rise, optional extension, apps step.
        contract
            .clauses
            .push(ContractClause::new(8, ContractClauseType::YearlyWageRise));
        contract.clauses.push(ContractClause::new(
            1,
            ContractClauseType::OptionalContractExtensionByClub,
        ));
        contract.clauses.push(ContractClause::new_threshold_pct(
            50,
            25,
            ContractClauseType::WageAfterReachingClubCareerLeagueGames,
        ));
        if ambition >= 13.0 {
            let release_value = release_clause_value(ability, rep, 1.0);
            contract.clauses.push(ContractClause::new(
                release_value as i32,
                ContractClauseType::MinimumFeeRelease,
            ));
        }
    } else if is_veteran {
        // Shorter-deal flavour — loyalty + appearance + extension on
        // hitting a season's apps. No release clause; veterans don't
        // negotiate them.
        if loyalty >= 10.0 {
            contract.bonuses.push(ContractBonus::new(
                ((salary as f32) * 0.10) as i32,
                ContractBonusType::LoyaltyBonus,
            ));
        }
        contract.bonuses.push(ContractBonus::new(
            ((salary as f32) * 0.02) as i32,
            ContractBonusType::AppearanceFee,
        ));
        contract.clauses.push(ContractClause::new(
            20,
            ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason,
        ));
    } else {
        // Standard / backup — modest appearance + unused-sub fee.
        contract.bonuses.push(ContractBonus::new(
            ((salary as f32) * 0.04) as i32,
            ContractBonusType::AppearanceFee,
        ));
        contract.bonuses.push(ContractBonus::new(
            ((salary as f32) * 0.005) as i32,
            ContractBonusType::UnusedSubstitutionFee,
        ));
    }

    // Relegation-risk clubs add a wage-drop clause so the cost side
    // trims if they go down. Threshold ≈ bottom-third of the league
    // reputation distribution.
    if club_rep_score < 0.35 {
        contract.clauses.push(ContractClause::new(
            20,
            ContractClauseType::RelegationWageDecrease,
        ));
    }

    // Final guard — strip any inert bonuses/clauses a future caller
    // might add. The is_inert_* lists are the source of truth for
    // "decorative without payout site"; the install path enforces it.
    contract.bonuses.retain(|b| !is_inert_bonus(&b.bonus_type));
    contract.clauses.retain(|c| !is_inert_clause(&c.bonus_type));
}

fn release_clause_value(ability: u8, current_reputation: i16, scale: f32) -> u32 {
    let base = (ability as u32) * (ability as u32) * 4_000;
    let rep_boost = (current_reputation.max(0) as u32) * 8_000;
    ((base + rep_boost) as f32 * scale) as u32
}
