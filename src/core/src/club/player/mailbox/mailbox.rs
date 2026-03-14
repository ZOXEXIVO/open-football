use crate::handlers::ProcessContractHandler;
use crate::{Player, PlayerMailboxResult, PlayerResult};
use chrono::NaiveDate;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PlayerMessage {
    pub message_type: PlayerMessageType,
}

#[derive(Debug, Clone)]
pub enum PlayerMessageType {
    Greeting,
    ContractProposal(PlayerContractProposal),
}

#[derive(Debug, Clone)]
pub struct PlayerContractProposal {
    pub salary: u32,
    pub years: u8,
    /// Staff negotiation skill (man_management, 0-20). Higher = more persuasive.
    pub negotiation_skill: u8,
}

#[derive(Debug)]
pub struct PlayerMailbox {
    messages: Mutex<VecDeque<PlayerMessage>>,
}

impl Clone for PlayerMailbox {
    fn clone(&self) -> Self {
        let messages = self.messages.lock().unwrap();
        PlayerMailbox {
            messages: Mutex::new(messages.clone()),
        }
    }
}

impl PlayerMailbox {
    pub fn new() -> Self {
        PlayerMailbox {
            messages: Mutex::new(VecDeque::new()),
        }
    }

    pub fn process(
        player: &mut Player,
        player_result: &mut PlayerResult,
        now: NaiveDate,
    ) -> PlayerMailboxResult {
        let result = PlayerMailboxResult::new();

        for message in player.mailbox.get() {
            match message.message_type {
                PlayerMessageType::Greeting => {}
                PlayerMessageType::ContractProposal(proposal) => {
                    ProcessContractHandler::process(player, proposal, now, player_result);
                }
            }
        }

        result
    }

    pub fn push(&self, message: PlayerMessage) {
        let mut messages = self.messages.lock().unwrap();
        messages.push_back(message);
    }

    pub fn get(&self) -> Vec<PlayerMessage> {
        let mut messages = self.messages.lock().unwrap();
        messages.drain(..).collect()
    }
}
