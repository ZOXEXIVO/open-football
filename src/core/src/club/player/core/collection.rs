use crate::PlayerPositionType;
use crate::club::player::player::Player;
use crate::club::{PlayerCollectionResult, PlayerResult};
use crate::context::GlobalContext;
use crate::utils::Logging;
use std::ops::Index;

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
            .iter_mut()
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
            if let Some(player) = self
                .players
                .iter_mut()
                .find(|p| p.id == *transfer_request_player_id)
            {
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

    /// Borrow a player by id. Returns `None` if the id is not in this
    /// collection — prefer this over open-coding `.players.iter().find(...)`
    /// so future changes (sorted lookup, HashMap cache, …) are a one-line fix.
    pub fn find(&self, player_id: u32) -> Option<&Player> {
        self.players.iter().find(|p| p.id == player_id)
    }

    /// Mutable variant of `find`.
    pub fn find_mut(&mut self, player_id: u32) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == player_id)
    }

    /// Shorthand for `self.players.iter()`.
    pub fn iter(&self) -> std::slice::Iter<'_, Player> {
        self.players.iter()
    }

    /// Shorthand for `self.players.iter_mut()`.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Player> {
        self.players.iter_mut()
    }

    /// Number of players in the collection.
    pub fn len(&self) -> usize {
        self.players.len()
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// Sum of `current_ability` across all players. Used by squad
    /// evaluation, transfer valuation and board expectation code — was
    /// previously re-inlined in 4+ places.
    pub fn current_ability_sum(&self) -> u32 {
        self.players
            .iter()
            .map(|p| p.player_attributes.current_ability as u32)
            .sum()
    }

    /// Average `current_ability` across the collection. Returns 0 when
    /// the collection is empty.
    pub fn current_ability_avg(&self) -> u8 {
        if self.players.is_empty() {
            0
        } else {
            (self.current_ability_sum() / self.players.len() as u32) as u8
        }
    }

    /// Descending-sorted CAs — used by squad-status calculation and
    /// several transfer/listing heuristics.
    pub fn current_abilities_desc(&self) -> Vec<u8> {
        let mut cas: Vec<u8> = self
            .players
            .iter()
            .map(|p| p.player_attributes.current_ability)
            .collect();
        cas.sort_unstable_by(|a, b| b.cmp(a));
        cas
    }
}

impl Index<u32> for PlayerCollection {
    type Output = Player;

    fn index(&self, player_id: u32) -> &Self::Output {
        self.players
            .iter()
            .find(|p| p.id == player_id)
            .expect(&format!("no player with id = {}", player_id))
    }
}
