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
                    AcceptContractHandler::process(player, proposal, now);
                    // Wage increase boosts happiness
                    player.happiness.add_event(HappinessEventType::WageIncrease, 5.0);
                    player.happiness.factors.salary_satisfaction = 0.0;
                } else {
                    result.contract.contract_rejected = true;
                }
            }
            None => match player.behaviour.state {
                PersonBehaviourState::Poor => {
                    result.contract.contract_rejected = true;
                }
                PersonBehaviourState::Normal => {}
                PersonBehaviourState::Good => {
                    AcceptContractHandler::process(player, proposal, now);
                }
            },
        }
    }
}
