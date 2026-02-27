use std::collections::HashMap;
use std::sync::Mutex;

pub struct TransferPool<T> {
    pool: Mutex<HashMap<u32, Vec<T>>>,
}

impl<T: Clone> Clone for TransferPool<T> {
    fn clone(&self) -> Self {
        let pool = self.pool.lock().unwrap();
        TransferPool {
            pool: Mutex::new(pool.clone()),
        }
    }
}

impl<T> TransferPool<T> {
    pub fn new() -> Self {
        TransferPool {
            pool: Mutex::new(HashMap::new()),
        }
    }

    pub fn push_transfer(&mut self, item: T, club_id: u32) {
        let mut inner_map = self.pool.lock().unwrap();

        let entry = inner_map.entry(club_id).or_insert_with(Vec::new);

        entry.push(item);
    }

    pub fn pull_transfers(&mut self, club_id: u32) -> Option<Vec<T>> {
        let mut inner_map = self.pool.lock().expect("lock poisoned");

        if !inner_map.contains_key(&club_id) {
            return None;
        }

        inner_map.remove(&club_id)
    }
}
