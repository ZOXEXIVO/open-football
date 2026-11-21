/// Maximum-total assignment of candidates to slots by bitmask DP: each
/// candidate takes at most one slot, and as many slots are filled as there
/// are candidates to fill them.
pub struct SlotAssignment;

impl SlotAssignment {
    const UNREACHED: u8 = u8::MAX;
    const SKIPPED: u8 = u8::MAX - 1;

    /// `scores[candidate * slots + slot]` is what the candidate is worth in
    /// the slot. A candidate flagged in `must_place` is never left out: the
    /// others are placed around him. Returns, for each slot, the candidate
    /// placed there.
    pub fn solve(scores: &[f32], slots: usize, must_place: &[bool]) -> Vec<Option<usize>> {
        let candidates = must_place.len();
        if slots == 0 || candidates == 0 {
            return vec![None; slots];
        }
        let full_mask = (1usize << slots) - 1;
        let states = full_mask + 1;
        let neg_inf = f32::NEG_INFINITY;
        // One byte per (candidate, mask) records the move that reached it:
        // the slot taken, or the skip. The mask it came from follows from
        // that, and only two rows of scores are ever live.
        let mut dp = vec![neg_inf; states];
        let mut next = vec![neg_inf; states];
        let mut moves = vec![Self::UNREACHED; (candidates + 1) * states];
        dp[0] = 0.0;

        for i in 0..candidates {
            next.fill(neg_inf);
            let row = (i + 1) * states;
            for mask in 0..=full_mask {
                let current = dp[mask];
                if !current.is_finite() {
                    continue;
                }
                if !must_place[i] && current > next[mask] {
                    next[mask] = current;
                    moves[row + mask] = Self::SKIPPED;
                }
                for slot in 0..slots {
                    let bit = 1usize << slot;
                    if mask & bit != 0 {
                        continue;
                    }
                    let candidate = current + scores[i * slots + slot];
                    if candidate > next[mask | bit] {
                        next[mask | bit] = candidate;
                        moves[row + (mask | bit)] = slot as u8;
                    }
                }
            }
            std::mem::swap(&mut dp, &mut next);
        }

        // Every slot when there are candidates enough; otherwise the best
        // filling of as many as there are candidates.
        let filled = slots.min(candidates) as u32;
        let best = (0..=full_mask)
            .filter(|mask| mask.count_ones() == filled && dp[*mask].is_finite())
            .max_by(|a, b| dp[*a].total_cmp(&dp[*b]));
        let mut assigned = vec![None; slots];
        let Some(mut mask) = best else {
            return assigned;
        };
        for i in (1..=candidates).rev() {
            match moves[i * states + mask] {
                Self::UNREACHED => break,
                Self::SKIPPED => {}
                slot => {
                    assigned[slot as usize] = Some(i - 1);
                    mask &= !(1usize << slot);
                }
            }
        }
        assigned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_takes_the_best_total_not_each_slots_best_man() {
        // The first man is the best in both slots; he goes where the second
        // man is weakest.
        let scores = [9.0, 8.0, 7.0, 2.0];
        assert_eq!(
            SlotAssignment::solve(&scores, 2, &[false, false]),
            vec![Some(1), Some(0)]
        );
    }

    #[test]
    fn a_candidate_who_must_be_placed_is_placed() {
        let scores = [9.0, 8.0, 1.0];
        assert_eq!(
            SlotAssignment::solve(&scores, 1, &[false, false, true]),
            vec![Some(2)]
        );
    }

    #[test]
    fn fewer_candidates_than_slots_fill_the_slots_they_are_best_in() {
        let scores = [1.0, 5.0, 2.0];
        assert_eq!(
            SlotAssignment::solve(&scores, 3, &[false]),
            vec![None, Some(0), None]
        );
        assert_eq!(SlotAssignment::solve(&[], 2, &[]), vec![None, None]);
    }
}
