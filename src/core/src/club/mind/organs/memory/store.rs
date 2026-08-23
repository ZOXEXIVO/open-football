//! Fixed-capacity, allocation-free backing store for the memory organs.
//!
//! Every memory store has a hard cap by design — a mind that grows
//! without bound is not a mind, it is a log. Backing them with `[T; N]`
//! rather than `Vec<T>` means a player's whole memory is inline in the
//! `Player` struct: no heap allocation at construction, none on insert,
//! and a `Player::clone` (which the simulator does constantly) copies it
//! rather than chasing pointers.
//!
//! The cost is that `T` must be `Copy + Default`. Every memory record is
//! packed POD precisely so it can live here.

use std::cmp::Ordering;
use std::slice::{Iter, IterMut};

/// A bounded array-backed collection with `push`, iteration and
/// weakest-first eviction. Order is insertion order; callers that need
/// ranking sort a borrowed view rather than keeping the store sorted.
#[derive(Debug, Clone, Copy)]
pub struct FixedStore<T: Copy + Default, const N: usize> {
    items: [T; N],
    len: u8,
}

impl<T: Copy + Default, const N: usize> Default for FixedStore<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Default, const N: usize> FixedStore<T, N> {
    /// Compile-time guard: `len` is a `u8`, so a capacity past 255
    /// would silently truncate.
    const _CAPACITY_FITS_IN_U8: () = assert!(N <= u8::MAX as usize);

    pub fn new() -> Self {
        // Force evaluation of the capacity guard — associated consts in
        // a generic impl are lazy, so an unreferenced assert never fires.
        let () = Self::_CAPACITY_FITS_IN_U8;
        FixedStore {
            items: [T::default(); N],
            len: 0,
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        N
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() >= N
    }

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len()]
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        let len = self.len();
        &mut self.items[..len]
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        self.as_slice().iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        self.as_mut_slice().iter_mut()
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }

    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.as_mut_slice().get_mut(index)
    }

    /// Append if there is room. Returns `false` when full — the caller
    /// decides whether to evict (see [`Self::push_evicting`]) or drop
    /// the item.
    pub fn push(&mut self, item: T) -> bool {
        if self.is_full() {
            return false;
        }
        self.items[self.len()] = item;
        self.len += 1;
        true
    }

    /// Append, making room when full by evicting the item that `rank`
    /// scores lowest among those `evictable` admits. Returns the evicted
    /// item, or `None` if it fit without eviction.
    ///
    /// If nothing is evictable the new item is dropped and `None` is
    /// returned — a full store of protected entries refuses new ones
    /// rather than discarding a protected memory.
    pub fn push_evicting<R, E>(&mut self, item: T, rank: R, evictable: E) -> Option<T>
    where
        R: Fn(&T) -> f32,
        E: Fn(&T) -> bool,
    {
        if self.push(item) {
            return None;
        }

        let victim = self
            .as_slice()
            .iter()
            .enumerate()
            .filter(|(_, existing)| evictable(existing))
            .min_by(|(_, a), (_, b)| rank(a).partial_cmp(&rank(b)).unwrap_or(Ordering::Equal))
            .map(|(index, _)| index);

        match victim {
            Some(index) => {
                let evicted = self.items[index];
                self.items[index] = item;
                Some(evicted)
            }
            None => None,
        }
    }

    /// Drop every item failing `keep`, preserving relative order.
    pub fn retain<F: Fn(&T) -> bool>(&mut self, keep: F) {
        let mut write = 0usize;
        for read in 0..self.len() {
            if keep(&self.items[read]) {
                self.items[write] = self.items[read];
                write += 1;
            }
        }
        self.len = write as u8;
    }

    /// First item matching `pred`.
    pub fn find<F: Fn(&T) -> bool>(&self, pred: F) -> Option<&T> {
        self.as_slice().iter().find(|item| pred(item))
    }

    /// First item matching `pred`, mutably.
    pub fn find_mut<F: Fn(&T) -> bool>(&mut self, pred: F) -> Option<&mut T> {
        self.as_mut_slice().iter_mut().find(|item| pred(item))
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    struct Item {
        id: u8,
        weight: f32,
        protected: bool,
    }

    fn item(id: u8, weight: f32) -> Item {
        Item {
            id,
            weight,
            protected: false,
        }
    }

    fn protected(id: u8, weight: f32) -> Item {
        Item {
            id,
            weight,
            protected: true,
        }
    }

    #[test]
    fn push_fills_then_refuses() {
        let mut store: FixedStore<Item, 3> = FixedStore::new();
        assert!(store.push(item(1, 1.0)));
        assert!(store.push(item(2, 2.0)));
        assert!(store.push(item(3, 3.0)));
        assert!(!store.push(item(4, 4.0)), "a full store refuses a push");
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn eviction_takes_the_weakest() {
        let mut store: FixedStore<Item, 3> = FixedStore::new();
        store.push(item(1, 5.0));
        store.push(item(2, 0.5));
        store.push(item(3, 3.0));

        let evicted = store.push_evicting(item(4, 9.0), |i| i.weight, |_| true);
        assert_eq!(evicted.map(|i| i.id), Some(2), "the 0.5 item is weakest");
        assert!(store.find(|i| i.id == 4).is_some());
        assert!(store.find(|i| i.id == 2).is_none());
    }

    #[test]
    fn protected_entries_are_never_evicted() {
        let mut store: FixedStore<Item, 2> = FixedStore::new();
        store.push(protected(1, 0.1));
        store.push(protected(2, 0.2));

        let evicted = store.push_evicting(item(3, 9.0), |i| i.weight, |i| !i.protected);
        assert!(
            evicted.is_none(),
            "nothing evictable — the new item is dropped, not a protected one"
        );
        assert_eq!(store.len(), 2);
        assert!(store.find(|i| i.id == 3).is_none());
    }

    #[test]
    fn retain_preserves_order() {
        let mut store: FixedStore<Item, 5> = FixedStore::new();
        for id in 1..=5 {
            store.push(item(id, id as f32));
        }
        store.retain(|i| i.id % 2 == 1);
        let ids: Vec<u8> = store.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![1, 3, 5]);
    }

    #[test]
    fn store_is_copy_and_inline() {
        // The whole point of the array backing: no heap, and cloning a
        // player copies the memory rather than chasing a pointer.
        fn assert_copy<T: Copy>() {}
        assert_copy::<FixedStore<Item, 8>>();
    }
}
