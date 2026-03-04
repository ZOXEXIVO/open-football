use crate::simulator::SimulatorData;

pub struct BoardResult {
    pub players_loan_listed: u32,
    pub players_transfer_listed: u32,
}

impl BoardResult {
    pub fn new() -> Self {
        BoardResult {
            players_loan_listed: 0,
            players_transfer_listed: 0,
        }
    }

    pub fn process(&self, _: &mut SimulatorData) {}
}
