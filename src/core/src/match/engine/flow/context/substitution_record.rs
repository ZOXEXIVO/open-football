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
        });
    }
}
