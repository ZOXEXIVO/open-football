//! The record of a swap, and why it fired.
//!
//! [`SubstitutionInfo`] is the persisted/wire shape carried on
//! [`MatchResultRaw`](super::raw::MatchResultRaw); [`SubstitutionReason`]
//! is the tag the post-match morale pass reads to tell a protective swap
//! (injury, youth guard, keeper emergency) from a discretionary hook.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstitutionInfo {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time_ms: u64,
    /// Why the substitution fired. Defaulted to `Discretionary` for
    /// backward compatibility with callers that don't yet tag the
    /// pass that produced the sub. Drives the morale-side
    /// `SubstitutionFrustration` gate: critical injury / youth
    /// protection / fatigue rotation never qualify as frustration.
    #[serde(default)]
    pub reason: SubstitutionReason,
    /// How long the match stopped for while the change was played out, in ms.
    ///
    /// The replay reads it to hold its substitution shot for exactly as long
    /// as the change lasted — the window closes when the last man reaches his
    /// slot, so it is nine or ten seconds for an ordinary change and nearly
    /// twice that when somebody has the width of the pitch to cross. See
    /// [`SubstitutionBreak`](super::super::touchline::SubstitutionBreak).
    ///
    /// Defaulted for documents written before the change was played out at
    /// all, and zero on the instant path (`OF_SUB_WALK_OFF`); a reader that
    /// sees zero should fall back to a constant of its own rather than cut
    /// the shot immediately.
    #[serde(default)]
    pub break_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubstitutionReason {
    /// Forced injury swap (in-match condition collapse / rolled injury).
    /// Never a frustration trigger.
    CriticalInjury,
    /// Under-17 condition / jadedness guard. Protective, not punitive.
    YouthProtection,
    /// Discretionary scored-pair swap (tactical / fatigue / development).
    /// The frustration-event detector inspects rating + minute to decide
    /// whether the player was hooked while playing well or pulled early
    /// in a big match — vs a routine 75th-minute fatigue swap that the
    /// player shrugs off.
    #[default]
    Discretionary,
    /// The side lost its goalkeeper (red card, typically) and burned a
    /// substitution to bring the bench keeper on for an outfielder.
    /// The sacrificed player is a victim of circumstance, not coach
    /// doubt — never a frustration trigger.
    GoalkeeperEmergency,
}
