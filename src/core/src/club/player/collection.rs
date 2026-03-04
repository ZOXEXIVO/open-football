use crate::club::player::player::Player;
use crate::club::{PlayerCollectionResult, PlayerResult};
use crate::context::GlobalContext;
use crate::utils::Logging;
use crate::PlayerPositionType;
use std::ops::Index;
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;

#[derive(Debug, Clone)]
pub struct PlayerCollection {
    pub players: Vec<Player>,
}

impl PlayerCollection {
    pub fn new(players: Vec<Player>) -> Self {
        PlayerCollection { players }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> PlayerCollectionResult {
        let player_results: Vec<PlayerResult> = self
            .players
            .par_iter_mut()
            .map(|player| {
                let message = &format!("simulate player: id: {}", player.id);
                Logging::estimate_result(
                    || player.simulate(ctx.with_player(Some(player.id))),
                    message,
                )
            })
            .collect();

        // Mark transfer-requested players as transfer-listed instead of removing them.
        // The transfer market will handle the actual move later.
        for transfer_request_player_id in player_results.iter().flat_map(|p| &p.transfer_requests) {
            if let Some(player) = self.players.iter_mut().find(|p| p.id == *transfer_request_player_id) {
                if let Some(ref mut contract) = player.contract {
                    contract.is_transfer_listed = true;
                }
            }
        }

        PlayerCollectionResult::new(player_results)
    }

    pub fn by_position(&self, position: &PlayerPositionType) -> Vec<&Player> {
        self.players
            .iter()
            .filter(|p| p.positions().contains(position))
            .collect()
    }

    pub fn add(&mut self, player: Player) {
        self.players.push(player);
    }

    pub fn add_range(&mut self, players: Vec<Player>) {
        for player in players {
            self.players.push(player);
        }
    }

    pub fn get_annual_salary(&self) -> u32 {
        self.players
            .iter()
            .filter_map(|p| p.contract.as_ref())
            .map(|c| c.salary)
            .sum::<u32>()
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.iter().map(|player| player).collect()
    }

    pub fn take_player(&mut self, player_id: &u32) -> Option<Player> {
        let player_idx = self.players.iter().position(|p| p.id == *player_id);
        match player_idx {
            Some(idx) => Some(self.players.remove(idx)),
            None => None,
        }
    }

    pub fn contains(&self, player_id: u32) -> bool {
        self.players.iter().any(|p| p.id == player_id)
    }
}

impl Index<u32> for PlayerCollection {
    type Output = Player;

    fn index(&self, player_id: u32) -> &Self::Output {
        self
            .players
            .iter()
            .find(|p| p.id == player_id)
            .expect(&format!("no player with id = {}", player_id))
    }
}
