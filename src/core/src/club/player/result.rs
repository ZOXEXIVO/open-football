use crate::club::Player;
use crate::club::player::injury::InjuryType;
use crate::simulator::SimulatorData;

pub struct PlayerCollectionResult {
    pub players: Vec<PlayerResult>,
    pub outgoing_players: Vec<Player>,
}

impl PlayerCollectionResult {
    pub fn new(players: Vec<PlayerResult>, outgoing_players: Vec<Player>) -> Self {
        PlayerCollectionResult {
            players,
            outgoing_players,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        for player in &self.players {
            player.process(data);
        }
    }
}

pub struct PlayerResult {
    pub player_id: u32,
    pub contract: PlayerContractResult,
    pub is_transfer_requested: bool,
    pub transfer_requests: Vec<u32>,
    pub injury_occurred: Option<InjuryType>,
    pub injury_recovered: bool,
    pub unhappy: bool,
    pub wants_to_leave: bool,
}

pub struct PlayerContractResult {
    pub no_contract: bool,
    pub contract_rejected: bool,
    pub want_improve_contract: bool,
    pub want_extend_contract: bool,
}

impl PlayerResult {
    pub fn new(player_id: u32) -> Self {
        PlayerResult {
            player_id,
            contract: PlayerContractResult {
                no_contract: false,
                contract_rejected: false,
                want_improve_contract: false,
                want_extend_contract: false,
            },
            is_transfer_requested: false,
            transfer_requests: Vec::new(),
            injury_occurred: None,
            injury_recovered: false,
            unhappy: false,
            wants_to_leave: false,
        }
    }

    pub fn process(&self, _: &mut SimulatorData) {}

    pub fn request_transfer(&mut self, player_id: u32) {
        self.transfer_requests.push(player_id);
    }

    pub fn has_contract_actions(&self) -> bool {
        self.contract.no_contract
            || self.contract.contract_rejected
            || self.contract.want_extend_contract
            || self.contract.want_improve_contract
    }
}
