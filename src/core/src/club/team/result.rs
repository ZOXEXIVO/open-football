use crate::club::team::behaviour::TeamBehaviourResult;
use crate::club::PlayerCollectionResult;
use crate::shared::{Currency, CurrencyValue};
use crate::simulator::SimulatorData;
use crate::{PlayerStatusType, StaffCollectionResult, TeamTrainingResult};

pub struct TeamResult {
    pub team_id: u32,
    pub players: PlayerCollectionResult,
    pub staffs: StaffCollectionResult,
    pub behaviour: TeamBehaviourResult,
    pub training: TeamTrainingResult,
    pub staff_transfer_list: Vec<u32>,
}

impl TeamResult {
    pub fn new(
        team_id: u32,
        players: PlayerCollectionResult,
        staffs: StaffCollectionResult,
        behaviour: TeamBehaviourResult,
        training: TeamTrainingResult,
        staff_transfer_list: Vec<u32>,
    ) -> Self {
        TeamResult {
            team_id,
            players,
            staffs,
            behaviour,
            training,
            staff_transfer_list,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        let now = data.date.date();

        // Add players that staff responsible for transfers decided to list
        for &player_id in &self.staff_transfer_list {
            let value = data
                .player(player_id)
                .map(|p| p.value(now))
                .unwrap_or(0.0);

            let team = data.team_mut(self.team_id).unwrap();
            team.add_player_to_transfer_list(
                player_id,
                CurrencyValue {
                    amount: value,
                    currency: Currency::Usd,
                },
            );

            // Mark player with Lst status
            if let Some(player) = data.player_mut(player_id) {
                if !player.statuses.get().contains(&PlayerStatusType::Lst) {
                    player.statuses.add(now, PlayerStatusType::Lst);
                }
            }
        }

        self.players.process(data);
        self.staffs.process(data);
        self.training.process(data);
        self.behaviour.process(data);
    }
}
