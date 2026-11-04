//! The unattached pool and the money that follows a player out of a
//! squad.
//!
//! Three passes, all world-level because a club and the player who just
//! left it can sit in different countries:
//!
//! * [`intake`] — sweeping contractless players off rosters into
//!   `data.free_agents`.
//! * [`retirement`] — the monthly roll that resolves a long sit in the
//!   pool.
//! * [`loan_wages`] — the parent club's residual share of a loanee's
//!   primary contract.

mod intake;
mod loan_wages;
mod retirement;

pub use loan_wages::LoanWageSettlement;

use crate::Player;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::ops::{Deref, DerefMut};

/// The world's unattached players, each one's slot kept beside them so an
/// id resolves without walking the pool.
///
/// Players are edited in place through the slice; anything that adds,
/// removes or reorders them goes through the pool, which keeps the slots.
#[derive(Default)]
pub struct FreeAgentPool {
    players: Vec<Player>,
    slots: FxHashMap<u32, usize>,
}

impl FreeAgentPool {
    pub fn push(&mut self, player: Player) {
        self.slots.insert(player.id, self.players.len());
        self.players.push(player);
    }

    pub fn swap_remove(&mut self, slot: usize) -> Player {
        let player = self.players.swap_remove(slot);
        self.slots.remove(&player.id);
        if let Some(moved) = self.players.get(slot) {
            self.slots.insert(moved.id, slot);
        }
        player
    }

    pub fn slot_of(&self, player_id: u32) -> Option<usize> {
        self.slots.get(&player_id).copied()
    }

    pub fn find(&self, player_id: u32) -> Option<&Player> {
        self.slot_of(player_id).map(|slot| &self.players[slot])
    }

    /// Drop the pool across the rayon pool, as a world copy is dropped.
    pub fn release(self) {
        self.players.into_par_iter().for_each(drop);
    }
}

impl From<Vec<Player>> for FreeAgentPool {
    fn from(players: Vec<Player>) -> Self {
        let slots = players
            .iter()
            .enumerate()
            .map(|(slot, player)| (player.id, slot))
            .collect();
        FreeAgentPool { players, slots }
    }
}

impl Extend<Player> for FreeAgentPool {
    fn extend<I: IntoIterator<Item = Player>>(&mut self, players: I) {
        for player in players {
            self.push(player);
        }
    }
}

/// The pool is most of a world copy's players, so they are copied across
/// the rayon pool.
impl Clone for FreeAgentPool {
    fn clone(&self) -> Self {
        FreeAgentPool {
            players: self.players.par_iter().cloned().collect(),
            slots: self.slots.clone(),
        }
    }
}

impl Deref for FreeAgentPool {
    type Target = [Player];

    fn deref(&self) -> &[Player] {
        &self.players
    }
}

impl DerefMut for FreeAgentPool {
    fn deref_mut(&mut self) -> &mut [Player] {
        &mut self.players
    }
}

impl<'a> IntoIterator for &'a FreeAgentPool {
    type Item = &'a Player;
    type IntoIter = std::slice::Iter<'a, Player>;

    fn into_iter(self) -> Self::IntoIter {
        self.players.iter()
    }
}

impl<'a> IntoIterator for &'a mut FreeAgentPool {
    type Item = &'a mut Player;
    type IntoIter = std::slice::IterMut<'a, Player>;

    fn into_iter(self) -> Self::IntoIter {
        self.players.iter_mut()
    }
}

#[cfg(test)]
mod pool_slot_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    fn player(id: u32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Pool".to_string(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(1996, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 16,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn assert_slots_match_a_walk(pool: &FreeAgentPool) {
        assert_eq!(pool.slots.len(), pool.len());
        for (slot, player) in pool.iter().enumerate() {
            assert_eq!(pool.slot_of(player.id), Some(slot));
        }
    }

    #[test]
    fn slots_follow_push_extend_and_swap_remove() {
        let mut pool = FreeAgentPool::from(vec![player(1), player(2)]);
        pool.push(player(3));
        pool.extend([player(4), player(5)]);
        assert_slots_match_a_walk(&pool);

        let first = pool.swap_remove(0);
        assert_eq!(first.id, 1);
        assert_eq!(pool.slot_of(1), None);
        assert_eq!(pool.slot_of(5), Some(0), "the last player fills the slot");
        assert_slots_match_a_walk(&pool);

        let last = pool.swap_remove(pool.len() - 1);
        assert_eq!(last.id, 4);
        assert_eq!(pool.find(4).map(|p| p.id), None);
        assert_slots_match_a_walk(&pool);

        let clone = pool.clone();
        assert_slots_match_a_walk(&clone);
    }
}
