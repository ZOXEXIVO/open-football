//! The in-match substitution ledger: one [`SubstitutionRecord`] per swap
//! the engine actually made, plus the budget checks the substitution pass
//! asks before it makes another.
//!
//! Distinct from [`SubstitutionInfo`](super::super::result::SubstitutionInfo),
//! which is the same event as it leaves the match on the result — this
//! one is live state, keyed to the match clock.

use super::match_context::MatchContext;

pub struct SubstitutionRecord {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time: u64,
    /// Reason the swap fired. Stamped at the call-site so post-match
    /// emit logic can distinguish protective swaps (injury / youth)
    /// from discretionary tactical hooks.
    pub reason: crate::r#match::engine::flow::result::SubstitutionReason,
    /// How long the match actually stopped for while the change was played
    /// out, in ms — see
    /// [`SubstitutionBreak`](super::super::touchline::SubstitutionBreak), whose
    /// window
    /// closes when the last man reaches his slot rather than on a clock.
    ///
    /// Zero until that window closes, and zero forever on the instant path
    /// (`OF_SUB_WALK_OFF`) and anywhere a swap is made outside a live match.
    /// The replay reads it to hold its substitution shot for exactly as long
    /// as the change lasted instead of guessing at a constant.
    pub break_ms: u64,
}

/// How many separate stoppages each side has spent on changes.
///
/// The Law gives a side five substitutions but only **three opportunities**
/// to make them, the interval not counting as one. It is the reason real
/// changes come in clusters — a double on the hour, a single at 72', two
/// more at 80' — rather than as five separately-scheduled events, and
/// without it a five-sub side simply interrupts the match five times.
///
/// A whole pass rides one window: the loop opens it on its first swap and
/// every further swap in the same pass is free, which is exactly what a
/// double change is.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubstitutionWindows {
    home: u8,
    away: u8,
}

impl SubstitutionWindows {
    /// Stoppages a side may interrupt for a change over normal time.
    /// Half-time is free and does not spend one.
    pub const PER_TEAM: u8 = 3;

    /// Windows this side has already spent.
    pub fn spent(&self, is_home: bool) -> u8 {
        if is_home { self.home } else { self.away }
    }

    /// Charge a side for interrupting play. Saturates rather than wrapping —
    /// a forced injury change is allowed to exceed the allowance (it does in
    /// the Law too), and the counter is only ever read as a comparison.
    pub fn open(&mut self, is_home: bool) {
        let slot = if is_home {
            &mut self.home
        } else {
            &mut self.away
        };
        *slot = slot.saturating_add(1);
    }
}

impl MatchContext {
    pub fn subs_used_by_team(&self, team_id: u32) -> usize {
        self.substitutions
            .iter()
            .filter(|s| s.team_id == team_id)
            .count()
    }

    pub fn can_substitute(&self, team_id: u32) -> bool {
        self.subs_used_by_team(team_id) < self.max_substitutions_per_team
    }

    pub fn record_substitution(
        &mut self,
        team_id: u32,
        player_out_id: u32,
        player_in_id: u32,
        match_time: u64,
        reason: crate::r#match::engine::flow::result::SubstitutionReason,
    ) {
        self.substitutions.push(SubstitutionRecord {
            team_id,
            player_out_id,
            player_in_id,
            match_time,
            reason,
            break_ms: 0,
        });
    }
}
