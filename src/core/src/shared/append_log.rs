use std::sync::Arc;

/// Records kept in insertion order, in fixed-size chunks that copies share.
///
/// Cloning copies no record: it shares every chunk. A write unshares only the
/// chunk it lands in, so a world copy costs each log its open tail rather than
/// its whole history.
#[derive(Debug, Clone)]
pub struct AppendLog<T> {
    chunks: Vec<Arc<Vec<T>>>,
    /// Records trimmed off the front of the first chunk, still stored there.
    offset: usize,
    len: usize,
}

impl<T> Default for AppendLog<T> {
    fn default() -> Self {
        AppendLog {
            chunks: Vec::new(),
            offset: 0,
            len: 0,
        }
    }
}

impl<T> AppendLog<T> {
    /// Every chunk but the last holds exactly this many records.
    pub const CHUNK: usize = 64;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let (chunk, at) = self.locate(index);
        self.chunks[chunk].get(at)
    }

    pub fn first(&self) -> Option<&T> {
        self.get(0)
    }

    pub fn last(&self) -> Option<&T> {
        self.len.checked_sub(1).and_then(|index| self.get(index))
    }

    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            log: self,
            front: 0,
            back: self.len,
        }
    }

    /// Drops the oldest `count` records. Whole chunks go at once; the dead
    /// prefix of a chunk that is still partly live stays until it goes too.
    pub fn drop_front(&mut self, count: usize) {
        let count = count.min(self.len);
        self.len -= count;
        self.offset += count;
        let whole = self.offset / Self::CHUNK;
        self.chunks.drain(..whole);
        self.offset -= whole * Self::CHUNK;
        if self.len == 0 {
            self.chunks.clear();
            self.offset = 0;
        }
    }

    fn locate(&self, index: usize) -> (usize, usize) {
        let at = index + self.offset;
        (at / Self::CHUNK, at % Self::CHUNK)
    }
}

impl<T: Clone> AppendLog<T> {
    pub fn push(&mut self, record: T) {
        let tail_full = self
            .chunks
            .last()
            .is_none_or(|tail| tail.len() == Self::CHUNK);
        if tail_full {
            self.chunks.push(Arc::new(Vec::new()));
        }
        let tail = self.chunks.last_mut().expect("a tail chunk exists");
        Arc::make_mut(tail).push(record);
        self.len += 1;
    }

    /// Keeps the first `len` records, unsharing only the chunk it cuts.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        if len == 0 {
            *self = AppendLog::new();
            return;
        }
        let (chunk, at) = self.locate(len - 1);
        self.chunks.truncate(chunk + 1);
        let tail = self.chunks.last_mut().expect("the cut chunk is kept");
        if tail.len() > at + 1 {
            Arc::make_mut(tail).truncate(at + 1);
        }
        self.len = len;
    }

    /// Unshares the one chunk that holds the record.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        let (chunk, at) = self.locate(index);
        Arc::make_mut(&mut self.chunks[chunk]).get_mut(at)
    }

    pub fn last_mut(&mut self) -> Option<&mut T> {
        let index = self.len.checked_sub(1)?;
        self.get_mut(index)
    }

    /// The earliest record matching `predicate`, unsharing only its chunk.
    pub fn find_mut(&mut self, predicate: impl FnMut(&T) -> bool) -> Option<&mut T> {
        let index = self.iter().position(predicate)?;
        self.get_mut(index)
    }

    /// The latest record matching `predicate`, unsharing only its chunk.
    pub fn rfind_mut(&mut self, predicate: impl FnMut(&T) -> bool) -> Option<&mut T> {
        let index = self.iter().rposition(predicate)?;
        self.get_mut(index)
    }

    /// Unshares every chunk: for the rare pass that rewrites history.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        let offset = self.offset;
        self.chunks
            .iter_mut()
            .enumerate()
            .flat_map(move |(chunk, records)| {
                let skip = if chunk == 0 { offset } else { 0 };
                Arc::make_mut(records).iter_mut().skip(skip)
            })
    }
}

impl<T: Clone> FromIterator<T> for AppendLog<T> {
    fn from_iter<I: IntoIterator<Item = T>>(records: I) -> Self {
        let mut log = AppendLog::new();
        log.extend(records);
        log
    }
}

impl<T: Clone> Extend<T> for AppendLog<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, records: I) {
        for record in records {
            self.push(record);
        }
    }
}

impl<T: Clone> From<Vec<T>> for AppendLog<T> {
    fn from(records: Vec<T>) -> Self {
        records.into_iter().collect()
    }
}

impl<'a, T> IntoIterator for &'a AppendLog<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A log's records in insertion order, from either end.
#[derive(Debug, Clone)]
pub struct Iter<'a, T> {
    log: &'a AppendLog<T>,
    front: usize,
    back: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.front += 1;
        self.log.get(self.front - 1)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.back - self.front;
        (left, Some(left))
    }
}

impl<T> DoubleEndedIterator for Iter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.log.get(self.back)
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}

