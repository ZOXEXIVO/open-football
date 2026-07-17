use crate::club::player::agent::PlayerAgent;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::calculators::{
    ContractValuation, ValuationContext, expected_annual_value, package_inputs_from_proposal,
};
/// Written into `decision_history.decision` whenever the player turns
/// down a proposal. The ContractRenewalManager reads this back to tell
/// "still waiting" from "already said no", applies a longer cooldown,
/// caps the retry count, and escalates terms on the next attempt.
/// Canonical definition lives with the stalemate assessment; re-exported
/// here so existing call sites keep their import path.
pub use crate::club::player::contract::RENEWAL_REJECTED_LABEL;
use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};
use crate::handlers::AcceptContractHandler;
use crate::utils::DateUtils;
use crate::utils::FormattingUtils;
use crate::{
    ContractEventContext, ContractEventEvidence, ContractEventKind, HappinessEventCause,
    HappinessEventContext, HappinessEventScope, HappinessEventSeverity, HappinessEventType,
    PersonBehaviourState, Player, PlayerContractProposal, PlayerResult, PlayerSquadStatus,
    PlayerStatusType,
};
use chrono::NaiveDate;

pub struct ProcessContractHandler;

fn log_rejection(player: &mut Player, proposal: &PlayerContractProposal, now: NaiveDate) {
    let movement = format!(
        "{}y · ${}/y",
        proposal.years,
        FormattingUtils::format_money(proposal.salary as f64)
    );
    player.decision_history.add(
        now,
        movement,
        RENEWAL_REJECTED_LABEL.to_string(),
        String::new(),
    );
}

/// Capture the player's stated terms after a rejection so the next offer
/// can converge. The club reads `player.pending_contract_ask` when building
/// the next proposal; without this the negotiation is blind on the club
/// side. Reason drives which lever the next offer should pull — wage,
/// length, role, or release clause.
fn record_counter_offer(
    player: &mut Player,
    proposal: &PlayerContractProposal,
    now: NaiveDate,
    min_years: u8,
    reason: RejectionReason,
) {
    let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
    let agent = PlayerAgent::for_player(player);

    let salary_anchor = proposal.salary.max(current_salary);
    // Greedy agents push the next ask up sharply — but only when money was
    // actually the sticking point. A rejection over length, role, or a
    // missing clause must not ratchet the wage ask above an offer the
    // player never complained about, or the club chases a moving target
    // it was never asked to hit.
    let greed_multiplier = match reason {
        RejectionReason::LowSalary | RejectionReason::NoSweetener => 1.0 + agent.greed * 0.25,
        _ => 1.0,
    };
    let desired_salary = ((salary_anchor as f32) * greed_multiplier) as u32;
    let desired_years = proposal.years.max(min_years);

    let demanded_status = match reason {
        RejectionReason::StatusBelowExpectation => Some(promote_status(
            player
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular),
        )),
        _ => None,
    };

    let demanded_release_clause = match reason {
        RejectionReason::AmbitionMismatch | RejectionReason::NoReleaseClause => {
            // Sized off ability and reputation — see build_sweeteners' base.
            let ability = player.player_attributes.current_ability as u32;
            let rep = player.player_attributes.current_reputation as u32;
            Some(ability * ability * 4_000 + rep * 8_000)
        }
        _ => None,
    };

    let demanded_signing_bonus = match reason {
        RejectionReason::NoSweetener | RejectionReason::LowSalary => {
            Some(((desired_salary as f32) * 0.30) as u32)
        }
        _ => None,
    };

    player.pending_contract_ask = Some(PlayerContractAsk {
        desired_salary,
        desired_years,
        recorded_on: now,
        demanded_status,
        demanded_release_clause,
        demanded_signing_bonus,
        rejection_reason: Some(reason),
    });
}

fn promote_status(status: PlayerSquadStatus) -> PlayerSquadStatus {
    use PlayerSquadStatus::*;
    match status {
        NotNeeded | MainBackupPlayer => FirstTeamSquadRotation,
        FirstTeamSquadRotation => FirstTeamRegular,
        FirstTeamRegular => KeyPlayer,
        DecentYoungster => HotProspectForTheFuture,
        other => other,
    }
}

