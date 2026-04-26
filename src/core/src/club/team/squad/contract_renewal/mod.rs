use crate::club::player::calculators::{ContractValuation, ValuationContext};
use crate::club::player::mailbox::handlers::contract_proposal::{
    ProcessContractHandler, RENEWAL_REJECTED_LABEL,
};
use crate::club::player::mailbox::RejectionReason;
use crate::club::player::player::Player;
use crate::utils::DateUtils;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerSquadStatus, PlayerStatusType,
    Team,
};
use chrono::NaiveDate;

/// Minimum gap between proactive offers when the previous one hasn't
/// actually been turned down yet. Set above 30 so the proactive (month
/// start) and reactive (daily) paths can't chain a fresh offer every
/// single month — clubs typically regroup for longer between attempts.
const RENEWAL_COOLDOWN_DAYS: i64 = 60;
/// Longer cooldown after the player rejects. Real clubs don't hammer the
/// same player every month with the same deal — they regroup, raise, and
/// try again after a while.
const RENEWAL_COOLDOWN_AFTER_REJECT_DAYS: i64 = 120;
/// Hard cap on how many times a proactive offer is made within a year.
/// After three flat refusals the club normally gives up — unless a
/// final-year or market-pressure override fires.
const MAX_RENEWAL_ATTEMPTS_PER_YEAR: usize = 3;
/// Days-to-expiry at which Bosman rules start biting: any longer and a
/// rival club can agree a pre-contract. Below this threshold the club
/// abandons the attempts cap and pushes harder.
const BOSMAN_PRESSURE_DAYS: i64 = 180;
/// Final-month panic. No cap, maximum sweeteners.
const FINAL_PANIC_DAYS: i64 = 30;
const DECISION_LABEL: &str = "dec_contract_renewal_offered";

pub struct ContractRenewalManager;

impl ContractRenewalManager {
    /// Walk the main team, deliver renewal proposals to valuable players
    /// whose contracts are approaching expiry. Deterministic — no AI call.
    ///
    /// Runs before the monthly TransferListManager so valuable players have
    /// already been offered a renewal by the time the listing AI evaluates
    /// them. This prevents the listing AI from inventing "contract expiring"
    /// as a reason to sell a player the club actually wants to keep.
    pub fn run(teams: &mut [Team], main_idx: usize, date: NaiveDate) {
        Self::run_with_budget(teams, main_idx, date, None, 5_000)
    }

