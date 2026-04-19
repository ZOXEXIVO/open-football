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
    /// One-off signature payment. Opens doors with greedy agents.
    pub signing_bonus: u32,
    /// Yearly loyalty bonus — rewards staying. Real clubs use this on
    /// long-service renewals where the base wage has hit the cap.
    pub loyalty_bonus: u32,
    /// Negotiated release clause. Players take short deals with a release
    /// more readily than long deals without one.
    pub release_clause: Option<u32>,
}

/// The player's own side of the negotiation. Stashed on the Player when a
/// proposal is turned down so the next offer from the club can converge on
/// terms the player would actually sign, rather than guessing again.
#[derive(Debug, Clone)]
pub struct PlayerContractAsk {
    pub desired_salary: u32,
    pub desired_years: u8,
    pub recorded_on: chrono::NaiveDate,
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