#[cfg(test)]
mod tests {
    use super::AppendLog;
    use std::sync::Arc;

    fn log(len: usize) -> AppendLog<usize> {
        (0..len).collect()
    }

    fn shared_chunks(a: &AppendLog<usize>, b: &AppendLog<usize>) -> usize {
        a.chunks
            .iter()
            .zip(&b.chunks)
            .filter(|(x, y)| Arc::ptr_eq(x, y))
            .count()
    }

    #[test]
    fn a_clone_shares_every_chunk() {
        let original = log(150);
        let copy = original.clone();
        assert_eq!(original.chunks.len(), 3);
        assert_eq!(shared_chunks(&original, &copy), 3);
    }

    #[test]
    fn a_push_after_a_clone_unshares_only_the_tail() {
        let original = log(150);
        let mut copy = original.clone();

        copy.push(999);

        assert_eq!(shared_chunks(&original, &copy), 2);
        assert_eq!(original.len(), 150);
        assert_eq!(original.last(), Some(&149));
        assert_eq!(copy.len(), 151);
        assert_eq!(copy.last(), Some(&999));
    }

    #[test]
    fn a_push_onto_a_full_tail_starts_a_chunk_and_unshares_nothing() {
        let original = log(128);
        let mut copy = original.clone();
        copy.push(128);
        assert_eq!(shared_chunks(&original, &copy), 2);
        assert_eq!(copy.chunks.len(), 3);
    }

    #[test]
    fn iteration_matches_a_vec_in_both_directions() {
        let records: Vec<usize> = (0..200).map(|n| n * 3).collect();
        let log: AppendLog<usize> = records.iter().copied().collect();

        assert!(log.iter().eq(records.iter()));
        assert!(log.iter().rev().eq(records.iter().rev()));
        assert_eq!(log.iter().len(), records.len());
        assert_eq!(log.get(130), records.get(130));
        assert_eq!(log.first(), records.first());
        assert_eq!(log.get(200), None);
    }

    #[test]
    fn get_mut_and_last_mut_unshare_exactly_one_chunk() {
        let original = log(150);

        let mut copy = original.clone();
        *copy.get_mut(70).unwrap() = 0;
        assert_eq!(shared_chunks(&original, &copy), 2);
        assert!(!Arc::ptr_eq(&original.chunks[1], &copy.chunks[1]));
        assert_eq!((original.get(70), copy.get(70)), (Some(&70), Some(&0)));

        let mut copy = original.clone();
        *copy.last_mut().unwrap() = 0;
        assert_eq!(shared_chunks(&original, &copy), 2);
        assert_eq!(original.last(), Some(&149));
    }

    #[test]
    fn find_mut_and_rfind_mut_take_opposite_ends_and_unshare_one_chunk() {
        let original: AppendLog<usize> = (0..150).map(|n| n % 100).collect();

        let mut copy = original.clone();
        *copy.find_mut(|&n| n == 20).unwrap() = 999;
        assert_eq!((copy.get(20), copy.get(120)), (Some(&999), Some(&20)));
        assert_eq!(shared_chunks(&original, &copy), 2);

        let mut copy = original.clone();
        *copy.rfind_mut(|&n| n == 20).unwrap() = 999;
        assert_eq!((copy.get(20), copy.get(120)), (Some(&20), Some(&999)));
        assert_eq!(shared_chunks(&original, &copy), 2);

        assert!(copy.find_mut(|&n| n == 500).is_none());
    }

    #[test]
    fn a_front_trim_drops_whole_chunks() {
        let mut log = log(150);

        log.drop_front(70);

        assert_eq!(log.chunks.len(), 2);
        assert_eq!(log.len(), 80);
        assert_eq!(log.first(), Some(&70));
        assert!(log.iter().copied().eq(70..150));
        assert!(log.iter().rev().copied().eq((70..150).rev()));

        log.push(150);
        assert_eq!(log.last(), Some(&150));
        log.drop_front(81);
        assert!(log.is_empty());
        assert!(log.chunks.is_empty());
    }

    #[test]
    fn a_truncate_unshares_only_the_chunk_it_cuts() {
        let original = log(150);
        let mut copy = original.clone();

        copy.truncate(100);

        assert_eq!(shared_chunks(&original, &copy), 1);
        assert_eq!(copy.chunks.len(), 2);
        assert!(copy.iter().copied().eq(0..100));
        assert!(original.iter().copied().eq(0..150));

        copy.push(7);
        assert_eq!((copy.len(), copy.last()), (101, Some(&7)));
        copy.truncate(64);
        assert_eq!(shared_chunks(&original, &copy), 1);
        copy.truncate(0);
        assert!(copy.is_empty());
    }

    #[test]
    fn iter_mut_rewrites_every_live_record() {
        let mut log = log(100);
        log.drop_front(10);
        let original = log.clone();

        for record in log.iter_mut() {
            *record += 1000;
        }

        assert!(log.iter().copied().eq(1010..1100));
        assert!(original.iter().copied().eq(10..100));
    }
}