    /// Variant used by the club layer where wage budget and league
    /// reputation are available. Falls through to the same logic as
    /// `run` but cannot exceed the supplied wage budget when offering.
    pub fn run_with_budget(
        teams: &mut [Team],
        main_idx: usize,
        date: NaiveDate,
        wage_budget: Option<u32>,
        league_reputation: u16,
    ) {
        // Apply team-level clause helpers once per pass before generating
        // new offers — match-highest-earner can change the snapshot used
        // by the renewal AI, and optional extensions can take a player out
        // of the candidate set entirely.
        Self::apply_team_level_clauses(&mut teams[main_idx], date);

        let (coach_name, negotiation_skill, judging_ability) =
            Self::resolve_staff(&teams[main_idx]);
        let team_rep_factor = teams[main_idx].reputation.overall_score();

        // Mutable: each accepted offer reserves its salary delta against
        // current_bill so subsequent candidates in this same monthly pass
        // see the budget already spent. Without this the loop could blow
        // the wage budget by approving five star offers in parallel.
        let mut structure = WageStructureSnapshot::from_team(&teams[main_idx]);
        let candidates = Self::collect_candidates(&teams[main_idx], date);
        let mut match_highest_used = false;

        for candidate in candidates {
            let attempts = teams[main_idx]
                .players
                .players
                .iter()
                .find(|p| p.id == candidate.player_id)
                .map(|p| {
                    p.decision_history
                        .items
                        .iter()
                        .filter(|d| {
                            d.decision == DECISION_LABEL && (date - d.date).num_days() < 365
                        })
                        .count()
                })
                .unwrap_or(0);

            let over_cap = attempts >= MAX_RENEWAL_ATTEMPTS_PER_YEAR;
            if over_cap && !candidate.override_attempts_cap {
                continue;
            }

            let current_salary = teams[main_idx]
                .players
                .players
                .iter()
                .find(|p| p.id == candidate.player_id)
                .and_then(|p| p.contract.as_ref().map(|c| c.salary))
                .unwrap_or(0);

            let built = Self::build_offer(
                &teams[main_idx],
                candidate.player_id,
                negotiation_skill,
                judging_ability,
                date,
                attempts,
                team_rep_factor,
                league_reputation,
                wage_budget,
                &structure,
                &candidate,
                match_highest_used,
            );
            let mut proposal = match built {
                Some(p) => p,
                None => continue,
            };

            // Match-highest-earner is genuinely elite — only one offer per
            // monthly pass may carry it, and only if the candidate is a
            // KeyPlayer. Without this guard a wave of "star" candidates
            // could each receive the clause and ratchet the wage hierarchy
            // upwards in one go.
            if proposal.match_highest_earner {
                if match_highest_used {
                    proposal.match_highest_earner = false;
                } else {
                    match_highest_used = true;
                }
            }

            // Reserve the salary delta against the running wage bill so the
            // next candidate's budget check sees this offer as already
            // spent. Conservative: assume the player will accept.
            let salary_delta = proposal.salary.saturating_sub(current_salary);
            structure.current_bill = structure.current_bill.saturating_add(salary_delta);
            // Lift top_earner so a follow-on KeyPlayer offer doesn't
            // immediately leapfrog the just-promised one.
            if proposal.salary > structure.top_earner {
                structure.top_earner = proposal.salary;
            }

            if let Some(player) = teams[main_idx]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == candidate.player_id)
            {
                let movement = format!(
                    "{}y · ${}/y",
                    proposal.years,
                    crate::utils::FormattingUtils::format_money(proposal.salary as f64)
                );
                player.decision_history.add(
                    date,
                    movement,
                    DECISION_LABEL.to_string(),
                    coach_name.clone(),
                );
                player.mailbox.push(PlayerMessage {
                    message_type: PlayerMessageType::ContractProposal(proposal),
                });
            }
        }
    }

    /// Apply the clause helpers that need team-level context once per
    /// monthly renewal pass.
    ///
    /// `MatchHighestEarner`: lift the holder's wage to match the current
    /// top earner *excluding self* — re-running with the same top is a
    /// no-op because the helper bails when `top <= self.salary`.
    ///
    /// `OptionalContractExtensionByClub`: club exercises the option only
    /// when the player is still a first-team contributor with
    /// >= 1 appearance per ~5 weeks of the contract window. Otherwise the
    /// option lapses and the clause is consumed.
    fn apply_team_level_clauses(team: &mut Team, date: NaiveDate) {
        // Snapshot ids + salaries so we can compute "top excluding self"
        // without holding two mutable borrows simultaneously.
        let snapshot: Vec<(u32, u32)> = team
            .players
            .players
            .iter()
            .filter_map(|p| p.contract.as_ref().map(|c| (p.id, c.salary)))
            .collect();
        let global_top = snapshot.iter().map(|(_, s)| *s).max().unwrap_or(0);

        for player in team.players.players.iter_mut() {
            // Compute top earner excluding this player — without it the
            // helper would be a fixed-point on the player's own salary
            // and never lift them.
            let top_excl = if let Some(c) = player.contract.as_ref() {
                if c.salary >= global_top {
                    snapshot
                        .iter()
                        .filter(|(id, _)| *id != player.id)
                        .map(|(_, s)| *s)
                        .max()
                        .unwrap_or(0)
                } else {
                    global_top
                }
            } else {
                continue;
            };

            // Cheap exit: only act on contracts that actually carry a
            // team-level clause.
            let has_team_clause = player
                .contract
                .as_ref()
                .map(|c| {
                    c.clauses.iter().any(|cl| {
                        matches!(
                            cl.bonus_type,
                            crate::ContractClauseType::MatchHighestEarner
                                | crate::ContractClauseType::OptionalContractExtensionByClub
                        )
                    })
                })
                .unwrap_or(false);
            if !has_team_clause {
                continue;
            }

            // Decide whether the optional extension is worth exercising
            // BEFORE we mutate the contract. Heuristic: only for first-team
            // contributors who started enough games this season.
            let should_extend = {
                let played_share = (player.statistics.played as f32)
                    .max(player.statistics.played_subs as f32 / 2.0);
                let is_contributor = matches!(
                    player.contract.as_ref().map(|c| &c.squad_status),
                    Some(crate::PlayerSquadStatus::KeyPlayer)
                        | Some(crate::PlayerSquadStatus::FirstTeamRegular)
                        | Some(crate::PlayerSquadStatus::FirstTeamSquadRotation)
                ) && played_share >= 8.0;
                is_contributor
            };

            if let Some(c) = player.contract.as_mut() {
                let _ = c.try_apply_match_highest_earner(top_excl);
                if should_extend {
                    let _ = c.exercise_optional_extension();
                } else {
                    // Drop unused options so they don't linger past the
                    // expiry date and pollute future audits.
                    let final_year = (c.expiration - date).num_days() <= 60;
                    if final_year {
                        c.clauses.retain(|cl| {
                            !matches!(
                                cl.bonus_type,
                                crate::ContractClauseType::OptionalContractExtensionByClub
                            )
                        });
                    }
                }
            }
        }
    }

    fn resolve_staff(team: &Team) -> (String, u8, u8) {
        let coach_name = team.staffs.head_coach().full_name.to_string();

        let resolver = team
            .staffs
            .responsibility
            .contract_renewal
            .handle_first_team_contracts
            .and_then(|id| team.staffs.find(id));

        let (negotiation, judging) = match resolver {
            Some(staff) => (
                staff.staff_attributes.mental.man_management,
                staff.staff_attributes.knowledge.judging_player_ability,
            ),
            None => {
                let hc = team.staffs.head_coach();
                (
                    hc.staff_attributes.mental.man_management,
                    hc.staff_attributes.knowledge.judging_player_ability,
                )
            }
        };

        (coach_name, negotiation, judging)
    }

    fn collect_candidates(team: &Team, date: NaiveDate) -> Vec<RenewalCandidate> {
        team.players
            .players
            .iter()
            .filter_map(|player| Self::evaluate(player, date))
            .collect()
    }

    fn evaluate(player: &Player, date: NaiveDate) -> Option<RenewalCandidate> {
        if player.is_on_loan() {
            return None;
        }

        let contract = player.contract.as_ref()?;
        let days_remaining = (contract.expiration - date).num_days();
        if days_remaining <= 0 {
            return None;
        }

        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Req)
            || statuses.contains(&PlayerStatusType::Lst)
            || statuses.contains(&PlayerStatusType::Frt)
        {
            return None;
        }

        let has_market_interest = statuses.iter().any(|s| {
            matches!(
                s,
                PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
            )
        });

        let effective_status = Self::effective_squad_status(player, &contract.squad_status);
        let threshold = Self::renewal_threshold_days(&effective_status);
        let final_panic = days_remaining <= FINAL_PANIC_DAYS;
        let bosman_pressure = days_remaining <= BOSMAN_PRESSURE_DAYS && has_market_interest;

        if days_remaining > threshold && !final_panic && !bosman_pressure {
            return None;
        }

        let override_cap = final_panic || bosman_pressure;

        if !override_cap && Self::recently_offered(player, date) {
            return None;
        }

        Some(RenewalCandidate {
            player_id: player.id,
            effective_status,
            has_market_interest,
            final_panic,
            bosman_pressure,
            override_attempts_cap: override_cap,
            months_remaining: (days_remaining / 30).max(0) as i32,
        })
    }

    /// Effective squad status used for renewal decisions. A player on a
    /// purple-patch run gets treated one tier up — real clubs rush to tie
    /// down in-form players regardless of their formal role. Same engine
    /// for every player; the boost just needs a strong, universal signal.
    fn effective_squad_status(player: &Player, base: &PlayerSquadStatus) -> PlayerSquadStatus {
        let form = player.load.form_rating;
        if form < 7.5 {
            return base.clone();
        }
        match base {
            PlayerSquadStatus::FirstTeamRegular => PlayerSquadStatus::KeyPlayer,
            PlayerSquadStatus::FirstTeamSquadRotation => PlayerSquadStatus::FirstTeamRegular,
            PlayerSquadStatus::MainBackupPlayer => PlayerSquadStatus::FirstTeamSquadRotation,
            PlayerSquadStatus::DecentYoungster => PlayerSquadStatus::HotProspectForTheFuture,
            PlayerSquadStatus::NotNeeded | PlayerSquadStatus::NotYetSet => {
                PlayerSquadStatus::MainBackupPlayer
            }
            _ => base.clone(),
        }
    }

    fn renewal_threshold_days(squad_status: &PlayerSquadStatus) -> i64 {
        match squad_status {
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular => 540,
            PlayerSquadStatus::FirstTeamSquadRotation
            | PlayerSquadStatus::HotProspectForTheFuture => 365,
            PlayerSquadStatus::MainBackupPlayer | PlayerSquadStatus::DecentYoungster => 180,
            // NotNeeded / unset still get a 90-day window. Real clubs at
            // least consider every expiring contract before letting a
            // player walk on a free — listing is a separate decision.
            _ => 90,
        }
    }

    /// Two-tier cooldown. A proactive offer uses the short window; a
    /// rejected offer gets a much longer one before we come back with a
    /// revised deal.
    fn recently_offered(player: &Player, date: NaiveDate) -> bool {
        let last = player.decision_history.items.iter().rev().find(|d| {
            d.decision == DECISION_LABEL || d.decision == RENEWAL_REJECTED_LABEL
        });
        match last {
            Some(d) if d.decision == RENEWAL_REJECTED_LABEL => {
                (date - d.date).num_days() < RENEWAL_COOLDOWN_AFTER_REJECT_DAYS
            }
            Some(d) => (date - d.date).num_days() < RENEWAL_COOLDOWN_DAYS,
            None => false,
        }
    }

    fn build_offer(
        team: &Team,
        player_id: u32,
        negotiation_skill: u8,
        judging_ability: u8,
        date: NaiveDate,
        previous_attempts: usize,
        team_rep_factor: f32,
        league_reputation: u16,
        wage_budget: Option<u32>,
        structure: &WageStructureSnapshot,
        candidate: &RenewalCandidate,
        match_highest_already_used: bool,
    ) -> Option<PlayerContractProposal> {
        let player = team.players.find(player_id)?;
        let contract = player.contract.as_ref()?;

        let ability = player.player_attributes.current_ability;
        let age = DateUtils::age(player.birth_date, date);

        let ctx = ValuationContext {
            age,
            club_reputation_score: team_rep_factor,
            league_reputation,
            squad_status: candidate.effective_status.clone(),
            current_salary: contract.salary,
            months_remaining: candidate.months_remaining,
            has_market_interest: candidate.has_market_interest,
        };
        let valuation = ContractValuation::evaluate(player, &ctx);

        // Coach judging-ability variance — within ±15% of the unified target.
        let accuracy = 0.85 + (judging_ability as f32 / 20.0) * 0.30;
        let mut offered = (valuation.expected_wage as f32 * accuracy) as u32;

        // Anchor never below current salary plus a token bump.
        offered = offered
            .max(contract.salary + contract.salary / 20)
            .max(contract.salary + 1);

        // Urgency escalation: +8% per prior rejection, plus a jolt when
        // Bosman / final-year kicks in. Capped by max_acceptable.
        let mut escalation = 1.0 + previous_attempts as f32 * 0.08;
        if candidate.bosman_pressure {
            escalation += 0.10;
        }
        if candidate.final_panic {
            escalation += 0.15;
        }
        if candidate.has_market_interest {
            escalation += 0.05;
        }
        offered = ((offered as f32) * escalation) as u32;

        // Converge toward the player's own ask when we have it, but never
        // beyond max_acceptable.
        if let Some(ask) = &player.pending_contract_ask {
            if ask.desired_salary > offered {
                offered = (offered + ask.desired_salary) / 2;
            }
        }

        // Wage structure protection: only KeyPlayer can break the
        // top-earner ceiling, and only by a small margin. FirstTeamRegular
        // can approach but not exceed; everyone else is capped under it.
        offered = structure.cap_for_status(offered, &candidate.effective_status);

        // FFP / wage-budget gate: never bust it. If the available budget
        // can't even cover the player's acceptance floor, return None —
        // the chairman defers the offer rather than papering over the
        // shortfall with `min_acceptable / 2` (which would generate a
        // proposal the player rejects on sight, wasting an attempt).
        if let Some(budget) = wage_budget {
            let salary_delta = offered.saturating_sub(contract.salary);
            if structure.current_bill + salary_delta > budget {
                let remaining = budget.saturating_sub(structure.current_bill);
                let capped = (contract.salary + remaining).max(contract.salary);
                if capped < valuation.min_acceptable && !candidate.final_panic {
                    // Defer: budget can't fund a credible offer right now.
                    // Final-panic players (last-month expiry) still get
                    // whatever the budget allows — better an insulting bid
                    // than letting them walk for free.
                    return None;
                }
                offered = capped;
            }
        }

        let years = proactive_contract_years(
            age,
            ability,
            &candidate.effective_status,
            negotiation_skill,
            player,
            date,
        );

        let profile = PlayerProfile::classify(player, age, ability, &candidate.effective_status);
        let budget_pressure = wage_budget
            .map(|b| {
                let target_delta = offered.saturating_sub(contract.salary);
                let projected = structure.current_bill + target_delta;
                if b > 0 && projected > (b * 95 / 100) {
                    1.0
                } else if b > 0 && projected > (b * 85 / 100) {
                    0.5
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);

        // Apply budget pressure to base wage: trim it back, compensate
        // with bonuses + clauses below.
        if budget_pressure > 0.5 {
            offered = ((offered as f32) * (1.0 - 0.05 * budget_pressure)) as u32;
            offered = offered.max(contract.salary + 1);
        }

        let mut proposal = PlayerContractProposal::basic(
            offered,
            years,
            negotiation_skill,
            0,
            0,
            None,
        );

        // Profile-driven package decoration. The `urgency` knob blends
        // attempts, market interest, and final-panic into one budget for
        // sweeteners.
        let urgency = previous_attempts
            + if candidate.bosman_pressure { 1 } else { 0 }
            + if candidate.final_panic { 2 } else { 0 }
            + if candidate.has_market_interest { 1 } else { 0 };

        decorate_proposal(
            &mut proposal,
            player,
            age,
            ability,
            &profile,
            &candidate.effective_status,
            urgency,
            budget_pressure,
            structure,
            match_highest_already_used,
        );

        // Honor demanded clauses from the previous rejection.
        if let Some(ask) = &player.pending_contract_ask {
            if ask.demanded_release_clause.is_some() && proposal.release_clause.is_none() {
                proposal.release_clause = ask.demanded_release_clause;
            }
            if let Some(status) = &ask.demanded_status {
                if proposal.squad_status_promise.is_none()
                    && status_rank(status) > status_rank(&contract.squad_status)
                {
                    proposal.squad_status_promise = Some(status.clone());
                }
            }
            if let Some(b) = ask.demanded_signing_bonus {
                if proposal.signing_bonus < b {
                    proposal.signing_bonus = b;
                }
            }
            if matches!(ask.rejection_reason, Some(RejectionReason::ShortContract)) {
                proposal.years = proposal.years.max(ask.desired_years);
            }
        }

        Some(proposal)
    }
}

/// Snapshot of the team's salary distribution. Renewal AI consults this
/// to avoid breaking wage structure when offering a new deal.
#[derive(Debug, Clone)]
pub struct WageStructureSnapshot {
    pub current_bill: u32,
    pub top_earner: u32,
    pub average_first_team: u32,
    pub average_backup: u32,
}

impl WageStructureSnapshot {
    pub fn from_team(team: &Team) -> Self {
        let mut top: u32 = 0;
        let mut bill: u32 = 0;
        let mut first_team_sum: u32 = 0;
        let mut first_team_count: u32 = 0;
        let mut backup_sum: u32 = 0;
        let mut backup_count: u32 = 0;

        for p in team.players.players.iter() {
            let c = match p.contract.as_ref() {
                Some(c) => c,
                None => continue,
            };
            bill = bill.saturating_add(c.salary);
            top = top.max(c.salary);
            match c.squad_status {
                PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular => {
                    first_team_sum = first_team_sum.saturating_add(c.salary);
                    first_team_count += 1;
                }
                PlayerSquadStatus::FirstTeamSquadRotation
                | PlayerSquadStatus::MainBackupPlayer => {
                    backup_sum = backup_sum.saturating_add(c.salary);
                    backup_count += 1;
                }
                _ => {}
            }
        }
        let average_first_team = if first_team_count > 0 {
            first_team_sum / first_team_count
        } else {
            0
        };
        let average_backup = if backup_count > 0 {
            backup_sum / backup_count
        } else {
            0
        };

        Self {
            current_bill: bill,
            top_earner: top,
            average_first_team,
            average_backup,
        }
    }

    /// Cap the offered wage based on squad status. KeyPlayer may exceed
    /// the current top earner by up to 10%; FirstTeamRegular gets to
    /// 95%; rotation/backup are held below the first-team average.
    pub fn cap_for_status(&self, offered: u32, status: &PlayerSquadStatus) -> u32 {
        if self.top_earner == 0 && self.average_first_team == 0 {
            return offered;
        }
        let top = self.top_earner.max(self.average_first_team);
        match status {
            PlayerSquadStatus::KeyPlayer => offered.min((top as f32 * 1.10) as u32),
            PlayerSquadStatus::FirstTeamRegular => offered.min((top as f32 * 0.95) as u32),
            PlayerSquadStatus::HotProspectForTheFuture => {
                offered.min(self.average_first_team.max(top / 3))
            }
            PlayerSquadStatus::FirstTeamSquadRotation => {
                offered.min(self.average_first_team.max(top / 2))
            }
            PlayerSquadStatus::MainBackupPlayer => {
                let cap = if self.average_backup > 0 {
                    (self.average_backup as f32 * 1.20) as u32
                } else {
                    self.average_first_team / 2
                };
                offered.min(cap.max(top / 4))
            }
            _ => offered.min(self.average_backup.max(top / 5)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerProfile {
    YoungProspect,
    Star,
    Veteran,
    Backup,
    Standard,
}

impl PlayerProfile {
    fn classify(player: &Player, age: u8, ability: u8, status: &PlayerSquadStatus) -> Self {
        let rep = player.player_attributes.current_reputation;
        let potential = player.player_attributes.potential_ability;

        if age <= 23 && (potential >= 130 || matches!(status, PlayerSquadStatus::HotProspectForTheFuture)) {
            return PlayerProfile::YoungProspect;
        }
        if age >= 31 {
            return PlayerProfile::Veteran;
        }
        if rep > 5000 || ability >= 150 || matches!(status, PlayerSquadStatus::KeyPlayer) {
            return PlayerProfile::Star;
        }
        if matches!(
            status,
            PlayerSquadStatus::MainBackupPlayer
                | PlayerSquadStatus::DecentYoungster
                | PlayerSquadStatus::NotNeeded
        ) {
            return PlayerProfile::Backup;
        }
        PlayerProfile::Standard
    }
}

struct RenewalCandidate {
    player_id: u32,
    effective_status: PlayerSquadStatus,
    has_market_interest: bool,
    final_panic: bool,
    bosman_pressure: bool,
    override_attempts_cap: bool,
    months_remaining: i32,
}

fn proactive_contract_years(
    age: u8,
    ability: u8,
    squad_status: &PlayerSquadStatus,
    negotiation_skill: u8,
    player: &Player,
    date: NaiveDate,
) -> u8 {
    let mut years: f32 = match squad_status {
        PlayerSquadStatus::KeyPlayer => 4.0,
        PlayerSquadStatus::FirstTeamRegular => 3.5,
        PlayerSquadStatus::HotProspectForTheFuture => 4.0,
        PlayerSquadStatus::FirstTeamSquadRotation => 3.0,
        _ => 2.0,
    };

    if age >= 34 {
        years = years.min(1.5);
    } else if age >= 32 {
        years = years.min(2.0);
    } else if age >= 30 {
        years = years.min(3.0);
    }

    if age < 22 && ability > 80 {
        years += 1.0;
    }

    if negotiation_skill >= 15 {
        years -= 0.5;
    }

    let min_accept = ProcessContractHandler::player_minimum_years(player, date) as f32;
    let raised = years.max(min_accept);

    (raised.round() as u8).clamp(1, 5)
}

fn decorate_proposal(
    proposal: &mut PlayerContractProposal,
    player: &Player,
    age: u8,
    ability: u8,
    profile: &PlayerProfile,
    status: &PlayerSquadStatus,
    urgency: usize,
    budget_pressure: f32,
    structure: &WageStructureSnapshot,
    match_highest_already_used: bool,
) {
    let salary = proposal.salary;

    // Promise the role unless we're keeping the same. KeyPlayer/FirstTeam
    // promises are leverage in negotiation; backup roles aren't promised
    // because the player would consider that a downgrade in writing.
    if matches!(
        status,
        PlayerSquadStatus::KeyPlayer
            | PlayerSquadStatus::FirstTeamRegular
            | PlayerSquadStatus::HotProspectForTheFuture
    ) {
        proposal.squad_status_promise = Some(status.clone());
    }

    let greedy_agent = player.attributes.ambition + player.attributes.controversy > 24.0;
    let loyal_agent = player.attributes.loyalty > 14.0;

    // Base sweeteners — same logic as before but applied uniformly.
    if urgency >= 2 || greedy_agent {
        // Signing bonus 20-60% of annual wage, scaled with urgency.
        let scale = 0.20 + (urgency.min(5) as f32) * 0.08;
        proposal.signing_bonus = ((salary as f32) * scale) as u32;
    }
    if urgency >= 2 || loyal_agent || age >= 30 {
        let scale = 0.10 + (urgency.min(4) as f32) * 0.04;
        proposal.loyalty_bonus = ((salary as f32) * scale) as u32;
    }

    // Profile-driven extras.
    match profile {
        PlayerProfile::YoungProspect => {
            // Long progression — yearly rise, optional extension, appearance step.
            proposal.yearly_wage_rise_pct = Some(8);
            proposal.optional_extension_years = Some(1);
            proposal.wage_after_apps = Some((50, 25));
            // Ambitious prospects insist on a release clause when the
            // club is below their ceiling.
            if player.attributes.ambition >= 13.0 {
                proposal.release_clause = Some(release_clause_value(player, ability, 1.0));
            }
            // Caps milestone if they're already pushing toward the senior team.
            if player.player_attributes.international_apps >= 3 {
                proposal.wage_after_caps = Some((20, 15));
            }
        }
        PlayerProfile::Star => {
            // Match highest earner only for true elites (status == KeyPlayer),
            // and never if another candidate this monthly pass already
            // claimed the privilege — otherwise the wage hierarchy collapses.
            if matches!(status, PlayerSquadStatus::KeyPlayer)
                && structure.top_earner > 0
                && salary >= (structure.top_earner * 90 / 100)
                && !match_highest_already_used
            {
                proposal.match_highest_earner = true;
            }
            if proposal.signing_bonus == 0 {
                proposal.signing_bonus = ((salary as f32) * 0.30) as u32;
            }
            if proposal.loyalty_bonus == 0 {
                proposal.loyalty_bonus = ((salary as f32) * 0.15) as u32;
            }
            // Stars only sign a clause when they have leverage — market
            // interest or two failed offers.
            if urgency >= 2 {
                proposal.release_clause = Some(release_clause_value(player, ability, 1.4));
            }
            // Star strikers/midfielders → goal bonus; defenders/keepers → clean sheet.
            attach_position_bonus(proposal, player, salary, 1.0);
            proposal.appearance_fee = Some(((salary as f32) * 0.01) as u32);
        }
        PlayerProfile::Veteran => {
            // Shorter deal, appearance fee, optional extension after league apps.
            proposal.appearance_fee = Some(((salary as f32) * 0.02) as u32);
            proposal.appearance_extension_threshold = Some(20);
            // Loyalty bonus already handled above.
        }
        PlayerProfile::Backup => {
            // Lower base, more on appearances + unused-sub fee.
            proposal.appearance_fee = Some(((salary as f32) * 0.04) as u32);
            proposal.unused_sub_fee = Some(((salary as f32) * 0.005) as u32);
        }
        PlayerProfile::Standard => {
            attach_position_bonus(proposal, player, salary, 0.6);
            if urgency >= 2 {
                proposal.release_clause = Some(release_clause_value(player, ability, 1.0));
            }
        }
    }

    // Budget pressure: substitute base-wage shortfall with bigger appearance fee
    // and signing bonus when we had to trim the base.
    if budget_pressure >= 0.5 {
        let extra_signing = ((salary as f32) * 0.10 * budget_pressure) as u32;
        proposal.signing_bonus = proposal.signing_bonus.saturating_add(extra_signing);
        let extra_app = proposal
            .appearance_fee
            .map(|f| f.saturating_add(((salary as f32) * 0.005) as u32))
            .unwrap_or(((salary as f32) * 0.005) as u32);
        proposal.appearance_fee = Some(extra_app);
    }
}

fn release_clause_value(player: &Player, ability: u8, scale: f32) -> u32 {
    let rep = player.player_attributes.current_reputation as u32;
    let base = (ability as u32) * (ability as u32) * 4_000;
    let rep_boost = rep * 8_000;
    ((base + rep_boost) as f32 * scale) as u32
}

fn attach_position_bonus(
    proposal: &mut PlayerContractProposal,
    player: &Player,
    salary: u32,
    scale: f32,
) {
    let pos = player.position();
    if pos.is_forward() || pos.is_midfielder() {
        proposal.goal_bonus = Some(((salary as f32) * 0.012 * scale) as u32);
    } else if pos.is_goalkeeper() || pos.is_defender() {
        proposal.clean_sheet_bonus = Some(((salary as f32) * 0.012 * scale) as u32);
    }
}

fn status_rank(status: &PlayerSquadStatus) -> u8 {
    use PlayerSquadStatus::*;
    match status {
        KeyPlayer => 7,
        FirstTeamRegular => 6,
        HotProspectForTheFuture => 5,
        FirstTeamSquadRotation => 4,
        MainBackupPlayer => 3,
        DecentYoungster => 2,
        NotNeeded => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod wage_structure_tests {
    use super::*;

    fn snapshot(top: u32, avg_first_team: u32, avg_backup: u32) -> WageStructureSnapshot {
        WageStructureSnapshot {
            current_bill: top * 5,
            top_earner: top,
            average_first_team: avg_first_team,
            average_backup: avg_backup,
        }
    }

    #[test]
    fn key_player_may_marginally_exceed_top() {
        let s = snapshot(200_000, 150_000, 60_000);
        // Asking for 300k — clamped to 110% of top = 220k.
        assert_eq!(s.cap_for_status(300_000, &PlayerSquadStatus::KeyPlayer), 220_000);
    }

    #[test]
    fn first_team_regular_held_below_top() {
        let s = snapshot(200_000, 150_000, 60_000);
        // Asking for 300k — clamped to 95% of top = 190k.
        assert_eq!(
            s.cap_for_status(300_000, &PlayerSquadStatus::FirstTeamRegular),
            190_000
        );
    }

    #[test]
    fn backup_capped_well_below_first_team() {
        let s = snapshot(200_000, 150_000, 60_000);
        let capped = s.cap_for_status(180_000, &PlayerSquadStatus::MainBackupPlayer);
        // 1.20 × average_backup = 72k, max with top/4 = 50k → cap is 72k.
        assert_eq!(capped, 72_000);
    }

    #[test]
    fn cap_passes_through_when_under_limit() {
        let s = snapshot(200_000, 150_000, 60_000);
        assert_eq!(s.cap_for_status(50_000, &PlayerSquadStatus::FirstTeamRegular), 50_000);
    }

    #[test]
    fn empty_structure_does_not_clamp() {
        let s = snapshot(0, 0, 0);
        assert_eq!(s.cap_for_status(500_000, &PlayerSquadStatus::KeyPlayer), 500_000);
    }

    #[test]
    fn budget_reservation_prevents_simultaneous_overcommit() {
        // Simulate the renewal loop's monthly behaviour: snapshot says
        // current_bill = 1.0M, budget = 1.2M. Two candidates each ask
        // for +200k. The first offer slides under the cap; the second
        // — after the loop reserves the first delta — must be capped
        // back to current_salary because no headroom remains.
        let mut s = snapshot(200_000, 150_000, 60_000);
        s.current_bill = 1_000_000;
        let budget: u32 = 1_200_000;
        let asked: u32 = 200_000; // each candidate's wage delta over current

        // Helper mirroring the production path's gate.
        fn reserve(s: &mut WageStructureSnapshot, current: u32, raise: u32, budget: u32) -> u32 {
            let mut offered = current + raise;
            let salary_delta = offered.saturating_sub(current);
            if s.current_bill + salary_delta > budget {
                let remaining = budget.saturating_sub(s.current_bill);
                offered = (current + remaining).max(current);
            }
            let final_delta = offered.saturating_sub(current);
            s.current_bill = s.current_bill.saturating_add(final_delta);
            offered
        }

        // Candidate A — current 100k, asks +200k → granted in full.
        let a = reserve(&mut s, 100_000, asked, budget);
        assert_eq!(a, 300_000);
        assert_eq!(s.current_bill, 1_200_000);
        // Candidate B — current 100k, asks +200k → no headroom, cap at current.
        let b = reserve(&mut s, 100_000, asked, budget);
        assert_eq!(b, 100_000, "budget should already be exhausted");
        assert_eq!(s.current_bill, 1_200_000);
    }
}
