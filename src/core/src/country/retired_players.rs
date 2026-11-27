use crate::Player;
use crate::shared::append_log::Iter;
use crate::shared::{AppendLog, RosterRevision};

/// The players who have retired from a country's football. They are only
/// ever added to, so a world copy shares them rather than duplicating them.
#[derive(Debug, Clone)]
pub struct RetiredPlayers {
    players: AppendLog<Player>,
    revision: RosterRevision,
}

impl Default for RetiredPlayers {
    fn default() -> Self {
        Self::new()
    }
}

impl RetiredPlayers {
    pub fn new() -> Self {
        RetiredPlayers {
            players: AppendLog::new(),
            revision: RosterRevision::next(),
        }
    }

    pub fn add(&mut self, player: Player) {
        self.players.push(player);
        self.revision = RosterRevision::next();
    }

    pub fn revision(&self) -> RosterRevision {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.players.len()
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    pub fn get(&self, slot: usize) -> Option<&Player> {
        self.players.get(slot)
    }

    pub fn iter(&self) -> Iter<'_, Player> {
        self.players.iter()
    }

    /// Unshares every stored player: for bootstrap passes only.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Player> {
        self.players.iter_mut()
    }
}

impl<'a> IntoIterator for &'a RetiredPlayers {
    type Item = &'a Player;
    type IntoIter = Iter<'a, Player>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