fn accept_and_clear(player: &mut Player, proposal: PlayerContractProposal, now: NaiveDate) {
    // A renewal closes the loop the offered / rejected rows opened — the
    // player putting pen to a fresh deal at his current club is a
    // first-class decision the register should show alongside them. Read
    // the incumbent contract BEFORE the handler installs the new one; a
    // free-agent first signing (no current contract) is narrated by the
    // signing path, not here.
    if player.contract.is_some() {
        let movement = format!(
            "{}y · ${}/y",
            proposal.years,
            FormattingUtils::format_money(proposal.salary as f64)
        );
        player.decision_history.add(
            now,
            movement,
            "dec_contract_renewal_signed".to_string(),
            String::new(),
        );
    }
    AcceptContractHandler::process(player, proposal, now);
    player.pending_contract_ask = None;
}

/// Total annual package value used in acceptance scoring. Delegates to
/// the shared `expected_annual_value` helper so happiness, acceptance,
/// and renewal-tuning all evaluate the same package shape consistently.
fn package_value(proposal: &PlayerContractProposal, player: &Player) -> u32 {
    let inputs = package_inputs_from_proposal(proposal, player);
    expected_annual_value(&inputs)
}

impl ProcessContractHandler {
    /// Expected annual value of a proposal's full package — base wage plus
    /// the amortized value of signing / loyalty bonuses, performance fees,
    /// and clauses. This is the same figure [`Self::process`] weighs as
    /// `pkg_value` when deciding acceptance, exposed so the renewal manager
    /// can judge a final offer against the SAME yardstick the player uses
    /// (a base wage below the player's ask can still clear acceptance once
    /// the package sweeteners are counted).
    pub fn expected_package_value(proposal: &PlayerContractProposal, player: &Player) -> u32 {
        package_value(proposal, player)
    }

