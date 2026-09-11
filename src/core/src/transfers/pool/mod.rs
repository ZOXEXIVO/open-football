//! The free-agent pool, as the market sees it.
//! World-wide aggregate of one tick's free-agent market activity.
//!
//! The pool lives on `SimulatorData`, but the decisions that touch it are
//! made inside every country's parallel pass. Each country reports what it
//! did — who it made an offer to, whose acceptance roll failed, why it
//! passed on the rest — and the orchestrator folds every country's report
//! into one of these before the serial drain.

use crate::club::player::transfer::FreeAgentBlockReason;

/// Every country's free-agent bumps for one tick, collected before the
/// serial Phase-C drain.
///
/// Each per-country `apply_deferred_transfer_ops` used to walk the ENTIRE
/// `data.free_agents` pool twice (offer/reject bump + block-reason stamp),
/// once for every country — `O(countries × pool)`. The orchestrator now
/// concatenates every country's bump ids into one of these and runs a
/// SINGLE pass over the pool via
/// [`ApproachPass::apply_free_agent_market_bumps_batch`](crate::transfers::pipeline::ApproachPass::apply_free_agent_market_bumps_batch),
/// matching the documented "one bump per player per tick" intent across the
/// whole world (the previous per-country dedup still allowed a player
/// pursued by two countries to be bumped twice).
#[derive(Default)]
pub struct FreeAgentBumpBatch {
    /// Pool players that fielded an offer this tick (any country).
    pub offered_ids: Vec<u32>,
    /// Pool players whose acceptance roll failed this tick (any country).
    pub rejected_ids: Vec<u32>,
    /// Per-player skip reasons from every country's matcher; merged to the
    /// highest-ranked reason per player before stamping.
    pub block_reasons: Vec<(u32, FreeAgentBlockReason)>,
}

impl FreeAgentBumpBatch {
    /// True when no country recorded any free-agent market activity this
    /// tick — lets the orchestrator skip the pool pass entirely.
    pub fn is_empty(&self) -> bool {
        self.offered_ids.is_empty() && self.rejected_ids.is_empty() && self.block_reasons.is_empty()
    }
}
