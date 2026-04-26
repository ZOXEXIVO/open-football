use crate::club::player::agent::PlayerAgent;
use crate::club::player::calculators::{
    expected_annual_value, package_inputs_from_proposal, ContractValuation, ValuationContext,
};
use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};
use crate::handlers::AcceptContractHandler;
use crate::utils::DateUtils;
use crate::{
    HappinessEventType, PersonBehaviourState, Player, PlayerContractProposal, PlayerResult,
    PlayerSquadStatus, PlayerStatusType,
};
use chrono::NaiveDate;

pub struct ProcessContractHandler;

/// Written into `decision_history.decision` whenever the player turns
/// down a proposal. The ContractRenewalManager reads this back to tell
/// "still waiting" from "already said no", applies a longer cooldown,
/// caps the retry count, and escalates terms on the next attempt.
pub const RENEWAL_REJECTED_LABEL: &str = "dec_contract_renewal_rejected";

fn log_rejection(player: &mut Player, proposal: &PlayerContractProposal, now: NaiveDate) {
    let movement = format!(
        "{}y · ${}/y",
        proposal.years,
        crate::utils::FormattingUtils::format_money(proposal.salary as f64)
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
    // Greedy agents push the next ask up sharply; loyal agents hold roughly steady.
    let greed_multiplier = 1.0 + agent.greed * 0.25;
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
            if status_rank(&promised) < status_rank(&current)
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
                let market_interest = player.statuses.get().iter().any(|s| {
                    matches!(
                        s,
                        PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
                    )
                });
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
                        log_rejection(player, &proposal, now);
                        return;
                    }
                    accept_and_clear(player, proposal, now);
                    player
                        .happiness
                        .add_event_default(HappinessEventType::ContractRenewal);
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
                        accept_and_clear(player, proposal, now);
                        player.happiness.add_event(HappinessEventType::ContractOffer, 2.0);
                    } else {
                        result.contract.contract_rejected = true;
                        let reason = if !has_clause && player.attributes.ambition >= 14.0 {
                            RejectionReason::NoReleaseClause
                        } else if sweetener_ratio < 0.05 {
                            RejectionReason::NoSweetener
                        } else {
                            RejectionReason::LowSalary
                        };
                        record_counter_offer(
                            player,
                            &proposal,
                            now,
                            min_acceptable_years,
                            reason,
                        );
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
                        accept_and_clear(player, proposal, now);
                        player.happiness.add_event(HappinessEventType::ContractOffer, 1.0);
                    } else {
                        result.contract.contract_rejected = true;
                        record_counter_offer(
                            player,
                            &proposal,
                            now,
                            min_acceptable_years,
                            RejectionReason::LowSalary,
                        );
                        log_rejection(player, &proposal, now);
                    }
                }
            }
            None => {
                // Free agent — uses the unified valuation to gate acceptance.
                let age = DateUtils::age(player.birth_date, now);
                let ctx = ValuationContext {
                    age,
                    club_reputation_score: 0.5,
                    league_reputation: 5_000,
                    squad_status: proposal
                        .squad_status_promise
                        .clone()
                        .unwrap_or(PlayerSquadStatus::FirstTeamRegular),
                    current_salary: 0,
                    months_remaining: 0,
                    has_market_interest: player.statuses.get().iter().any(|s| {
                        matches!(
                            s,
                            PlayerStatusType::Wnt
                                | PlayerStatusType::Enq
                                | PlayerStatusType::Bid
                        )
                    }),
                };
                let valuation = ContractValuation::evaluate(player, &ctx);

                let meets_floor = proposal.salary >= valuation.min_acceptable;

                let behaviour_pass = match player.behaviour.state {
                    PersonBehaviourState::Poor => proposal.negotiation_skill >= 16,
                    PersonBehaviourState::Normal => proposal.negotiation_skill >= 8,
                    PersonBehaviourState::Good => true,
                };

                if meets_floor && behaviour_pass {
                    accept_and_clear(player, proposal, now);
                } else {
                    result.contract.contract_rejected = true;
                    record_counter_offer(
                        player,
                        &proposal,
                        now,
                        min_acceptable_years,
                        if !meets_floor {
                            RejectionReason::LowSalary
                        } else {
                            RejectionReason::AmbitionMismatch
                        },
                    );
                    log_rejection(player, &proposal, now);
                }
            }
        }
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

        let has_interest = player.statuses.get().iter().any(|s| {
            matches!(
                s,
                PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
            )
        });
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
