use crate::{Player, PlayerCollectionResult, SimulatorData};

pub struct ClubAcademyResult {
    pub players: PlayerCollectionResult
}

impl ClubAcademyResult {
    pub fn new(players: PlayerCollectionResult) -> Self {
        ClubAcademyResult {
            players
        }
    }

    pub fn process(&self, _: &mut SimulatorData) {}
}

pub struct ProduceYouthPlayersResult {
    pub players: Vec<Player>,
}

impl ProduceYouthPlayersResult {
    pub fn new(players: Vec<Player>) -> Self {
        ProduceYouthPlayersResult { players }
    }

    pub fn process(&self, _: &mut SimulatorData) {}
}
