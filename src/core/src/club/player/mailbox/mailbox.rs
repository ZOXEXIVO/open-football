use crate::handlers::ProcessContractHandler;
use crate::{Player, PlayerMailboxResult, PlayerResult};
use chrono::NaiveDate;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct PlayerMessage {
    pub message_type: PlayerMessageType,
}

#[derive(Debug, Clone)]
pub enum PlayerMessageType {
    ContractProposal(PlayerContractProposal),
}

#[derive(Debug, Clone)]
pub struct PlayerContractProposal {
    pub salary: u32,
    pub years: u8,
    /// Staff negotiation skill (man_management, 0-20). Higher = more persuasive.
    pub negotiation_skill: u8,
}

#[derive(Debug, Clone)]
pub struct PlayerMailbox {
    messages: VecDeque<PlayerMessage>,
}

impl PlayerMailbox {
    pub fn new() -> Self {
        PlayerMailbox {
            messages: VecDeque::new(),
        }
    }

    pub fn process(
        player: &mut Player,
        player_result: &mut PlayerResult,
        now: NaiveDate,
    ) -> PlayerMailboxResult {
        let result = PlayerMailboxResult::new();

        let messages: Vec<PlayerMessage> = player.mailbox.messages.drain(..).collect();
        for message in messages {
            match message.message_type {
                PlayerMessageType::ContractProposal(proposal) => {
                    ProcessContractHandler::process(player, proposal, now, player_result);
                }
            }
        }

        result
    }

    pub fn push(&mut self, message: PlayerMessage) {
        self.messages.push_back(message);
    }
}
