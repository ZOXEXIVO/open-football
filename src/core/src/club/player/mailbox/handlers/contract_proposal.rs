use crate::club::player::agent::PlayerAgent;
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
    let movement = format!("{}y · ${}/y", proposal.years, proposal.salary);
    player.decision_history.add(
        now,
        movement,
        RENEWAL_REJECTED_LABEL.to_string(),
        String::new(),
    );
}

impl ProcessContractHandler {
    pub fn process(
        player: &mut Player,
        proposal: PlayerContractProposal,
        now: NaiveDate,
        result: &mut PlayerResult,
    ) {
        // Player evaluates contract length — ambitious/reputable players reject too-short deals
        let min_acceptable_years = Self::player_minimum_years(player, now);
        if proposal.years < min_acceptable_years {
            // Contract too short — player/agent rejects regardless of salary
            result.contract.contract_rejected = true;
            log_rejection(player, &proposal, now);
            return;
        }

        let agent = PlayerAgent::for_player(player);
        match &player.contract {
            Some(player_contract) => {
                if proposal.salary > player_contract.salary {
                    // Salary increase. A greedy agent may still reject a
                    // token raise — "we can do better on the open market."
                    let raise_ratio = proposal.salary as f32 / player_contract.salary.max(1) as f32;
                    let agent_delta = agent.renewal_delta(raise_ratio);
                    // Neutral delta accepts; very negative (greedy agent on
                    // small raise) flips to a rejection.
                    if agent_delta < -4.0 && raise_ratio < 1.15 {
                        result.contract.contract_rejected = true;
                        log_rejection(player, &proposal, now);
                        return;
                    }
                    AcceptContractHandler::process(player, proposal, now);
                    player.happiness.add_event(HappinessEventType::ContractRenewal, 5.0);
                    player.happiness.factors.salary_satisfaction = 0.0;
                    // Reset negotiation timer so cooldown restarts from this raise
                    player.happiness.last_salary_negotiation = Some(now);
                } else if proposal.salary >= player_contract.salary {
                    // Same salary — accept if player is loyal/happy enough or staff is persuasive
                    let loyalty = player.attributes.loyalty;
                    let morale = player.happiness.morale;
                    let negotiation = proposal.negotiation_skill as f32;

                    // loyalty (0-20) + morale_bonus (0-10) + negotiation (0-20)
                    // + agent lean (−6 to +6 roughly): a loyal agent nudges
                    // them over the line, a greedy one pulls them back.
                    let accept_score = loyalty
                        + (morale / 10.0)
                        + negotiation
                        + agent.renewal_delta(1.0);
                    if accept_score >= 20.0 {
                        AcceptContractHandler::process(player, proposal, now);
                        player.happiness.add_event(HappinessEventType::ContractOffer, 2.0);
                    } else {
                        result.contract.contract_rejected = true;
                        log_rejection(player, &proposal, now);
                    }
                } else {
                    // Lower salary — accept only with very high loyalty + excellent negotiator
                    let loyalty = player.attributes.loyalty;
                    let negotiation = proposal.negotiation_skill as f32;

                    // Only accept if within 15% of current salary AND loyalty + negotiation high
                    let salary_ratio = proposal.salary as f32 / player_contract.salary as f32;
                    if salary_ratio >= 0.85 && loyalty >= 15.0 && negotiation >= 15.0 {
                        AcceptContractHandler::process(player, proposal, now);
                        player.happiness.add_event(HappinessEventType::ContractOffer, 1.0);
                    } else {
                        result.contract.contract_rejected = true;
                        log_rejection(player, &proposal, now);
                    }
                }
            }
            None => {
                // No contract — staff negotiation skill determines outcome
                match player.behaviour.state {
                    PersonBehaviourState::Poor => {
                        if proposal.negotiation_skill >= 16 {
                            AcceptContractHandler::process(player, proposal, now);
                        } else {
                            result.contract.contract_rejected = true;
                            log_rejection(player, &proposal, now);
                        }
                    }
                    PersonBehaviourState::Normal => {
                        if proposal.negotiation_skill >= 8 {
                            AcceptContractHandler::process(player, proposal, now);
                        } else {
                            result.contract.contract_rejected = true;
                            log_rejection(player, &proposal, now);
                        }
                    }
                    PersonBehaviourState::Good => {
                        AcceptContractHandler::process(player, proposal, now);
                    }
                }
            },
        }
    }

    /// Player/agent has a minimum acceptable contract length.
    /// High-reputation players with interest from other clubs won't accept short deals.
    /// Loyal or older players are more flexible.
    fn player_minimum_years(player: &Player, now: NaiveDate) -> u8 {
        let age = DateUtils::age(player.birth_date, now);
        let reputation = player.player_attributes.current_reputation;
        let ambition = player.attributes.ambition;
        let loyalty = player.attributes.loyalty;

        let mut min_years: f32 = 1.0;

        // High reputation → demands longer commitment
        if reputation > 7000 {
            min_years += 2.0;
        } else if reputation > 4000 {
            min_years += 1.0;
        } else if reputation > 2000 {
            min_years += 0.5;
        }

        // Ambitious players demand longer deals
        if ambition > 15.0 {
            min_years += 1.0;
        } else if ambition > 10.0 {
            min_years += 0.5;
        }

        // Loyal players accept shorter deals (trust the club)
        if loyalty > 15.0 {
            min_years -= 1.5;
        } else if loyalty > 10.0 {
            min_years -= 0.5;
        }

        // Other club interest → player has leverage, demands more
        let has_interest = player.statuses.get().iter().any(|s| {
            matches!(s, PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid)
        });
        if has_interest {
            min_years += 1.0;
        }

        // Older players accept shorter deals (fewer options)
        if age >= 33 {
            min_years -= 1.0;
        } else if age >= 30 {
            min_years -= 0.5;
        }

        // Young players with potential want security
        if age < 24 {
            min_years += 0.5;
        }

        (min_years.round() as u8).clamp(1, 4)
    }
}