    pub fn process(
        player: &mut Player,
        proposal: PlayerContractProposal,
        now: NaiveDate,
        result: &mut PlayerResult,
    ) {
        let min_acceptable_years = Self::player_minimum_years(player, now);
        if proposal.years < min_acceptable_years {
            result.contract.contract_rejected = true;
            record_counter_offer(
                player,
                &proposal,
                now,
                min_acceptable_years,
                RejectionReason::ShortContract,
            );
            Self::emit_rejected_contract_offer(player, &proposal, RejectionReason::ShortContract);
            log_rejection(player, &proposal, now);
            return;
        }

        let agent = PlayerAgent::for_player(player);

        // Status promise check: if the player wanted a clearer role
        // upgrade and the proposal demotes them, reject before salary maths.
        if let (Some(promised), Some(current)) = (
            proposal.squad_status_promise.clone(),
            player.contract.as_ref().map(|c| c.squad_status.clone()),
        ) {
            if promised.seniority_rank() < current.seniority_rank()
                && player.attributes.ambition >= 12.0
            {
                result.contract.contract_rejected = true;
                record_counter_offer(
                    player,
                    &proposal,
                    now,
                    min_acceptable_years,
                    RejectionReason::StatusBelowExpectation,
                );
                Self::emit_rejected_contract_offer(
                    player,
                    &proposal,
                    RejectionReason::StatusBelowExpectation,
                );
                log_rejection(player, &proposal, now);
                return;
            }
        }

        match &player.contract {
            Some(player_contract) => {
                let current_salary = player_contract.salary.max(1);
                let sweetener_total = proposal.signing_bonus + proposal.loyalty_bonus;
                let sweetener_ratio = sweetener_total as f32 / current_salary as f32;
                let has_clause = proposal.release_clause.is_some()
                    || proposal.relegation_release.is_some()
                    || proposal.non_promotion_release.is_some();

                let pkg_value = package_value(&proposal, player);
                let pkg_ratio = pkg_value as f32 / current_salary as f32;

                // Compare the package against what the player thinks they
                // are actually worth. Prefer the offer-time valuation
                // stamped by the renewal AI (real club/league reputation,
                // status, leverage); fall back to a freshly-computed
                // valuation with neutral reputation only when the offer
                // came from a legacy code path that didn't supply context.
                let age = DateUtils::age(player.birth_date, now);
                let market_interest = player.statuses.has(PlayerStatusType::Wnt)
                    || player.statuses.has(PlayerStatusType::Enq)
                    || player.statuses.has(PlayerStatusType::Bid);
                let months_remaining =
                    ((player_contract.expiration - now).num_days() / 30).max(0) as i32;
                let valuation = match (
                    proposal.valuation_expected_wage,
                    proposal.valuation_min_acceptable,
                ) {
                    (Some(expected), Some(min_acc)) => {
                        // Offer carried explicit valuation — use it.
                        ContractValuation {
                            expected_wage: expected,
                            min_acceptable: min_acc,
                            // Recompute remaining fields from the offer-time
                            // reputation so leverage / status_premium stay
                            // consistent with the offer's framing.
                            max_acceptable: ((expected as f32) * 1.30) as u32,
                            leverage: 0.3,
                            status_premium: 1.0,
                        }
                    }
                    _ => {
                        let valuation_ctx = ValuationContext {
                            age,
                            club_reputation_score: proposal
                                .valuation_club_reputation
                                .unwrap_or(0.5),
                            league_reputation: proposal
                                .valuation_league_reputation
                                .unwrap_or(5_000),
                            squad_status: proposal
                                .squad_status_promise
                                .clone()
                                .unwrap_or(player_contract.squad_status.clone()),
                            current_salary: player_contract.salary,
                            months_remaining,
                            has_market_interest: market_interest,
                        };
                        ContractValuation::evaluate(player, &valuation_ctx)
                    }
                };

                // Loyal veterans and players with strong release clauses /
                // signing bonuses tolerate a 15% underpay vs market.
                let loyalty = player.attributes.loyalty;
                let tolerance = if loyalty >= 16.0 {
                    0.85
                } else if has_clause || sweetener_ratio >= 0.3 {
                    0.90
                } else {
                    0.95
                };
                let market_floor = (valuation.expected_wage as f32 * tolerance) as u32;
                let badly_underpaid = pkg_value < market_floor;

                // Absolute walk-away floor. Below `min_acceptable` the player
                // refuses no matter how the offer is framed — a +20% raise on
                // a deeply underpaid deal is still a deeply underpaid deal.
                // Without this, the relative-raise gates below let a
                // budget-capped panic offer lock a star at a fraction of his
                // market value, and the salary-happiness model immediately
                // re-detected the gap on a contract he just signed.
                if pkg_value < valuation.min_acceptable {
                    result.contract.contract_rejected = true;
                    record_counter_offer(
                        player,
                        &proposal,
                        now,
                        min_acceptable_years,
                        RejectionReason::LowSalary,
                    );
                    Self::emit_rejected_contract_offer(
                        player,
                        &proposal,
                        RejectionReason::LowSalary,
                    );
                    log_rejection(player, &proposal, now);
                    return;
                }

                if proposal.salary > player_contract.salary {
                    let raise_ratio = proposal.salary as f32 / current_salary as f32;
                    let agent_delta =
                        agent.renewal_delta_with(raise_ratio, sweetener_ratio, has_clause);
                    if agent_delta < -4.0 && raise_ratio < 1.15 && sweetener_ratio < 0.20 {
                        result.contract.contract_rejected = true;
                        record_counter_offer(
                            player,
                            &proposal,
                            now,
                            min_acceptable_years,
                            RejectionReason::LowSalary,
                        );
                        Self::emit_rejected_contract_offer(
                            player,
                            &proposal,
                            RejectionReason::LowSalary,
                        );
                        log_rejection(player, &proposal, now);
                        return;
                    }
                    // Even with a token raise, if the package still leaves
                    // the player meaningfully below market they push back.
                    if badly_underpaid && raise_ratio < 1.20 && sweetener_ratio < 0.25 {
                        result.contract.contract_rejected = true;
                        record_counter_offer(
                            player,
                            &proposal,
                            now,
                            min_acceptable_years,
                            RejectionReason::LowSalary,
                        );
                        Self::emit_rejected_contract_offer(
                            player,
                            &proposal,
                            RejectionReason::LowSalary,
                        );
                        log_rejection(player, &proposal, now);
                        return;
                    }
                    let raise_ratio_now = proposal.salary as f32 / current_salary.max(1) as f32;
                    let proposal_years = proposal.years;
                    accept_and_clear(player, proposal, now);
                    let mut cctx = ContractEventContext::new(ContractEventKind::Renewed)
                        .with_wage_vs_previous(raise_ratio_now)
                        .with_years_remaining(proposal_years);
                    if raise_ratio_now >= 1.20 {
                        cctx = cctx.with_evidence(ContractEventEvidence::SquadStatusUpgrade);
                    }
                    if player.attributes.loyalty >= 15.0 {
                        cctx = cctx.with_evidence(ContractEventEvidence::HighLoyalty);
                    }
                    let happiness_ctx = HappinessEventContext::new(
                        HappinessEventCause::Other,
                        HappinessEventSeverity::Moderate,
                        HappinessEventScope::Boardroom,
                    )
                    .with_contract_context(cctx);
                    let mag = HappinessConfig::default()
                        .catalog
                        .magnitude(HappinessEventType::ContractRenewal);
                    player.happiness.add_event_with_context(
                        HappinessEventType::ContractRenewal,
                        mag,
                        None,
                        happiness_ctx,
                    );
                    player.happiness.factors.salary_satisfaction = 0.0;
                    player.happiness.last_salary_negotiation = Some(now);
                } else if proposal.salary >= player_contract.salary {
                    let loyalty = player.attributes.loyalty;
                    let morale = player.happiness.morale;
                    let negotiation = proposal.negotiation_skill as f32;

                    let accept_score = loyalty
                        + (morale / 10.0)
                        + negotiation
                        + agent.renewal_delta_with(1.0, sweetener_ratio, has_clause);
                    if accept_score >= 20.0 || pkg_ratio >= 1.10 {
                        let proposal_years = proposal.years;
                        accept_and_clear(player, proposal, now);
                        let cctx = ContractEventContext::new(ContractEventKind::OfferReceived)
                            .with_wage_vs_previous(1.0)
                            .with_years_remaining(proposal_years);
                        let happiness_ctx = HappinessEventContext::new(
                            HappinessEventCause::Other,
                            HappinessEventSeverity::Minor,
                            HappinessEventScope::Boardroom,
                        )
                        .with_contract_context(cctx);
                        player.happiness.add_event_with_context(
                            HappinessEventType::ContractOffer,
                            2.0,
                            None,
                            happiness_ctx,
                        );
                    } else {
                        result.contract.contract_rejected = true;
                        let reason = if !has_clause && player.attributes.ambition >= 14.0 {
                            RejectionReason::NoReleaseClause
                        } else if sweetener_ratio < 0.05 {
                            RejectionReason::NoSweetener
                        } else {
                            RejectionReason::LowSalary
                        };
                        record_counter_offer(player, &proposal, now, min_acceptable_years, reason);
                        if reason == RejectionReason::NoReleaseClause {
                            let demanded = player
                                .pending_contract_ask
                                .as_ref()
                                .and_then(|a| a.demanded_release_clause)
                                .map(|v| v as u64);
                            Self::maybe_emit_release_clause_demanded(player, now, demanded);
                        }
                        Self::emit_rejected_contract_offer(player, &proposal, reason);
                        log_rejection(player, &proposal, now);
                    }
                } else {
                    let loyalty = player.attributes.loyalty;
                    let negotiation = proposal.negotiation_skill as f32;
                    let salary_ratio = proposal.salary as f32 / current_salary as f32;
                    // Sweeteners + bonuses + clauses make a pay-cut palatable: if
                    // the total package matches current salary, relax the gates.
                    let effective_ratio = pkg_ratio;
                    let eligible = if effective_ratio >= 0.98 {
                        loyalty >= 10.0 && negotiation >= 10.0
                    } else {
                        salary_ratio >= 0.85 && loyalty >= 15.0 && negotiation >= 15.0
                    };
                    if eligible {
                        let pay_cut_ratio = proposal.salary as f32 / current_salary.max(1) as f32;
                        let proposal_years = proposal.years;
                        accept_and_clear(player, proposal, now);
                        let mut cctx =
                            ContractEventContext::new(ContractEventKind::LoyaltyDiscountAccepted)
                                .with_wage_vs_previous(pay_cut_ratio)
                                .with_years_remaining(proposal_years);
                        if player.attributes.loyalty >= 15.0 {
                            cctx = cctx.with_evidence(ContractEventEvidence::HighLoyalty);
                        }
                        let happiness_ctx = HappinessEventContext::new(
                            HappinessEventCause::Other,
                            HappinessEventSeverity::Minor,
                            HappinessEventScope::Boardroom,
                        )
                        .with_contract_context(cctx);
                        player.happiness.add_event_with_context(
                            HappinessEventType::ContractOffer,
                            1.0,
                            None,
                            happiness_ctx,
                        );
                    } else {
                        result.contract.contract_rejected = true;
                        record_counter_offer(
                            player,
                            &proposal,
                            now,
                            min_acceptable_years,
                            RejectionReason::LowSalary,
                        );
                        Self::emit_rejected_contract_offer(
                            player,
                            &proposal,
                            RejectionReason::LowSalary,
                        );
                        log_rejection(player, &proposal, now);
                    }
                }
            }
            None => {
                // Free agent — uses the unified valuation to gate acceptance.
                // Honor the offer-time context when the proposing club
                // stamped it: a fourth-tier club's offer must be judged
                // against fourth-tier expectations, not a hardcoded mid-tier
                // league — and an elite club shouldn't get a mid-tier
                // discount either.
                let age = DateUtils::age(player.birth_date, now);
                let valuation = match (
                    proposal.valuation_expected_wage,
                    proposal.valuation_min_acceptable,
                ) {
                    (Some(expected), Some(min_acc)) => ContractValuation {
                        expected_wage: expected,
                        min_acceptable: min_acc,
                        max_acceptable: ((expected as f32) * 1.30) as u32,
                        leverage: 0.3,
                        status_premium: 1.0,
                    },
                    _ => {
                        let ctx = ValuationContext {
                            age,
                            club_reputation_score: proposal
                                .valuation_club_reputation
                                .unwrap_or(0.5),
                            league_reputation: proposal
                                .valuation_league_reputation
                                .unwrap_or(5_000),
                            squad_status: proposal
                                .squad_status_promise
                                .clone()
                                .unwrap_or(PlayerSquadStatus::FirstTeamRegular),
                            current_salary: 0,
                            months_remaining: 0,
                            has_market_interest: player.statuses.has(PlayerStatusType::Wnt)
                                || player.statuses.has(PlayerStatusType::Enq)
                                || player.statuses.has(PlayerStatusType::Bid),
                        };
                        ContractValuation::evaluate(player, &ctx)
                    }
                };

                let meets_floor = proposal.salary >= valuation.min_acceptable;

                // A difficult character holds out for his full price unless a
                // skilled negotiator manages him — but he is not frozen out of
                // the market entirely: money he can't argue with still signs.
                let behaviour_pass = match player.behaviour.state {
                    PersonBehaviourState::Poor => {
                        proposal.negotiation_skill >= 12
                            || proposal.salary >= valuation.expected_wage
                    }
                    PersonBehaviourState::Normal => proposal.negotiation_skill >= 8,
                    PersonBehaviourState::Good => true,
                };

                if meets_floor && behaviour_pass {
                    accept_and_clear(player, proposal, now);
                } else {
                    result.contract.contract_rejected = true;
                    let reason = if !meets_floor {
                        RejectionReason::LowSalary
                    } else {
                        RejectionReason::AmbitionMismatch
                    };
                    record_counter_offer(player, &proposal, now, min_acceptable_years, reason);
                    Self::emit_rejected_contract_offer(player, &proposal, reason);
                    log_rejection(player, &proposal, now);
                }
            }
        }
    }

