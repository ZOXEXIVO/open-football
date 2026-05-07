use std::collections::HashMap;

#[derive(Clone)]
pub struct TransferPool<T> {
    pool: HashMap<u32, Vec<T>>,
}

impl<T> Default for TransferPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TransferPool<T> {
    pub fn new() -> Self {
        TransferPool {
            pool: HashMap::new(),
        }
    }

    pub fn push_transfer(&mut self, item: T, club_id: u32) {
        self.pool.entry(club_id).or_default().push(item);
    }

    pub fn pull_transfers(&mut self, club_id: u32) -> Option<Vec<T>> {
        self.pool.remove(&club_id)
    }
}
