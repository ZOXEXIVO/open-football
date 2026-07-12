use crate::shared::CurrencyValue;
use rustc_hash::FxHashSet;

const DEFAULT_TRANSFER_LIST_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub struct Transfers {
    items: Vec<TransferItem>,
}

impl Default for Transfers {
    fn default() -> Self {
        Self::new()
    }
}

impl Transfers {
    pub fn new() -> Self {
        Transfers {
            items: Vec::with_capacity(DEFAULT_TRANSFER_LIST_SIZE),
        }
    }

    pub fn add(&mut self, item: TransferItem) {
        // Don't add duplicates
        if self.items.iter().any(|i| i.player_id == item.player_id) {
            return;
        }
        self.items.push(item);
    }

    pub fn remove(&mut self, player_id: u32) {
        self.items.retain(|item| item.player_id != player_id);
    }

    /// Drop every listing whose player id is in `player_ids` — one walk
    /// with set probes, the batched counterpart of [`Self::remove`] for
    /// world-sweep callers (a remove-per-id loop re-walked the list per
    /// signed player).
    pub fn remove_all(&mut self, player_ids: &FxHashSet<u32>) {
        self.items
            .retain(|item| !player_ids.contains(&item.player_id));
    }

    /// Remove and return the listing entry for `player_id`, if any. Used to
    /// migrate a player's asking-price entry between a club's own teams on an
    /// internal squad move (Main ↔ Reserve / B) so the listing follows the
    /// player instead of being dropped — market discovery is status-based and
    /// scans every team, so a desynced entry would only hide the asking price.
    pub fn take(&mut self, player_id: u32) -> Option<TransferItem> {
        let idx = self.items.iter().position(|i| i.player_id == player_id)?;
        Some(self.items.remove(idx))
    }

    pub fn contains(&self, player_id: u32) -> bool {
        self.items.iter().any(|i| i.player_id == player_id)
    }

    pub fn listed_player_ids(&self) -> Vec<u32> {
        self.items.iter().map(|i| i.player_id).collect()
    }

    pub fn items(&self) -> &[TransferItem] {
        &self.items
    }
}

#[derive(Debug, Clone)]
pub struct TransferItem {
    pub player_id: u32,
    pub amount: CurrencyValue,
}

impl TransferItem {
    pub fn new(player_id: u32, amount: CurrencyValue) -> TransferItem {
        TransferItem { player_id, amount }
    }
}
