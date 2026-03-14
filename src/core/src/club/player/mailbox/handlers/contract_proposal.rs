use crate::handlers::AcceptContractHandler;
use crate::{HappinessEventType, PersonBehaviourState, Player, PlayerContractProposal, PlayerResult};
use chrono::NaiveDate;

pub struct ProcessContractHandler;

impl ProcessContractHandler {
    pub fn process(
        player: &mut Player,
        proposal: PlayerContractProposal,
        now: NaiveDate,
        result: &mut PlayerResult,
    ) {
        match &player.contract {
            Some(player_contract) => {
                if proposal.salary > player_contract.salary {
                    // Salary increase — always accept
                    AcceptContractHandler::process(player, proposal, now);
                    player.happiness.add_event(HappinessEventType::WageIncrease, 5.0);
                    player.happiness.factors.salary_satisfaction = 0.0;
                } else if proposal.salary >= player_contract.salary {
                    // Same salary — accept if player is loyal/happy enough or staff is persuasive
                    let loyalty = player.attributes.loyalty;
                    let morale = player.happiness.morale;
                    let negotiation = proposal.negotiation_skill as f32;

                    // loyalty (0-20) + morale_bonus (0-10) + negotiation (0-20)
                    let accept_score = loyalty + (morale / 10.0) + negotiation;
                    if accept_score >= 20.0 {
                        AcceptContractHandler::process(player, proposal, now);
                        player.happiness.add_event(HappinessEventType::ContractOffer, 2.0);
                    } else {
                        result.contract.contract_rejected = true;
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
                    }
                }
            }
            None => {
                // No contract — staff negotiation skill determines outcome
                match player.behaviour.state {
                    PersonBehaviourState::Poor => {
                        // Poor behavior: only accept with exceptional negotiator
                        if proposal.negotiation_skill >= 16 {
                            AcceptContractHandler::process(player, proposal, now);
                        } else {
                            result.contract.contract_rejected = true;
                        }
                    }
                    PersonBehaviourState::Normal => {
                        // Normal behavior: accept with decent negotiator
                        if proposal.negotiation_skill >= 8 {
                            AcceptContractHandler::process(player, proposal, now);
                        } else {
                            // Still reject rather than limbo — gives club a clear signal
                            result.contract.contract_rejected = true;
                        }
                    }
                    PersonBehaviourState::Good => {
                        AcceptContractHandler::process(player, proposal, now);
                    }
                }
            },
        }
    }
}
