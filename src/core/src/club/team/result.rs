use crate::club::PlayerCollectionResult;
use crate::club::team::behaviour::TeamBehaviourResult;
use crate::simulator::SimulatorData;
use crate::{StaffCollectionResult, TeamTrainingResult};

pub struct TeamResult {
    pub team_id: u32,
    pub players: PlayerCollectionResult,
    pub staffs: StaffCollectionResult,
    pub behaviour: TeamBehaviourResult,
    pub training: TeamTrainingResult,
}

impl TeamResult {
    pub fn new(
        team_id: u32,
        players: PlayerCollectionResult,
        staffs: StaffCollectionResult,
        behaviour: TeamBehaviourResult,
        training: TeamTrainingResult,
    ) -> Self {
        TeamResult {
            team_id,
            players,
            staffs,
            behaviour,
            training,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        self.players.process(data);
        self.staffs.process(data);
        self.training.process(data);
        self.behaviour.process(data);
    }
}
