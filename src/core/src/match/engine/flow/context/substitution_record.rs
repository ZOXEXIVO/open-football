//! The in-match substitution ledger: one [`SubstitutionRecord`] per swap
//! the engine actually made, plus the budget checks the substitution pass
//! asks before it makes another.
//!
//! Distinct from [`SubstitutionInfo`](super::super::result::SubstitutionInfo),
//! which is the same event as it leaves the match on the result — this
//! one is live state, keyed to the match clock.

use super::match_context::MatchContext;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::flow::result::SubstitutionReason;

pub struct SubstitutionRecord {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time: u64,
    /// Reason the swap fired. Stamped at the call-site so post-match
    /// emit logic can distinguish protective swaps (injury / youth)
    /// from discretionary tactical hooks.
    pub reason: SubstitutionReason,
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
    interval: bool,
    /// Extra stoppages granted for extra time.
    extra: u8,
}

impl SubstitutionWindows {
    /// Stoppages a side may interrupt for a change over normal time.
    /// Half-time is free and does not spend one.
    pub const PER_TEAM: u8 = 3;

    /// Stoppages a side may interrupt for a change in this match so far.
    pub fn allowance(&self) -> u8 {
        Self::PER_TEAM + self.extra
    }

    /// Extra time brings one more stoppage to make a change in.
    pub fn grant_extra_time(&mut self) {
        self.extra += 1;
    }

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

    /// The half-time whistle: the free opportunity is open.
    ///
    /// It is taken at the first stoppage of the second half rather than at
    /// the break itself, because the break has no play to show a change in —
    /// the period boundary re-forms both sides ten milliseconds later, so an
    /// interval change was never walked, never recorded, and its marker on
    /// the replay led nowhere. On that stoppage it is walked on and clipped
    /// like every other change, and it is still the interval's: free, and
    /// priced as one.
    pub fn open_interval(&mut self) {
        self.interval = true;
    }

    /// Whether the interval is still waiting to be taken.
    pub fn at_interval(&self) -> bool {
        self.interval
    }

    /// Taken by the first substitution pass after the restart, whether or
    /// not either side changed on it — the manager has had his look.
    pub fn close_interval(&mut self) {
        self.interval = false;
    }
}

impl MatchContext {
    /// He has left the pitch for good: his stat line and his physical state
    /// are taken now, at the minute he went, and stand over whatever the
    /// final whistle would have recorded for him.
    pub fn record_departure(&mut self, player: &MatchPlayer) {
        let minutes = player.minutes_played_at(self.total_match_time);
        self.substituted_out_stats
            .push((player.id, player.to_match_end_stats(minutes)));
        self.substituted_out_physical_snapshots
            .push(player.to_physical_snapshot(self.total_match_time));
    }

    /// Extra time brings one more change and one more stoppage to make it
    /// in, where the competition's rules allow it.
    pub fn grant_extra_time_allowance(&mut self) {
        if !self.allow_extra_time_extra_sub {
            return;
        }
        if self.max_substitutions_per_team < usize::MAX {
            self.max_substitutions_per_team += 1;
        }
        self.substitution_windows.grant_extra_time();
    }

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
        reason: SubstitutionReason,
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
