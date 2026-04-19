use crate::club::player::agent::PlayerAgent;
use crate::club::player::mailbox::PlayerContractAsk;
use crate::handlers::AcceptContractHandler;
use crate::{HappinessEventType, PersonBehaviourState, Player, PlayerContractProposal, PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
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
/// the next proposal; without this the negotiation is blind on the club side.
fn record_counter_offer(
    player: &mut Player,
    proposal: &PlayerContractProposal,
    now: NaiveDate,
    min_years: u8,
) {
    let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
    let agent = PlayerAgent::for_player(player);

    let salary_anchor = proposal.salary.max(current_salary);
    let greed_multiplier = 1.0 + agent.greed * 0.25;
    let desired_salary = ((salary_anchor as f32) * greed_multiplier) as u32;

    let desired_years = proposal.years.max(min_years);

    player.pending_contract_ask = Some(PlayerContractAsk {
        desired_salary,
        desired_years,
        recorded_on: now,
    });
}

fn accept_and_clear(player: &mut Player, proposal: PlayerContractProposal, now: NaiveDate) {
    AcceptContractHandler::process(player, proposal, now);
    player.pending_contract_ask = None;
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
            record_counter_offer(player, &proposal, now, min_acceptable_years);
            log_rejection(player, &proposal, now);
            return;
        }

        let agent = PlayerAgent::for_player(player);
        match &player.contract {
            Some(player_contract) => {
                let current_salary = player_contract.salary.max(1);
                let sweetener_total = proposal.signing_bonus + proposal.loyalty_bonus;
                let sweetener_ratio = sweetener_total as f32 / current_salary as f32;
                let has_clause = proposal.release_clause.is_some();

                if proposal.salary > player_contract.salary {
                    let raise_ratio = proposal.salary as f32 / current_salary as f32;
                    let agent_delta = agent.renewal_delta_with(raise_ratio, sweetener_ratio, has_clause);
                    if agent_delta < -4.0 && raise_ratio < 1.15 && sweetener_ratio < 0.20 {
                        result.contract.contract_rejected = true;
                        record_counter_offer(player, &proposal, now, min_acceptable_years);
                        log_rejection(player, &proposal, now);
                        return;
                    }
                    accept_and_clear(player, proposal, now);
                    player.happiness.add_event(HappinessEventType::ContractRenewal, 5.0);
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
                    if accept_score >= 20.0 {
                        accept_and_clear(player, proposal, now);
                        player.happiness.add_event(HappinessEventType::ContractOffer, 2.0);
                    } else {
                        result.contract.contract_rejected = true;
                        record_counter_offer(player, &proposal, now, min_acceptable_years);
                        log_rejection(player, &proposal, now);
                    }
                } else {
                    let loyalty = player.attributes.loyalty;
                    let negotiation = proposal.negotiation_skill as f32;
                    let salary_ratio = proposal.salary as f32 / current_salary as f32;
                    // Sweeteners make a pay-cut palatable: if the total
                    // package (bonuses included) matches current salary,
                    // relax the loyalty/negotiation gates.
                    let effective_ratio = salary_ratio + sweetener_ratio;
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
                        record_counter_offer(player, &proposal, now, min_acceptable_years);
                        log_rejection(player, &proposal, now);
                    }
                }
            }
            None => {
                match player.behaviour.state {
                    PersonBehaviourState::Poor => {
                        if proposal.negotiation_skill >= 16 {
                            accept_and_clear(player, proposal, now);
                        } else {
                            result.contract.contract_rejected = true;
                            record_counter_offer(player, &proposal, now, min_acceptable_years);
                            log_rejection(player, &proposal, now);
                        }
                    }
                    PersonBehaviourState::Normal => {
                        if proposal.negotiation_skill >= 8 {
                            accept_and_clear(player, proposal, now);
                        } else {
                            result.contract.contract_rejected = true;
                            record_counter_offer(player, &proposal, now, min_acceptable_years);
                            log_rejection(player, &proposal, now);
                        }
                    }
                    PersonBehaviourState::Good => {
                        accept_and_clear(player, proposal, now);
                    }
                }
            },
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
            matches!(s, PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid)
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
