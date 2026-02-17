use crate::shared::CurrencyValue;

const DEFAULT_TRANSFER_LIST_SIZE: usize = 10;

#[derive(Debug)]
pub struct Transfers {
    items: Vec<TransferItem>,
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

    pub fn items(&self) -> &[TransferItem] {
        &self.items
    }
}

#[derive(Debug)]
pub struct TransferItem {
    pub player_id: u32,
    pub amount: CurrencyValue,
}

impl TransferItem {
    pub fn new(player_id: u32, amount: CurrencyValue) -> TransferItem {
        TransferItem { player_id, amount }
    }
}
