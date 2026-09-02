//! Per-player transfer **funnel trace** — the diagnostic every springboard
//! change is judged with.
//!
//! A move that never happens leaves no evidence anywhere: no request, no
//! shortlist row, no negotiation, no rejection. Asking "why was this player
//! never bought?" of the shipped model meant re-deriving the whole funnel by
//! hand from file:line. This turns that into one line per (buyer, stage) per
//! pass, printed to **stderr** so it survives being read next to a report on
//! stdout.
//!
//! Armed by `OF_TRACE_PLAYER=<player_id>` (read once, like every other `OF_*`
//! knob). Disarmed it costs one cached `OnceLock` read per call site and
//! nothing else: every emitter is written as
//! `if TransferTrace::is(id) { TransferTrace::line(...) }`, so no string is
//! ever formatted for a player nobody is tracing.
//!
//! Stages, in funnel order:
//!   * `pool`      — the player is in the world snapshot a buyer scans
//!   * `discovery` — breakout / standing scores against the admission bar
//!   * `buyer`     — one buying club's gates: reach, dedup, plausibility,
//!                   listed-target verdict, whether a file was opened
//!   * `need`      — the buyer's own squad evaluation (investment appetite)
//!   * `plaus`     — the staged move-plausibility verdict at negotiation time
//!   * `approach`  — the seller's engagement roll and its inputs
//!   * `fee`       — the club-fee round: reservation, ratio, windfall
//!   * `seller`    — seller-side economics (asset class, importance, income)
//!
//! Per memory `feedback_keep_match_debug_data`, this is kept after the
//! campaign that motivated it, not deleted.

use std::env;
use std::sync::OnceLock;

/// Funnel tracer for one player id. Zero-cost when disarmed.
pub struct TransferTrace;

impl TransferTrace {
    /// The traced player id, or `None` when `OF_TRACE_PLAYER` is unset or
    /// unparseable. Read once for the lifetime of the process.
    pub fn target() -> Option<u32> {
        static TARGET: OnceLock<Option<u32>> = OnceLock::new();
        *TARGET.get_or_init(|| {
            env::var("OF_TRACE_PLAYER")
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok())
        })
    }

    /// True when `player_id` is the traced player. Guard every emitter with
    /// this so the argument formatting never runs on an untraced pass.
    #[inline]
    pub fn is(player_id: u32) -> bool {
        Self::target() == Some(player_id)
    }

    /// Emit one funnel line. `stage` is one of the stage tags in the module
    /// docs; `detail` is free-form `key=value` text so the whole trace can be
    /// grepped per stage and pasted as a table.
    pub fn line(player_id: u32, stage: &str, detail: impl AsRef<str>) {
        eprintln!("[of-trace {player_id}] {stage:<9} | {}", detail.as_ref());
    }
}
