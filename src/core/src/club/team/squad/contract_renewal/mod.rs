use crate::ContractClauseType;
use crate::club::player::calculators::{ContractValuation, ValuationContext};
use crate::club::player::contract::RENEWAL_OFFERED_LABEL;
use crate::club::player::mailbox::RejectionReason;
use crate::club::player::mailbox::handlers::contract_proposal::{
    ProcessContractHandler, RENEWAL_REJECTED_LABEL,
};
use crate::club::player::player::Player;
use crate::club::staff::perception::PotentialEstimator;
use crate::utils::DateUtils;
use crate::utils::FormattingUtils;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerSquadStatus, PlayerStatusType,
    Team,
};
use chrono::NaiveDate;
use log::debug;

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
const DECISION_LABEL: &str = RENEWAL_OFFERED_LABEL;

pub struct ContractRenewalManager;

impl ContractRenewalManager {
    /// Walk the main team, deliver renewal proposals to valuable players
    /// whose contracts are approaching expiry. Deterministic — no AI call.
    ///
    /// Runs monthly so valuable players are offered a renewal before the
    /// stalemate/listing pipeline can act on an expiring contract — the
    /// club shouldn't sell a player it actually wants to keep just because
    /// his deal is winding down.
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

            let player_ref = match teams[main_idx]
                .players
                .players
                .iter()
                .find(|p| p.id == candidate.player_id)
            {
                Some(p) => p,
                None => continue,
            };
            let built = Self::build_offer(
                player_ref,
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
                    FormattingUtils::format_money(proposal.salary as f64)
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
                            ContractClauseType::MatchHighestEarner
                                | ContractClauseType::OptionalContractExtensionByClub
                        )
                    })
                })
                .unwrap_or(false);
            if !has_team_clause {
                continue;
            }

            // Decide whether the optional extension is worth exercising
            // BEFORE we mutate the contract. The gate is deliberately
            // strict — early extensions (3+ years before expiry) burn
            // future flexibility for no upside.
            //
            // Fire only when ALL of these hold:
            //   1. Inside the final 12 months (extensions exist to avoid
            //      losing the player to a Bosman, not to lock them in
            //      mid-deal).
            //   2. Player is a first-team contributor (KeyPlayer /
            //      FirstTeamRegular / FirstTeamSquadRotation) with
            //      meaningful match minutes this season.
            //   3. Player isn't transfer-listed or pushing for a move —
            //      a Req/Lst/Frt player extending is just paperwork.
            //   4. Ability hasn't visibly fallen off a cliff vs potential
            //      (>40% decline = the player is finished, let the deal run).
            let now_status = player.contract.as_ref().map(|c| c.squad_status.clone());
            let days_to_expiry = player
                .contract
                .as_ref()
                .map(|c| (c.expiration - date).num_days())
                .unwrap_or(i64::MAX);
            let in_final_year = days_to_expiry > 0 && days_to_expiry <= 365;
            let played_share =
                (player.statistics.played as f32).max(player.statistics.played_subs as f32 / 2.0);
            let is_contributor = matches!(
                now_status.as_ref(),
                Some(PlayerSquadStatus::KeyPlayer)
                    | Some(PlayerSquadStatus::FirstTeamRegular)
                    | Some(PlayerSquadStatus::FirstTeamSquadRotation)
            ) && played_share >= 8.0;
            let unsettled = player.statuses.get().iter().any(|s| {
                matches!(
                    s,
                    PlayerStatusType::Req | PlayerStatusType::Lst | PlayerStatusType::Frt
                )
            }) || player
                .contract
                .as_ref()
                .map(|c| c.is_transfer_listed)
                .unwrap_or(false);
            let ca = player.player_attributes.current_ability;
            // Observable ceiling — the club judges decline from what it
            // can see, never the hidden biological PA.
            let pa = PotentialEstimator::observable_ceiling(player, date).max(ca);
            let declining = pa > 0 && (ca as f32 / pa as f32) < 0.60;

            let should_extend = in_final_year && is_contributor && !unsettled && !declining;

            if let Some(c) = player.contract.as_mut() {
                let _ = c.try_apply_match_highest_earner(top_excl);
                if should_extend {
                    let _ = c.exercise_optional_extension();
                } else if days_to_expiry <= 60 {
                    // Drop unused options once expiry is near so the
                    // clause doesn't linger in stale-data audits.
                    c.clauses.retain(|cl| {
                        !matches!(
                            cl.bonus_type,
                            ContractClauseType::OptionalContractExtensionByClub
                        )
                    });
                }
            }
        }
    }

    fn resolve_staff(team: &Team) -> (String, u8, u8) {
        let coach_name = team.staffs.head_coach_name();

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
        Self::evaluate_inner(player, date, false)
    }

    /// Same evaluation as [`Self::evaluate`] but skips the borrower-side
    /// "loaned-in" gate so a parent club can decide whether to renew a
    /// player who is currently away on loan. The permanent contract still
    /// lives on `player.contract`; the loan agreement on `contract_loan`
    /// is incidental to the renewal question.
    fn evaluate_for_parent(player: &Player, date: NaiveDate) -> Option<RenewalCandidate> {
        Self::evaluate_inner(player, date, true)
    }

    /// Expiry-day evaluation: the contract has already lapsed
    /// (`days_remaining <= 0`), which `evaluate_inner` treats as "not a
    /// renewal candidate". This path exists for the synchronous last-chance
    /// offer the release sweep makes before clearing the contract — the
    /// club's final attempt to keep the player from walking for free.
    /// Always `final_panic` and never capped on attempts: there is no
    /// later tick to defer to.
    fn evaluate_for_expiry(player: &Player, date: NaiveDate) -> Option<RenewalCandidate> {
        // A loaned-in player's permanent contract belongs to the parent
        // club; the borrower gets no expiry-day claim on him.
        if player.is_on_loan() || player.is_retired() {
            return None;
        }

        let contract = player.contract.as_ref()?;
        let days_remaining = (contract.expiration - date).num_days();
        if days_remaining > 0 {
            return None;
        }

        // A transfer-listed player gets no last-chance offer — the club
        // already decided to move him on. His deal simply lapses and he
        // walks for free. Checked on the contract flag as well as the
        // statuses because club-side listers set `is_transfer_listed`
        // without stamping `Lst`.
        if contract.is_transfer_listed {
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

        Some(RenewalCandidate {
            player_id: player.id,
            effective_status: Self::effective_squad_status(player, &contract.squad_status),
            has_market_interest,
            final_panic: true,
            bosman_pressure: false,
            override_attempts_cap: true,
            months_remaining: 0,
        })
    }

    fn evaluate_inner(
        player: &Player,
        date: NaiveDate,
        for_parent_loanee: bool,
    ) -> Option<RenewalCandidate> {
        // Borrower-side: a loaned-in player isn't the borrower's to renew.
        // Parent-side: the player is OUR loanee — proceed against the
        // permanent contract on `player.contract`.
        if !for_parent_loanee && player.is_on_loan() {
            return None;
        }

        let contract = player.contract.as_ref()?;
        let days_remaining = (contract.expiration - date).num_days();
        if days_remaining <= 0 {
            return None;
        }

        // Players on the transfer list are never offered new contracts —
        // renewing a player the club is actively selling is contradictory
        // paperwork that kept unsold listings alive for years. The
        // contract flag matters as much as the statuses: club-side
        // listers (surplus trim, salary fallback) set `is_transfer_listed`
        // without stamping `Lst`.
        if contract.is_transfer_listed {
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
        let last = player
            .decision_history
            .items
            .iter()
            .rev()
            .find(|d| d.decision == DECISION_LABEL || d.decision == RENEWAL_REJECTED_LABEL);
        match last {
            Some(d) if d.decision == RENEWAL_REJECTED_LABEL => {
                (date - d.date).num_days() < RENEWAL_COOLDOWN_AFTER_REJECT_DAYS
            }
            Some(d) => (date - d.date).num_days() < RENEWAL_COOLDOWN_DAYS,
            None => false,
        }
    }

    fn build_offer(
        player: &Player,
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

        let profile =
            PlayerProfile::classify(player, age, ability, &candidate.effective_status, date);
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

        let mut proposal =
            PlayerContractProposal::basic(offered, years, negotiation_skill, 0, 0, None);

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

        // Stamp the offer-time valuation context onto the proposal so
        // acceptance evaluates against the SAME elite-club / mid-tier
        // expectations this offer was tuned for.
        proposal.valuation_club_reputation = Some(team_rep_factor);
        proposal.valuation_league_reputation = Some(league_reputation);
        proposal.valuation_expected_wage = Some(valuation.expected_wage);
        proposal.valuation_min_acceptable = Some(valuation.min_acceptable);

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

    /// Build a renewal proposal for a player whose permanent contract is
    /// owned by `parent_team`'s club but who currently lives at another
    /// club because of an active loan. Returns the proposal and the
    /// parent coach's name to stamp into the player's decision history.
    ///
    /// The caller is expected to:
    ///   1. Have established `parent_team` is the loanee's parent main team
    ///      (via `Player::is_loaned_out_from(parent_team.club_id)`).
    ///   2. Push the returned proposal into the loanee's mailbox and
    ///      append the decision-history row — same wire-up the in-house
    ///      `run_with_budget` loop performs. The loanee sits in another
    ///      club's roster, so we can't take a mut-borrow on him here
    ///      without violating the country-level borrow plan.
    pub fn try_build_loanee_offer(
        parent_team: &Team,
        loanee: &Player,
        date: NaiveDate,
        wage_budget: Option<u32>,
        league_reputation: u16,
        structure: &WageStructureSnapshot,
    ) -> Option<(PlayerContractProposal, String)> {
        let candidate = Self::evaluate_for_parent(loanee, date)?;

        let attempts = loanee
            .decision_history
            .items
            .iter()
            .filter(|d| d.decision == DECISION_LABEL && (date - d.date).num_days() < 365)
            .count();
        if attempts >= MAX_RENEWAL_ATTEMPTS_PER_YEAR && !candidate.override_attempts_cap {
            return None;
        }

        let (coach_name, negotiation_skill, judging_ability) = Self::resolve_staff(parent_team);
        let team_rep_factor = parent_team.reputation.overall_score();

        let proposal = Self::build_offer(
            loanee,
            negotiation_skill,
            judging_ability,
            date,
            attempts,
            team_rep_factor,
            league_reputation,
            wage_budget,
            structure,
            &candidate,
            // Loanee offers run one-at-a-time on the country pass, so
            // match-highest-earner is allowed in isolation. The parent's
            // monthly proactive pass already used (or didn't use) the slot
            // for in-house candidates; loanees shouldn't be starved of it.
            false,
        )?;
        Some((proposal, coach_name))
    }

    /// Build the synchronous last-chance proposal for a player whose
    /// contract expires today (or has already lapsed). Called by the
    /// release sweep BEFORE it clears the contract, so the club gets one
    /// final renewal attempt instead of silently losing the player.
    ///
    /// Returns the proposal and the coach's name for the decision-history
    /// row. The caller wires both up and runs the acceptance handler
    /// in-place — pushing to the mailbox would race the release sweep,
    /// which can clear the contract before the mailbox is drained.
    pub fn try_build_expiry_day_offer(
        team: &Team,
        player: &Player,
        date: NaiveDate,
        wage_budget: Option<u32>,
        league_reputation: u16,
        structure: &WageStructureSnapshot,
    ) -> Option<(PlayerContractProposal, String)> {
        let candidate = Self::evaluate_for_expiry(player, date)?;

        // No attempts-cap gate: expiry candidates always carry
        // `override_attempts_cap`. Prior attempts still feed escalation
        // inside `build_offer`, so count them anyway.
        let attempts = player
            .decision_history
            .items
            .iter()
            .filter(|d| d.decision == DECISION_LABEL && (date - d.date).num_days() < 365)
            .count();

        let (coach_name, negotiation_skill, judging_ability) = Self::resolve_staff(team);
        let team_rep_factor = team.reputation.overall_score();

        let proposal = Self::build_offer(
            player,
            negotiation_skill,
            judging_ability,
            date,
            attempts,
            team_rep_factor,
            league_reputation,
            wage_budget,
            structure,
            &candidate,
            // Same one-at-a-time reasoning as the loanee wrapper:
            // match-highest-earner is allowed in isolation here.
            false,
        )?;

        // Don't spam a pointless final offer. Once the player has already
        // turned down a season's worth of attempts, the expiry-day offer
        // is only worth making if it MATERIALLY improves on what he kept
        // rejecting — it matches his stated wage ask or grants a demand he
        // was holding out for. If even the last offer can't clear that
        // bar, the club has effectively decided to let him walk; record
        // *why* (for diagnostics) and make no offer rather than firing the
        // same losing proposal again.
        if attempts >= MAX_RENEWAL_ATTEMPTS_PER_YEAR
            && !Self::expiry_offer_materially_improves(player, &proposal)
        {
            let reason = Self::diagnose_walk(player, &proposal);
            debug!(
                "Expiry renewal: {} (id {}) walks — {} (attempts={}, final offer ${}/y, {}y)",
                player.full_name,
                player.id,
                reason.label(),
                attempts,
                proposal.salary,
                proposal.years,
            );
            return None;
        }

        Some((proposal, coach_name))
    }

    /// Whether the final expiry-day offer is worth making to a player who
    /// has already rejected the season's worth of attempts: it must match
    /// his stated wage ask, or grant a key demand (release clause, squad
    /// role, or contract length) he was holding out for. With no recorded
    /// ask there is nothing new to put on the table, so a repeat offer is
    /// pointless.
    fn expiry_offer_materially_improves(
        player: &Player,
        proposal: &PlayerContractProposal,
    ) -> bool {
        // Full-package check FIRST, independent of any stated ask. The
        // acceptance handler signs off when the total package (base wage
        // plus amortized bonuses / fees / clauses) is at least a 10% uplift
        // on the current salary — even when the base wage sits below the
        // player's ask. Mirror that so we never suppress a package-sweetened
        // final offer the player would actually accept.
        //
        // The acceptance handler gates that package path on
        // `proposal.salary >= current_salary` — a base pay cut is refused
        // regardless of sweeteners (e.g. a player who is his own top earner
        // gets the offer clamped below his wage by the wage-structure cap).
        // Mirror that gate too, or we'd green-light a doomed pay-cut offer.
        if let Some(current) = player.contract.as_ref().map(|c| c.salary.max(1)) {
            if proposal.salary >= current {
                let pkg = ProcessContractHandler::expected_package_value(proposal, player);
                if pkg as f32 / current as f32 >= 1.10 {
                    return true;
                }
            }
        }

        let Some(ask) = player.pending_contract_ask.as_ref() else {
            return false;
        };
        // Meets (or beats) the wage he asked for.
        if proposal.salary >= ask.desired_salary {
            return true;
        }
        // Grants the release clause he demanded.
        if ask.demanded_release_clause.is_some() && proposal.release_clause.is_some() {
            return true;
        }
        // Grants the squad role he wanted written in.
        if let Some(status) = ask.demanded_status.as_ref() {
            if proposal.squad_status_promise.as_ref() == Some(status) {
                return true;
            }
        }
        // Honours a longer contract when length was the sticking point.
        if matches!(ask.rejection_reason, Some(RejectionReason::ShortContract))
            && proposal.years >= ask.desired_years
        {
            return true;
        }
        false
    }

    /// Classify why a player the club is no longer chasing on expiry day
    /// ends up walking, from his last stated ask versus the best final
    /// offer. Diagnostics only — drives the debug log so long runs can be
    /// audited for *which* sticking point loses players for free.
    fn diagnose_walk(player: &Player, proposal: &PlayerContractProposal) -> RenewalWalkReason {
        let Some(ask) = player.pending_contract_ask.as_ref() else {
            return RenewalWalkReason::ClubLetWalk;
        };
        if let Some(status) = ask.demanded_status.as_ref() {
            if proposal.squad_status_promise.as_ref() != Some(status) {
                return RenewalWalkReason::WantedBiggerRole;
            }
        }
        if ask.demanded_release_clause.is_some() && proposal.release_clause.is_none() {
            return RenewalWalkReason::WantedReleaseClause;
        }
        if matches!(ask.rejection_reason, Some(RejectionReason::ShortContract))
            && proposal.years < ask.desired_years
        {
            return RenewalWalkReason::WantedLongerDeal;
        }
        if proposal.salary < ask.desired_salary {
            return RenewalWalkReason::CouldNotAffordAsk;
        }
        RenewalWalkReason::ClubLetWalk
    }
}

/// Why a player walks for free on expiry day once the club stops chasing
/// the renewal. Surfaced in the debug log so multi-season runs can be
/// audited for which sticking point most often loses players on a free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenewalWalkReason {
    /// The wage the player asked for sat above what the club would fund.
    CouldNotAffordAsk,
    /// The player demanded a higher squad role than the club promised.
    WantedBiggerRole,
    /// The player insisted on a release clause the offer didn't carry.
    WantedReleaseClause,
    /// The player wanted a longer contract than the club would commit to.
    WantedLongerDeal,
    /// No outstanding demand to satisfy — the club simply chose not to
    /// re-offer and let the player leave.
    ClubLetWalk,
}

impl RenewalWalkReason {
    fn label(self) -> &'static str {
        match self {
            RenewalWalkReason::CouldNotAffordAsk => "could not afford wage ask",
            RenewalWalkReason::WantedBiggerRole => "player wanted a bigger role",
            RenewalWalkReason::WantedReleaseClause => "player wanted a release clause",
            RenewalWalkReason::WantedLongerDeal => "player wanted a longer deal",
            RenewalWalkReason::ClubLetWalk => "club let the player walk",
        }
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
            // Loaned-in players are billed against the loan contract,
            // not the parent contract. Their parent salary (which can
            // be 1M+ for an elite-club loanee at a tier-3 borrower) must
            // not anchor the borrower's top earner — the renewal AI
            // would then refuse to offer reasonable wages to permanent
            // squad members because "we already pay X".
            let c = if let Some(loan) = p.contract_loan.as_ref() {
                loan
            } else if let Some(c) = p.contract.as_ref() {
                c
            } else {
                continue;
            };
            bill = bill.saturating_add(c.salary);
            top = top.max(c.salary);
            // Squad-status counters use the player's *resolved* status —
            // a loanee's status sits on the parent contract; we still
            // bucket them by it so the wage-tier averages stay coherent.
            let status = p
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
            match status {
                PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular => {
                    first_team_sum = first_team_sum.saturating_add(c.salary);
                    first_team_count += 1;
                }
                PlayerSquadStatus::FirstTeamSquadRotation | PlayerSquadStatus::MainBackupPlayer => {
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
    fn classify(
        player: &Player,
        age: u8,
        ability: u8,
        status: &PlayerSquadStatus,
        date: NaiveDate,
    ) -> Self {
        let rep = player.player_attributes.current_reputation;
        // Renewal packages are a club decision — the prospect profile
        // reads the observable ceiling, never the hidden biological PA.
        let potential = PotentialEstimator::observable_ceiling(player, date);

        if age <= 23
            && (potential >= 130 || matches!(status, PlayerSquadStatus::HotProspectForTheFuture))
        {
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
        assert_eq!(
            s.cap_for_status(300_000, &PlayerSquadStatus::KeyPlayer),
            220_000
        );
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
        assert_eq!(
            s.cap_for_status(50_000, &PlayerSquadStatus::FirstTeamRegular),
            50_000
        );
    }

    #[test]
    fn empty_structure_does_not_clamp() {
        let s = snapshot(0, 0, 0);
        assert_eq!(
            s.cap_for_status(500_000, &PlayerSquadStatus::KeyPlayer),
            500_000
        );
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

#[cfg(test)]
mod loanee_evaluate_tests {
    //! Borrower-side renewal must ignore loaned-in players (they aren't
    //! the borrower's to renew). Parent-side renewal — exposed via
    //! `evaluate_for_parent` — must still pick the same player up when
    //! his parent contract is approaching expiry.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_loanee(parent_expiration: NaiveDate, loan_end: NaiveDate) -> Player {
        let mut p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Loanee".into()))
            .birth_date(d(1998, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        let mut contract = PlayerClubContract::new(100_000, parent_expiration);
        contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
        p.contract = Some(contract);
        // Parent club id = 99, loan agreement at borrower club id = 2.
        p.contract_loan = Some(PlayerClubContract::new_loan(50_000, loan_end, 99, 1, 2));
        p
    }

    #[test]
    fn evaluate_skips_loaned_in_player() {
        let today = d(2026, 5, 13);
        // Parent contract expires in ~110 days — would normally fire
        // final-panic / bosman pressure, but borrower-side evaluate must
        // still return None because the player isn't theirs to renew.
        let p = make_loanee(d(2026, 8, 31), d(2026, 7, 31));
        assert!(ContractRenewalManager::evaluate(&p, today).is_none());
    }

    #[test]
    fn evaluate_for_parent_picks_up_loaned_out_near_expiry() {
        let today = d(2026, 5, 13);
        // Same player, parent perspective: the renewal manager must
        // produce a candidate so the country-level pass can build an
        // offer at the parent's wage hierarchy.
        let p = make_loanee(d(2026, 8, 31), d(2026, 7, 31));
        assert!(ContractRenewalManager::evaluate_for_parent(&p, today).is_some());
    }

    #[test]
    fn evaluate_for_parent_silent_when_parent_contract_has_long_runway() {
        let today = d(2026, 5, 13);
        // Parent contract still has ~4 years to run — well past every
        // squad-status threshold. No renewal pressure even on parent side.
        let p = make_loanee(d(2030, 6, 30), d(2027, 5, 31));
        assert!(ContractRenewalManager::evaluate_for_parent(&p, today).is_none());
    }
}

#[cfg(test)]
mod expiry_evaluate_tests {
    //! The expiry-day path fires only for already-lapsed contracts and
    //! must always carry the final-panic / cap-override flags — the
    //! release sweep gives the club exactly one shot before clearing
    //! the contract.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };

    struct ExpiryFixtures;

    impl ExpiryFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn player_with_expiration(expiration: NaiveDate) -> Player {
            let mut p = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".into(), "Expiry".into()))
                .birth_date(Self::d(1998, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            let mut contract = PlayerClubContract::new(100_000, expiration);
            contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
            p.contract = Some(contract);
            p
        }
    }

    #[test]
    fn evaluate_for_expiry_fires_with_panic_flags_on_expiry_day() {
        let today = ExpiryFixtures::d(2026, 6, 10);
        let p = ExpiryFixtures::player_with_expiration(today);
        let candidate =
            ContractRenewalManager::evaluate_for_expiry(&p, today).expect("expired → candidate");
        assert!(candidate.final_panic, "expiry-day offer is always panic");
        assert!(
            candidate.override_attempts_cap,
            "the last chance cannot be attempt-capped"
        );
        assert_eq!(candidate.months_remaining, 0);
    }

    #[test]
    fn evaluate_for_expiry_silent_while_contract_still_runs() {
        let today = ExpiryFixtures::d(2026, 6, 10);
        let p = ExpiryFixtures::player_with_expiration(ExpiryFixtures::d(2026, 6, 11));
        assert!(
            ContractRenewalManager::evaluate_for_expiry(&p, today).is_none(),
            "a live contract belongs to the normal renewal paths"
        );
    }

    #[test]
    fn evaluate_for_expiry_skips_loaned_in_player() {
        let today = ExpiryFixtures::d(2026, 6, 10);
        let mut p = ExpiryFixtures::player_with_expiration(today);
        // Borrower club 2 holds the player; parent club 99 owns the
        // expired permanent contract — not the borrower's to renew.
        p.contract_loan = Some(PlayerClubContract::new_loan(
            50_000,
            ExpiryFixtures::d(2026, 12, 31),
            99,
            1,
            2,
        ));
        assert!(ContractRenewalManager::evaluate_for_expiry(&p, today).is_none());
    }

    #[test]
    fn evaluate_for_expiry_skips_listed_player() {
        let today = ExpiryFixtures::d(2026, 6, 10);
        let mut p = ExpiryFixtures::player_with_expiration(today);
        p.statuses.add(today, PlayerStatusType::Lst);
        assert!(
            ContractRenewalManager::evaluate_for_expiry(&p, today).is_none(),
            "the club already decided to move a listed player on"
        );
    }

    #[test]
    fn evaluate_for_expiry_skips_flagged_transfer_listed_player() {
        // Club-side listers set `contract.is_transfer_listed` WITHOUT
        // stamping `Lst` — the flag alone must block the last-chance
        // offer, or listed players get renewed and linger for years.
        let today = ExpiryFixtures::d(2026, 6, 10);
        let mut p = ExpiryFixtures::player_with_expiration(today);
        p.contract.as_mut().unwrap().is_transfer_listed = true;
        assert!(
            ContractRenewalManager::evaluate_for_expiry(&p, today).is_none(),
            "a transfer-listed contract must simply lapse"
        );
    }

    #[test]
    fn evaluate_skips_flagged_transfer_listed_player() {
        // The regular renewal path must equally refuse to open talks
        // with a player the club is actively selling.
        let today = ExpiryFixtures::d(2026, 6, 10);
        // ~3 months of runway — well inside every renewal threshold, so
        // only the listing flag can be the reason for silence.
        let mut p = ExpiryFixtures::player_with_expiration(ExpiryFixtures::d(2026, 9, 10));
        p.contract.as_mut().unwrap().is_transfer_listed = true;
        assert!(
            ContractRenewalManager::evaluate(&p, today).is_none(),
            "players on the transfer list are never offered new contracts"
        );
    }

    /// The expiry-day anti-spam gate must align with the player's actual
    /// acceptance logic, which signs off on the FULL package (base wage
    /// plus amortized sweeteners), gated on the base wage not being a pay
    /// cut. A base wage below the player's ask is therefore NOT pointless
    /// when sweeteners lift the package over the 10% acceptance floor — the
    /// offer must still be made, not suppressed.
    #[test]
    fn materially_improves_counts_full_package_not_just_base_wage() {
        use crate::club::player::mailbox::PlayerContractAsk;

        let today = ExpiryFixtures::d(2026, 6, 10);
        let mut player = ExpiryFixtures::player_with_expiration(today);
        // Current salary is 100k (set by the fixture). He's holding out for
        // a far higher base wage, with no clause / role / length demand.
        player.pending_contract_ask = Some(PlayerContractAsk {
            desired_salary: 800_000,
            desired_years: 3,
            recorded_on: today,
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        });

        // (a) A package-sweetened offer: base 110k (above current, below
        // the 800k ask) plus a 60k loyalty bonus pushes the package to
        // ~170k — a 1.7x uplift the acceptance gate would sign. It must
        // NOT be suppressed even though base < ask. This is the fix.
        let sweetened = PlayerContractProposal::basic(110_000, 3, 10, 0, 60_000, None);
        assert!(
            ContractRenewalManager::expiry_offer_materially_improves(&player, &sweetened),
            "a package-sweetened offer the player would accept must not be suppressed"
        );

        // (b) A meagre offer below the ask with no sweeteners (package only
        // ~1.05x current) IS pointless — suppress it rather than spam.
        let meagre = PlayerContractProposal::basic(105_000, 3, 10, 0, 0, None);
        assert!(
            !ContractRenewalManager::expiry_offer_materially_improves(&player, &meagre),
            "a meagre below-ask offer with no package value is correctly suppressed"
        );

        // (c) A pay cut (base below current) can never clear the package
        // path — the acceptance gate refuses base pay cuts outright.
        let pay_cut = PlayerContractProposal::basic(90_000, 3, 10, 0, 500_000, None);
        assert!(
            !ContractRenewalManager::expiry_offer_materially_improves(&player, &pay_cut),
            "a base pay cut is suppressed regardless of sweeteners"
        );

        // (d) Meeting the wage ask outright is always worth offering.
        let meets_ask = PlayerContractProposal::basic(800_000, 3, 10, 0, 0, None);
        assert!(ContractRenewalManager::expiry_offer_materially_improves(
            &player, &meets_ask
        ));
    }
}