    /// Emit a visible [`RejectedContractOffer`] event after the player /
    /// agent turned down a proposal. The morale hit lives here rather
    /// than at every reject branch above so the cause-evidence wiring
    /// stays in one place. Cooldowned so a club that re-offers the same
    /// week doesn't double-fire the event.
    ///
    /// [`RejectedContractOffer`]: HappinessEventType::RejectedContractOffer
    fn emit_rejected_contract_offer(
        player: &mut Player,
        proposal: &PlayerContractProposal,
        reason: RejectionReason,
    ) {
        // Don't restate the same rejection in a tight window — the club
        // hasn't had a chance to come back with a meaningfully different
        // offer yet.
        if player
            .happiness
            .has_recent_event(&HappinessEventType::RejectedContractOffer, 21)
        {
            return;
        }
        let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let wage_ratio = if current_salary > 0 {
            proposal.salary as f32 / current_salary as f32
        } else {
            1.0
        };

        let evidence = match reason {
            RejectionReason::LowSalary | RejectionReason::NoSweetener => {
                ContractEventEvidence::RejectedOverWage
            }
            RejectionReason::StatusBelowExpectation => ContractEventEvidence::RejectedOverRole,
            RejectionReason::NoReleaseClause => ContractEventEvidence::RejectedOverReleaseClause,
            RejectionReason::ShortContract => ContractEventEvidence::RejectedOverLength,
            RejectionReason::AmbitionMismatch => ContractEventEvidence::RejectedOverAmbition,
        };

        let mut cctx = ContractEventContext::new(ContractEventKind::OfferRejectedByPlayer)
            .with_wage_vs_previous(wage_ratio)
            .with_years_remaining(proposal.years)
            .with_evidence(evidence);
        if player.attributes.ambition >= 14.0 {
            cctx = cctx.with_evidence(ContractEventEvidence::HighAmbition);
        }
        if player.attributes.loyalty <= 7.0 {
            cctx = cctx.with_evidence(ContractEventEvidence::LowLoyalty);
        }
        let has_other_interest = player.statuses.has(PlayerStatusType::Wnt)
            || player.statuses.has(PlayerStatusType::Enq)
            || player.statuses.has(PlayerStatusType::Bid);
        if has_other_interest {
            cctx = cctx.with_evidence(ContractEventEvidence::HasOtherInterest);
        }

        // Magnitude scales by the dominant reason — ambition / role
        // rejections sting more than a wage haggle.
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::RejectedContractOffer);
        let reason_mul = match reason {
            RejectionReason::AmbitionMismatch | RejectionReason::StatusBelowExpectation => 1.25,
            RejectionReason::NoReleaseClause => 1.10,
            RejectionReason::ShortContract => 0.85,
            _ => 1.0,
        };
        let magnitude = base * reason_mul;

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Boardroom,
        )
        .with_contract_context(cctx);
        player.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::RejectedContractOffer,
            magnitude,
            None,
            happiness_ctx,
            21,
        );
    }

    /// Promote a private "no release clause" rejection into a visible,
    /// first-class [`ReleaseClauseDemanded`] event when there is real
    /// leverage behind the demand. Suppresses the loyal-and-uncourted
    /// case and the ageing-low-rep case (unless the agent is greedy), and
    /// is cooldown-gated so repeated rejected offers don't spam it.
    ///
    /// [`ReleaseClauseDemanded`]: HappinessEventType::ReleaseClauseDemanded
    fn maybe_emit_release_clause_demanded(
        player: &mut Player,
        now: NaiveDate,
        demanded_clause: Option<u64>,
    ) {
        if player
            .happiness
            .has_recent_event(&HappinessEventType::ReleaseClauseDemanded, 90)
        {
            return;
        }

        let has_other_interest = player.statuses.has(PlayerStatusType::Wnt)
            || player.statuses.has(PlayerStatusType::Enq)
            || player.statuses.has(PlayerStatusType::Bid);
        let loyalty = player.attributes.loyalty;
        // A loyal, uncourted player asking for a clause is private noise,
        // not a leverage event — keep it as the soft life-sim desire.
        if loyalty >= 17.0 && !has_other_interest {
            return;
        }

        let age = DateUtils::age(player.birth_date, now);
        let world_rep = player.player_attributes.world_reputation;
        let agent = PlayerAgent::for_player(player);
        // Ageing, lower-profile players have no exit-path leverage unless
        // a greedy agent is driving it.
        if age >= 32 && world_rep < 3000 && agent.greed < 0.6 {
            return;
        }

        // External interest in the recent window is the clearest leverage
        // signal behind the demand.
        let used_leverage = player
            .happiness
            .has_recent_event(&HappinessEventType::InterestFromBiggerClub, 90)
            || player
                .happiness
                .has_recent_event(&HappinessEventType::WantedByBiggerClub, 90)
            || player
                .happiness
                .has_recent_event(&HappinessEventType::TransferBidRejected, 90);

        let mut cctx = ContractEventContext::new(ContractEventKind::ReleaseClauseDemanded)
            .with_evidence(ContractEventEvidence::ReleaseClauseDemanded)
            .with_agent_pressure(agent.greed);
        if let Some(value) = demanded_clause {
            cctx = cctx.with_demanded_release_clause(value);
        }
        if player.attributes.ambition >= 14.0 {
            cctx = cctx.with_evidence(ContractEventEvidence::HighAmbition);
        }
        if has_other_interest {
            cctx = cctx.with_evidence(ContractEventEvidence::HasOtherInterest);
        }
        if used_leverage {
            cctx = cctx.with_evidence(ContractEventEvidence::UsedExternalInterestAsLeverage);
        }

        let magnitude = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ReleaseClauseDemanded);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Boardroom,
        )
        .with_contract_context(cctx);
        player.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::ReleaseClauseDemanded,
            magnitude,
            None,
            happiness_ctx,
            90,
        );
    }

    /// Player/agent has a minimum acceptable contract length.
    /// High-reputation players with interest from other clubs won't accept short deals.
    /// Loyal or older players are more flexible.
    pub(crate) fn player_minimum_years(player: &Player, now: NaiveDate) -> u8 {
        let age = DateUtils::age(player.birth_date, now);
        let reputation = player.player_attributes.current_reputation;
        let ambition = player.attributes.ambition;
        let loyalty = player.attributes.loyalty;

        let mut min_years: f32 = 1.0;

        if reputation > 7000 {
            min_years += 2.0;
        } else if reputation > 4000 {
            min_years += 1.0;
        } else if reputation > 2000 {
            min_years += 0.5;
        }

        if ambition > 15.0 {
            min_years += 1.0;
        } else if ambition > 10.0 {
            min_years += 0.5;
        }

        if loyalty > 15.0 {
            min_years -= 1.5;
        } else if loyalty > 10.0 {
            min_years -= 0.5;
        }

        let has_interest = player.statuses.has(PlayerStatusType::Wnt)
            || player.statuses.has(PlayerStatusType::Enq)
            || player.statuses.has(PlayerStatusType::Bid);
        if has_interest {
            min_years += 1.0;
        }

        // Ageing veterans scale their demand down — fewer suitors on the
        // open market, and their agent knows it. Without this, 35+ stars
        // auto-reject every offer on length alone.
        if age >= 34 {
            min_years -= 2.0;
        } else if age >= 32 {
            min_years -= 1.0;
        } else if age >= 30 {
            min_years -= 0.5;
        }

        if age < 24 {
            min_years += 0.5;
        }

        (min_years.round() as u8).clamp(1, 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn build(ambition: f32, loyalty: f32, world_rep: i16) -> Player {
        let attrs = PersonAttributes {
            adaptability: 12.0,
            ambition,
            controversy: 5.0,
            loyalty,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 12.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        };
        let mut pa = PlayerAttributes::default();
        pa.world_reputation = world_rep;
        pa.current_reputation = world_rep;
        pa.current_ability = 140;
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Player".into()))
            .birth_date(d(1998, 1, 1))
            .country_id(1)
            .attributes(attrs)
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(pa)
            .build()
            .unwrap()
    }

    fn count_demanded(player: &Player) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ReleaseClauseDemanded)
            .count()
    }

    #[test]
    fn ambitious_player_with_interest_demands_clause() {
        let now = d(2026, 5, 30);
        let mut p = build(15.0, 10.0, 5_000);
        p.statuses.add(now, PlayerStatusType::Wnt);
        ProcessContractHandler::maybe_emit_release_clause_demanded(&mut p, now, Some(20_000_000));
        assert_eq!(
            count_demanded(&p),
            1,
            "leverage + ambition emits the demand"
        );
    }

    #[test]
    fn loyal_uncourted_player_does_not_demand() {
        let now = d(2026, 5, 30);
        let mut p = build(15.0, 18.0, 5_000);
        ProcessContractHandler::maybe_emit_release_clause_demanded(&mut p, now, Some(20_000_000));
        assert_eq!(
            count_demanded(&p),
            0,
            "a loyal player with no outside interest keeps the desire private"
        );
    }

    #[test]
    fn release_clause_demand_respects_cooldown() {
        let now = d(2026, 5, 30);
        let mut p = build(15.0, 10.0, 5_000);
        p.statuses.add(now, PlayerStatusType::Wnt);
        ProcessContractHandler::maybe_emit_release_clause_demanded(&mut p, now, Some(20_000_000));
        ProcessContractHandler::maybe_emit_release_clause_demanded(&mut p, now, Some(20_000_000));
        assert_eq!(count_demanded(&p), 1, "90-day cooldown blocks the repeat");
    }

    fn rejected_count(player: &Player) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::RejectedContractOffer)
            .count()
    }

    fn make_proposal(salary: u32, years: u8) -> PlayerContractProposal {
        PlayerContractProposal::basic(salary, years, 10, 0, 0, None)
    }

    #[test]
    fn rejected_contract_offer_records_reason_evidence() {
        let mut p = build(15.0, 10.0, 5_000);
        let proposal = make_proposal(50_000, 2);
        ProcessContractHandler::emit_rejected_contract_offer(
            &mut p,
            &proposal,
            RejectionReason::AmbitionMismatch,
        );
        assert_eq!(rejected_count(&p), 1);
        let stored = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::RejectedContractOffer)
            .unwrap();
        let cc = stored
            .context
            .as_ref()
            .and_then(|c| c.contract_context.as_ref())
            .expect("contract context attached");
        assert!(matches!(
            cc.kind,
            crate::ContractEventKind::OfferRejectedByPlayer
        ));
        assert!(
            cc.evidence
                .contains(&crate::ContractEventEvidence::RejectedOverAmbition),
            "rejection reason must surface as evidence: {:?}",
            cc.evidence
        );
    }

    #[test]
    fn rejected_contract_offer_cooldown_prevents_double_fire() {
        let mut p = build(15.0, 10.0, 5_000);
        let proposal = make_proposal(50_000, 2);
        ProcessContractHandler::emit_rejected_contract_offer(
            &mut p,
            &proposal,
            RejectionReason::LowSalary,
        );
        ProcessContractHandler::emit_rejected_contract_offer(
            &mut p,
            &proposal,
            RejectionReason::LowSalary,
        );
        assert_eq!(
            rejected_count(&p),
            1,
            "21-day cooldown blocks a same-week refire"
        );
    }
}
