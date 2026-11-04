//! The free-agent pool, as the market sees it.
//! World-wide aggregate of one tick's free-agent market activity.
//!
//! The pool lives on `SimulatorData`, but the decisions that touch it are
//! made inside every country's parallel pass. Each country reports what it
//! did — who it made an offer to, whose acceptance roll failed, why it
//! passed on the rest — and the orchestrator folds every country's report
//! into one of these before the serial drain.

use crate::club::player::transfer::FreeAgentBlockReason;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// Every country's free-agent bumps for one tick, borrowed in drain order
/// before the serial Phase-C drain.
///
/// Each per-country `apply_deferred_transfer_ops` used to walk the ENTIRE
/// `data.free_agents` pool twice (offer/reject bump + block-reason stamp),
/// once for every country — `O(countries × pool)`. The orchestrator now
/// gathers every country's bump ids into one of these and runs a
/// SINGLE pass over the pool via
/// [`ApproachPass::apply_free_agent_market_bumps_batch`](crate::transfers::pipeline::ApproachPass::apply_free_agent_market_bumps_batch),
/// matching the documented "one bump per player per tick" intent across the
/// whole world (the previous per-country dedup still allowed a player
/// pursued by two countries to be bumped twice).
#[derive(Default)]
pub struct FreeAgentBumpBatch<'a> {
    /// Pool players that fielded an offer this tick, one slice per country.
    pub offered_ids: Vec<&'a [u32]>,
    /// Pool players whose acceptance roll failed this tick, one slice per
    /// country.
    pub rejected_ids: Vec<&'a [u32]>,
    /// Per-player skip reasons from every country's matcher, one slice per
    /// country; merged to the highest-ranked reason per player before
    /// stamping.
    pub block_reasons: Vec<&'a [(u32, FreeAgentBlockReason)]>,
}

impl FreeAgentBumpBatch<'_> {
    /// True when no country recorded any free-agent market activity this
    /// tick — lets the orchestrator skip the pool pass entirely.
    pub fn is_empty(&self) -> bool {
        self.offered_ids.iter().all(|ids| ids.is_empty())
            && self.rejected_ids.iter().all(|ids| ids.is_empty())
            && self.block_reasons.iter().all(|reasons| reasons.is_empty())
    }

    pub fn offered(&self) -> FxHashSet<u32> {
        self.offered_ids
            .iter()
            .flat_map(|ids| ids.iter().copied())
            .collect()
    }

    pub fn rejected(&self) -> FxHashSet<u32> {
        self.rejected_ids
            .iter()
            .flat_map(|ids| ids.iter().copied())
            .collect()
    }

    /// The highest-ranked reason per player across every country; on a tie
    /// the first country in drain order keeps it.
    ///
    /// A player's reasons only meet each other, so the merge splits by id
    /// and each shard reads the countries in order — the serial fold's
    /// answer, without its single core.
    pub fn merged_block_reasons(&self) -> MergedBlockReasons {
        let shards = (0..MergedBlockReasons::SHARDS)
            .into_par_iter()
            .map(|shard| {
                let mut merged: FxHashMap<u32, FreeAgentBlockReason> = FxHashMap::default();
                for &(player_id, reason) in self.block_reasons.iter().flat_map(|r| r.iter()) {
                    if player_id % MergedBlockReasons::SHARDS != shard {
                        continue;
                    }
                    merged
                        .entry(player_id)
                        .and_modify(|existing| {
                            if reason.rank() > existing.rank() {
                                *existing = reason;
                            }
                        })
                        .or_insert(reason);
                }
                merged
            })
            .collect();
        MergedBlockReasons { shards }
    }
}

/// One block reason per pool player, split by id across a fixed number of
/// shards.
pub struct MergedBlockReasons {
    shards: Vec<FxHashMap<u32, FreeAgentBlockReason>>,
}

impl MergedBlockReasons {
    /// Every shard reads every reason once, so more shards buy little wall
    /// time for a lot of reading.
    const SHARDS: u32 = 8;

    pub fn get(&self, player_id: u32) -> Option<FreeAgentBlockReason> {
        self.shards[(player_id % Self::SHARDS) as usize]
            .get(&player_id)
            .copied()
    }
}
