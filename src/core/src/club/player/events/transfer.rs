//! Side effects of a permanent transfer / free signing / loan landing
//! on the player. These methods own the legal-state mutation: contract
//! install, status reset, signing-plan stage, sell-on bookkeeping.
//!
//! The *social* fallout (other players reacting to a teammate
//! leaving, dream-move collapse, bid rejection) lives in
//! [`super::transfer_social`].

use chrono::{Duration, NaiveDate};

use super::types::{LoanCompletion, TransferCompletion};
use crate::TeamInfo;
use crate::club::PlayerClubContract;
use crate::club::player::adaptation::PendingSigning;
use crate::club::player::calculators::WageCalculator;
use crate::club::player::contract::contract::{
    ContractBonus, ContractClause, ContractClauseType, is_inert_bonus, is_inert_clause,
};
use crate::club::player::load::PlayerLoad;
use crate::club::player::player::{Player, SellOnObligation};
use crate::club::staff::perception::PotentialEstimator;
use crate::transfers::offer::{PersonalTermsOffer, PromisedSquadStatus};
use crate::{
    ContractBonusType, HappinessEventType, Person, PlayerHappiness, PlayerPlan, PlayerSquadStatus,
    PlayerStatusType,
};

impl Player {
    /// React to a completed permanent transfer. Resets stats history,
    /// clears transient statuses and happiness, installs a fresh contract
    /// and signing plan, and stages a pending signing so the next sim
    /// tick can emit the shock / role-fit / promise events.
    pub fn complete_transfer(&mut self, t: TransferCompletion<'_>) {
        let previous_salary = self.contract.as_ref().map(|c| c.salary);
        let desire_carry = self.snapshot_desire_carry();
        let source_club_reputation = t.from.reputation;
        let source_league_reputation = t.selling_league_reputation;
        if t.loan_buyout {
            self.on_loan_buyout(t.to, t.fee, t.date);
        } else {
            self.on_transfer(t.history_source, t.to, t.fee, t.date);
        }
        self.sold_from = Some((t.selling_club_id, t.fee));
        self.reset_on_club_change();
        // No more market-state to track once they're under contract.
        self.clear_free_agent_state();
        self.install_permanent_contract_with_terms(
            t.date,
            t.to.reputation,
            t.buying_league_reputation,
            t.agreed_wage,
            t.personal_terms.as_ref(),
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
            had_return_home_desire: desire_carry.return_home,
            had_european_desire: desire_carry.european,
            had_libertadores_desire: desire_carry.libertadores,
            source_club_reputation,
            source_league_reputation,
            dest_position_depth_rank: None,
            source_is_rival: t.source_is_rival,
        });
        // Stamp the move on the decisions register — the "From → To" row a
        // demotion/listing already reads like, now closing the loop on the
        // actual sale. A fee-bearing deal reads as a permanent transfer; a
        // free-of-charge club-to-club move reads as a free transfer.
        if t.record_decision {
            let decision = if t.fee > 0.0 {
                "dec_transfer_completed"
            } else {
                "dec_free_transfer_completed"
            };
            self.decision_history
                .add_move(t.date, &t.from.name, &t.to.name, t.fee, decision);
        }
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
        to: &TeamInfo,
        date: NaiveDate,
        buying_club_id: u32,
        buying_league_reputation: u16,
        agreed_wage: Option<u32>,
    ) {
        let previous_salary = self.contract.as_ref().map(|c| c.salary);
        let desire_carry = self.snapshot_desire_carry();
        // Source rep derived from the player's most recent senior club
        // *before* `on_free_agent_signing` mutates the history rows. A
        // released prospect coming off a small-club spell needs that
        // recent rep so the source-aware dream-move gate can fire when
        // they sign for a giant — without it every free-agent signing
        // reads as zero source and fails closed. League rep isn't kept
        // per-row here so the gate evaluates club-rep gap only for free
        // agents; that matches the "either-axis" rule on
        // [`Player::is_source_aware_step_up`].
        let history_source_club_rep = self
            .statistics_history
            .last_known_senior_team_reputation()
            .unwrap_or(0);
        self.on_free_agent_signing(to, date);
        self.reset_on_club_change();
        self.clear_free_agent_state();
        self.install_permanent_contract(date, to.reputation, buying_league_reputation, agreed_wage);
        self.plan = Some(PlayerPlan::from_signing(self.age(date), 0.0, date));
        self.pending_signing = Some(PendingSigning {
            previous_salary,
            fee: 0.0,
            is_loan: false,
            destination_club_id: buying_club_id,
            had_return_home_desire: desire_carry.return_home,
            had_european_desire: desire_carry.european,
            had_libertadores_desire: desire_carry.libertadores,
            // Free-agent signing has no live source club; we anchor to
            // the most recent senior club from career history so the
            // source-aware dream-move gate stays meaningful. Unknown
            // history still leaves both reps at 0, which the gate
            // already fails closed for.
            source_club_reputation: history_source_club_rep,
            source_league_reputation: 0,
            dest_position_depth_rank: None,
            // A free agent arrives unattached — whatever badge he wore
            // before release, he didn't come STRAIGHT from the rival.
            source_is_rival: false,
        });
        // A free-agent capture is a genuine recruitment decision — there is
        // no selling club, so the register shows the club he joined.
        self.decision_history.add(
            date,
            to.name.clone(),
            "dec_free_agent_signed".to_string(),
            String::new(),
        );
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
        let desire_carry = self.snapshot_desire_carry();
        let source_club_reputation = l.from.reputation;
        let source_league_reputation = l.parent_league_reputation;
        self.on_loan(l.history_source, l.to, l.loan_fee, l.date);
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
            had_return_home_desire: desire_carry.return_home,
            had_european_desire: desire_carry.european,
            had_libertadores_desire: desire_carry.libertadores,
            source_club_reputation,
            source_league_reputation,
            dest_position_depth_rank: None,
            // Loans between open rivals essentially don't happen — the
            // parent won't strengthen the enemy — so the borrowed
            // arrival never carries the rival-past stigma.
            source_is_rival: false,
        });
        // Record the loan departure on the register (parent → borrower). A
        // loan fee, when one was paid, rides alongside the route; most youth
        // loans carry none and simply show the two clubs.
        self.decision_history.add_move(
            l.date,
            &l.from.name,
            &l.to.name,
            l.loan_fee,
            "dec_loan_started",
        );
    }

    /// Stage a `PendingSigning` after a manual web-driven move. Mirrors
    /// what the AI `complete_transfer` / `complete_loan` / free-signing
    /// paths do at the end of their flow, so the very next sim tick
    /// fires the same DreamMove / AmbitionShock / SalaryShock / RoleMismatch
    /// / FeelingIsolated events for a user-created move. Caller is
    /// responsible for clearing happiness, installing the new contract,
    /// and recording career history beforehand — this helper *only*
    /// captures the transient pending-signing state.
    ///
    /// `previous_salary` is read straight from the player's existing
    /// contract (the source contract, pre-installation of the destination
    /// contract). The desire-carry snapshot is read from the player's
    /// happiness — call this BEFORE `player.happiness.clear()`, otherwise
    /// the carry flags will all be false and the on-completion satisfaction
    /// events won't fire.
    pub fn stage_manual_pending_signing(
        &mut self,
        destination_club_id: u32,
        fee: f64,
        is_loan: bool,
        source_club_reputation: u16,
        source_league_reputation: u16,
        dest_position_depth_rank: Option<u8>,
    ) {
        let previous_salary = self.contract.as_ref().map(|c| c.salary);
        let desire_carry = self.snapshot_desire_carry();
        self.pending_signing = Some(PendingSigning {
            previous_salary,
            fee,
            is_loan,
            destination_club_id,
            had_return_home_desire: desire_carry.return_home,
            had_european_desire: desire_carry.european,
            had_libertadores_desire: desire_carry.libertadores,
            source_club_reputation,
            source_league_reputation,
            dest_position_depth_rank,
            // Manual web moves don't resolve the buying club's rivals
            // list at this layer; the cold-shoulder beat is an AI-flow
            // nuance, not a user-move guarantee.
            source_is_rival: false,
        });
    }

    /// Snapshot which career-desire moods the player carried into this
    /// transfer. Read by `complete_*` before `reset_on_club_change` wipes
    /// `happiness`, so `process_transfer_shock` can fire the matching
    /// satisfaction events at the new club.
    fn snapshot_desire_carry(&self) -> DesireCarry {
        DesireCarry {
            return_home: self
                .happiness
                .has_recent_event(&HappinessEventType::WantsReturnHome, 180),
            european: self
                .happiness
                .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 180),
            libertadores: self
                .happiness
                .has_recent_event(&HappinessEventType::WantsCopaLibertadores, 180),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DesireCarry {
    return_home: bool,
    european: bool,
    libertadores: bool,
}

impl Player {
    /// Canonical transient-state reset for the end of a club spell. Every
    /// path that moves the player off a roster — the AI completion
    /// methods above, the released-to-free-agents sweep, and the manual
    /// web transfer / loan / release actions — must run this so transfer
    /// statuses (`Lst` / `Loa` / `Frt` / `Req` / `Unh` / ...) and the
    /// unhappiness they were attached to never outlive the club they
    /// described. Keep a single list here: parallel copies drift.
    ///
    /// Callers that branch on `Frt` (the "released early" marker) must
    /// read it BEFORE calling — this consumes it.
    pub fn reset_on_club_change(&mut self) {
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
        // Squad social view belongs to the previous club's roster; the
        // new club's weekly pre-tick will rebuild it on its next Monday.
        self.squad_social_view = None;
        // Transfer-request reasons evaporate when the move actually
        // happens — the wish was granted.
        self.transfer_request_reasons.clear();
        // The availability-market diagnosis described the previous club's
        // market read — it must not outlive the spell it belonged to.
        self.clear_availability_state();
        // Force-selection is the previous manager's pin — the new
        // club's manager hasn't expressed a preference yet, so the
        // flag mustn't survive the move.
        self.is_force_match_selection = false;
        // The release reason described the previous spell's exit — once
        // the player has actually moved on (signed elsewhere, or swept
        // into the free-agent pool after the sweep has read it) it must
        // not linger onto the next spell.
        self.release_reason = None;
        // A staged pre-contract is consumed the moment the player changes
        // clubs — whether the move IS that pre-contract (he just joined
        // the agreed club) or any other exit (sold, swept to the pool).
        // Either way the agreement must not survive onto the new spell.
        self.pending_pre_contract = None;
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
        self.install_permanent_contract_with_terms(
            date,
            buying_club_reputation,
            buying_league_reputation,
            agreed_wage,
            None,
        );
    }

    /// Variant of [`Self::install_permanent_contract`] that honours an
    /// agreed [`PersonalTermsOffer`]. When `personal_terms` is `Some`,
    /// each populated field overrides the corresponding compute-from-
    /// context default:
    ///
    ///   - `contract_years` → contract length (replaces age band)
    ///   - `annual_wage` → salary (replaces calculator output)
    ///   - `signing_bonus` → adds a `SigningBonus` contract bonus on top
    ///     of any bonus the profile package would already install
    ///   - `release_clause_fee` → writes a `MinimumFeeRelease` clause
    ///   - `squad_status_promise` → sets the contract's `squad_status`
    ///     so the role promise sticks (Day 1 squad role)
    ///
    /// Unset fields fall through to the existing defaults — this
    /// preserves behaviour for manual UI moves and tests that don't
    /// stage a structured terms package.
    pub fn install_permanent_contract_with_terms(
        &mut self,
        date: NaiveDate,
        buying_club_reputation: u16,
        buying_league_reputation: u16,
        agreed_wage: Option<u32>,
        personal_terms: Option<&PersonalTermsOffer>,
    ) {
        let age = self.age(date);
        let years = personal_terms
            .and_then(|t| t.contract_years)
            .unwrap_or_else(|| {
                if age < 24 {
                    5
                } else if age < 28 {
                    4
                } else if age < 32 {
                    3
                } else {
                    2
                }
            });
        let expiry = date
            .checked_add_signed(Duration::days(years as i64 * 365))
            .unwrap_or(date);

        // Resolved wage: structured terms beat the loose `agreed_wage`
        // argument; both beat the calculator fallback.
        let resolved_wage = personal_terms
            .and_then(|t| t.annual_wage)
            .or(agreed_wage)
            .unwrap_or_else(|| {
                let club_score = (buying_club_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
                WageCalculator::expected_annual_wage(
                    self,
                    age,
                    club_score,
                    buying_league_reputation,
                )
            });
        let mut contract = PlayerClubContract::new(resolved_wage, expiry);
        // Stamp the contract with its real start date so the wage-envy
        // grace window, yearly-rise anniversary, and loyalty-bonus
        // anchors all reason from the move date. Without this, the
        // monthly wage audit treats a fresh transfer as "ancient
        // contract" and a youth-graduation salary as outside the grace
        // window — both fire SalaryGapNoticed within weeks of the
        // signing.
        contract.started = Some(date);

        // Install a profile-appropriate set of bonuses + clauses so
        // transfer-completed contracts feel like the same market as
        // renewals. Without this every transfer signs a bare
        // salary/years deal and never pays a goal/clean-sheet/loyalty
        // bonus.
        install_transfer_package(&mut contract, self, age, buying_club_reputation, date);

        // Honour the staged personal-terms additions.
        if let Some(terms) = personal_terms {
            // Signing bonus stacks: if the profile already wrote one,
            // the explicit personal-terms amount replaces it so the
            // negotiated number sticks.
            if let Some(amount) = terms.signing_bonus {
                contract
                    .bonuses
                    .retain(|b| !matches!(b.bonus_type, ContractBonusType::SigningBonus));
                contract.bonuses.push(ContractBonus::new(
                    amount as i32,
                    ContractBonusType::SigningBonus,
                ));
            }
            // Release clause replaces any auto-installed one (the
            // negotiation explicitly settled on this number).
            if let Some(fee) = terms.release_clause_fee {
                contract
                    .clauses
                    .retain(|c| !matches!(c.bonus_type, ContractClauseType::MinimumFeeRelease));
                contract.clauses.push(ContractClause::new(
                    fee as i32,
                    ContractClauseType::MinimumFeeRelease,
                ));
            }
            // Role promise — written onto the contract as Day-1 squad
            // status. Without this the player walks in as `NotYetSet`
            // and the next role-fit tick may downgrade them despite
            // the buyer's promise.
            if let Some(promise) = terms.squad_status_promise {
                let promised = match promise {
                    PromisedSquadStatus::KeyPlayer => PlayerSquadStatus::KeyPlayer,
                    PromisedSquadStatus::FirstTeamRegular => PlayerSquadStatus::FirstTeamRegular,
                    PromisedSquadStatus::FirstTeamSquadRotation => {
                        PlayerSquadStatus::FirstTeamSquadRotation
                    }
                    PromisedSquadStatus::HotProspectForTheFuture => {
                        PlayerSquadStatus::HotProspectForTheFuture
                    }
                };
                contract.squad_status = promised.clone();
                // Bind the promise for a season so the monthly CA-rank pass
                // can't quietly demote below it (see
                // PlayerClubContract::promised_squad_status). A broken promise
                // then surfaces as real playing-time unhappiness instead of
                // being silently absorbed by the recompute.
                let until = date.checked_add_signed(Duration::days(365)).unwrap_or(date);
                contract.promised_squad_status = Some((promised, until));
            }
        }

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
    date: NaiveDate,
) {
    let salary = contract.salary;
    let pos = player.position();
    let rep = player.player_attributes.current_reputation;
    let ability = player.player_attributes.current_ability;
    // Contract shape is a club decision — the prospect profile reads the
    // observable ceiling, never the hidden biological PA.
    let potential = PotentialEstimator::observable_ceiling(player, date);
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

#[cfg(test)]
mod free_agent_source_aware_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::{CurrentSeasonEntry, PlayerStatistics};
    use crate::league::Season;
    use crate::shared::fullname::FullName;
    use crate::{
        HappinessEventType, PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    /// Fixtures for the free-agent source-aware dream-move tests.
    /// Wrapped in a unit struct per project convention.
    struct FreeAgentFixtures;

    impl FreeAgentFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn dest(rep: u16) -> TeamInfo {
            TeamInfo {
                name: "Dest".to_string(),
                slug: "dest".to_string(),
                reputation: rep,
                league_name: String::new(),
                league_slug: String::new(),
            }
        }

        fn person(ambition: f32) -> PersonAttributes {
            PersonAttributes {
                adaptability: 10.0,
                ambition,
                controversy: 10.0,
                loyalty: 10.0,
                pressure: 10.0,
                professionalism: 10.0,
                sportsmanship: 10.0,
                temperament: 10.0,
                consistency: 10.0,
                important_matches: 10.0,
                dirtiness: 10.0,
            }
        }

        fn player(age: u8, ambition: f32, world_rep: i16) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.world_reputation = world_rep;
            attrs.current_reputation = world_rep;
            attrs.current_ability = 130;
            attrs.potential_ability = 140;
            let today = Self::d(2026, 4, 26);
            let birth = today
                .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
                .unwrap();
            PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("X".into(), "Y".into()))
                .birth_date(birth)
                .country_id(1)
                .attributes(Self::person(ambition))
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Striker,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap()
        }

        /// Attach a single most-recent "released by club at reputation `rep`"
        /// entry so the helper has a senior anchor to read off.
        fn attach_released_from(player: &mut Player, rep: u16) {
            let entry = CurrentSeasonEntry {
                team_name: "previous".to_string(),
                team_slug: "previous".to_string(),
                team_reputation: rep,
                league_name: String::new(),
                league_slug: String::new(),
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                joined_date: Self::d(2025, 8, 1),
                departed_date: Some(Self::d(2026, 4, 1)),
                seq_id: 1,
            };
            player.statistics_history.current.push(entry);
        }

        fn count(p: &Player, ev: HappinessEventType) -> usize {
            p.happiness
                .recent_events
                .iter()
                .filter(|e| e.event_type == ev)
                .count()
        }
    }

    /// A released small-club prospect (last senior rep ~1500) signing
    /// for Real Madrid (dest rep ~9500) must clear the source-aware
    /// gate via the history anchor and fire `DreamMove` on the next
    /// transfer-shock tick. This is the case the original gate failed
    /// closed for — free-agent source rep used to be zero.
    #[test]
    fn released_small_club_prospect_signs_real_madrid_emits_dream_move() {
        let mut p = FreeAgentFixtures::player(22, 15.0, 2000);
        FreeAgentFixtures::attach_released_from(&mut p, 1500);
        let date = FreeAgentFixtures::d(2026, 6, 1);
        p.complete_free_agent_signing(&FreeAgentFixtures::dest(9500), date, 42, 9500, Some(80_000));
        p.process_transfer_shock(date, 0.95, 9500, "es", None);
        assert!(
            FreeAgentFixtures::count(&p, HappinessEventType::DreamMove) >= 1,
            "elite free-agent landing must fire DreamMove once history anchors the source rep"
        );
    }

    /// A high-rep veteran free agent dropping into a mid-tier club is
    /// NOT a dream move — the destination is below the elite gate AND
    /// no source-aware step up exists.
    #[test]
    fn high_rep_veteran_free_agent_to_mid_tier_club_does_not_emit_dream_move() {
        let mut p = FreeAgentFixtures::player(33, 12.0, 7500);
        FreeAgentFixtures::attach_released_from(&mut p, 8000);
        let date = FreeAgentFixtures::d(2026, 6, 1);
        p.complete_free_agent_signing(
            &FreeAgentFixtures::dest(4500),
            date,
            42,
            4500,
            Some(120_000),
        );
        p.process_transfer_shock(date, 0.45, 4500, "it", None);
        assert_eq!(
            FreeAgentFixtures::count(&p, HappinessEventType::DreamMove),
            0,
            "step-down free-agent landing must not fire DreamMove"
        );
    }

    /// Free agent with NO career history (typical of generated players
    /// fresh out of an academy and immediately released) must not
    /// overfire — the helper returns None, the staged source rep stays
    /// zero, and the source-aware gate fails closed.
    #[test]
    fn free_agent_with_unknown_history_does_not_overfire() {
        let mut p = FreeAgentFixtures::player(22, 17.0, 2000);
        // Deliberately no attach_released_from — empty history.
        assert!(p.statistics_history.current.is_empty());
        assert!(p.statistics_history.items.is_empty());
        let date = FreeAgentFixtures::d(2026, 6, 1);
        p.complete_free_agent_signing(&FreeAgentFixtures::dest(9500), date, 42, 9500, Some(80_000));
        let pending = p.pending_signing.as_ref().expect("pending signing staged");
        assert_eq!(
            pending.source_club_reputation, 0,
            "unknown history must leave source rep at zero"
        );
        // Run the shock pipeline — the source-aware gate must keep
        // silent because the source data is unknown.
        p.process_transfer_shock(date, 0.95, 9500, "es", None);
        assert_eq!(
            FreeAgentFixtures::count(&p, HappinessEventType::DreamMove),
            0,
            "unknown free-agent history must not over-fire DreamMove"
        );
    }

    /// Sanity check: the helper actually returns the rep stamped on a
    /// frozen prior season too, not just the live `current` row.
    #[test]
    fn frozen_history_only_still_anchors_source_rep() {
        use crate::club::player::statistics::PlayerStatisticsHistoryItem;
        let mut p = FreeAgentFixtures::player(24, 14.0, 3000);
        p.statistics_history
            .items
            .push(PlayerStatisticsHistoryItem {
                season: Season::new(2024),
                team_name: "previous".to_string(),
                team_slug: "previous".to_string(),
                team_reputation: 2_000,
                league_name: String::new(),
                league_slug: String::new(),
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                seq_id: 7,
            });
        let date = FreeAgentFixtures::d(2026, 6, 1);
        p.complete_free_agent_signing(&FreeAgentFixtures::dest(9500), date, 42, 9500, Some(80_000));
        let pending = p.pending_signing.as_ref().expect("pending signing staged");
        assert_eq!(pending.source_club_reputation, 2_000);
    }

    /// Fixtures + assertions for the decisions-register rows the transfer
    /// completion paths now stamp. Wrapped in a unit struct per convention.
    struct MoveFixtures;

    impl MoveFixtures {
        fn team(name: &str) -> TeamInfo {
            TeamInfo {
                name: name.to_string(),
                slug: name.to_ascii_lowercase(),
                reputation: 5_000,
                league_name: String::new(),
                league_slug: String::new(),
            }
        }

        fn dec_count(p: &Player, key: &str) -> usize {
            p.decision_history
                .items
                .iter()
                .filter(|d| d.decision == key)
                .count()
        }

        fn completion<'a>(
            from: &'a TeamInfo,
            to: &'a TeamInfo,
            fee: f64,
            date: NaiveDate,
            record_decision: bool,
        ) -> TransferCompletion<'a> {
            TransferCompletion {
                from,
                history_source: from,
                to,
                fee,
                date,
                selling_club_id: 10,
                buying_club_id: 20,
                agreed_wage: None,
                buying_league_reputation: 5_000,
                selling_league_reputation: 5_000,
                source_is_rival: false,
                record_sell_on: None,
                personal_terms: None,
                record_decision,
                loan_buyout: false,
            }
        }
    }

    /// A fee-bearing permanent transfer stamps a `dec_transfer_completed`
    /// row whose movement carries both clubs and the fee.
    #[test]
    fn complete_transfer_records_permanent_transfer_decision() {
        let mut p = FreeAgentFixtures::player(24, 12.0, 3000);
        let from = MoveFixtures::team("Old FC");
        let to = MoveFixtures::team("New FC");
        let date = FreeAgentFixtures::d(2026, 7, 1);
        p.complete_transfer(MoveFixtures::completion(&from, &to, 2_500_000.0, date, true));

        assert_eq!(MoveFixtures::dec_count(&p, "dec_transfer_completed"), 1);
        let row = p
            .decision_history
            .items
            .iter()
            .find(|d| d.decision == "dec_transfer_completed")
            .unwrap();
        assert!(row.movement.contains("Old FC"), "movement: {}", row.movement);
        assert!(row.movement.contains("New FC"), "movement: {}", row.movement);
        assert!(row.movement.contains('→'), "movement: {}", row.movement);
    }

    /// A zero-fee club-to-club move reads as a free transfer, not a
    /// permanent one.
    #[test]
    fn complete_transfer_free_move_records_free_transfer_decision() {
        let mut p = FreeAgentFixtures::player(24, 12.0, 3000);
        let from = MoveFixtures::team("Old FC");
        let to = MoveFixtures::team("New FC");
        let date = FreeAgentFixtures::d(2026, 7, 1);
        p.complete_transfer(MoveFixtures::completion(&from, &to, 0.0, date, true));

        assert_eq!(MoveFixtures::dec_count(&p, "dec_free_transfer_completed"), 1);
        assert_eq!(MoveFixtures::dec_count(&p, "dec_transfer_completed"), 0);
    }

    /// The loan-buyout path reuses `complete_transfer` to flip ownership but
    /// narrates itself with `dec_loan_buyout`; `record_decision: false` must
    /// keep the generic transfer row off so one event reads as one decision.
    #[test]
    fn complete_transfer_suppresses_decision_when_flagged() {
        let mut p = FreeAgentFixtures::player(24, 12.0, 3000);
        let from = MoveFixtures::team("Old FC");
        let to = MoveFixtures::team("New FC");
        let date = FreeAgentFixtures::d(2026, 7, 1);
        p.complete_transfer(MoveFixtures::completion(&from, &to, 1_000_000.0, date, false));

        assert_eq!(MoveFixtures::dec_count(&p, "dec_transfer_completed"), 0);
        assert_eq!(MoveFixtures::dec_count(&p, "dec_free_transfer_completed"), 0);
    }

    /// A free-agent capture is a genuine recruitment decision — one
    /// `dec_free_agent_signed` row naming the club he joined.
    #[test]
    fn complete_free_agent_signing_records_signed_decision() {
        let mut p = FreeAgentFixtures::player(24, 12.0, 3000);
        let date = FreeAgentFixtures::d(2026, 7, 1);
        p.complete_free_agent_signing(&FreeAgentFixtures::dest(5000), date, 42, 5000, Some(60_000));

        assert_eq!(MoveFixtures::dec_count(&p, "dec_free_agent_signed"), 1);
    }
}
