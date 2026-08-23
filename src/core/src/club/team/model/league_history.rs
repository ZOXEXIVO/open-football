//! Which division a side actually played in, season by season.
//!
//! `Team::league_id` answers "where does this side play *now*", and for a
//! long time that was the only answer anyone could get. Every surface that
//! wanted the division for a PAST season — the player history table above
//! all — re-derived it from the current one, which is right only for a club
//! that has never moved. Promote or relegate a club once and every earlier
//! season on every one of its players' pages silently re-labels itself to
//! the new division.
//!
//! So the move is recorded instead of inferred. Entries are written by the
//! season-end snapshot, which is also what stamps the player rows, so the
//! two agree on the season year by construction rather than by two
//! independent calendar derivations.
//!
//! Consecutive seasons in the same division collapse into the earliest of
//! them: a club that stays put carries exactly one entry for its whole
//! existence, and one that yo-yos carries one per move. The whole
//! `SimulatorData` graph is deep-cloned on every holiday publish, so this
//! staying O(moves) rather than O(seasons) is the point.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamLeagueSpell {
    /// First season the side played in `league_id`, as a season START
    /// year — the same key the player history rows are filed under.
    pub season_start_year: u16,
    pub league_id: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TeamLeagueHistory {
    /// Oldest first, one entry per division change.
    spells: Vec<TeamLeagueSpell>,
    /// Most recent season this record actually closed. Needed because the
    /// spells collapse: a side that has sat in one division since 2026
    /// carries a single `(2026, …)` entry whether the record ends in 2027
    /// or 2040, so the entry alone cannot say how far the record reaches.
    /// Beyond it the record is silent and callers fall back to
    /// `Team::league_id` — which is exactly right for the campaign now
    /// being played, and for a side that has already been promoted out of
    /// the last season the record closed.
    last_recorded_season: Option<u16>,
}

impl TeamLeagueHistory {
    /// Note that `season_start_year` was played in `league_id`. Idempotent:
    /// re-running a season's snapshot overwrites that season's entry rather
    /// than appending a second one, and a season that matches what the side
    /// was already in adds nothing.
    pub fn record(&mut self, season_start_year: u16, league_id: u32) {
        self.last_recorded_season = Some(
            self.last_recorded_season
                .map_or(season_start_year, |last| last.max(season_start_year)),
        );
        if let Some(existing) = self
            .spells
            .iter_mut()
            .find(|s| s.season_start_year == season_start_year)
        {
            existing.league_id = league_id;
            self.collapse();
            return;
        }
        // Already covered: the most recent entry at or before this season
        // names the same division, so the side simply stayed put.
        if self
            .spell_covering(season_start_year)
            .is_some_and(|s| s.league_id == league_id)
        {
            return;
        }
        self.spells.push(TeamLeagueSpell {
            season_start_year,
            league_id,
        });
        self.spells.sort_by_key(|s| s.season_start_year);
        self.collapse();
    }

    /// The division the side played in during `season_start_year`, or
    /// `None` when that season falls outside what the record covers —
    /// before it begins, or after the last season it closed. Callers fall
    /// back to `Team::league_id` there, which is the right answer for the
    /// campaign now being played and the best available one for a season
    /// older than the record.
    pub fn league_for_season(&self, season_start_year: u16) -> Option<u32> {
        if self.last_recorded_season? < season_start_year {
            return None;
        }
        self.spell_covering(season_start_year).map(|s| s.league_id)
    }

    /// The entry that speaks for `season_start_year`: the latest one that
    /// starts at or before it. Entries are kept sorted ascending, so the
    /// first hit walking backwards is it.
    fn spell_covering(&self, season_start_year: u16) -> Option<&TeamLeagueSpell> {
        self.spells
            .iter()
            .rev()
            .find(|s| s.season_start_year <= season_start_year)
    }

    pub fn spells(&self) -> &[TeamLeagueSpell] {
        &self.spells
    }

    /// The most recent season this record closed. Past it the record has
    /// nothing to say.
    pub fn last_recorded_season(&self) -> Option<u16> {
        self.last_recorded_season
    }

    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }

    /// Drop entries a neighbour already implies. An overwrite can leave two
    /// adjacent entries naming the same division; the earlier one is the
    /// one that carries the meaning.
    fn collapse(&mut self) {
        self.spells.dedup_by(|later, earlier| {
            let _ = later;
            earlier.league_id == later.league_id
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_side_that_never_moves_keeps_one_entry() {
        let mut history = TeamLeagueHistory::default();
        for year in 2026..2032 {
            history.record(year, 10);
        }
        assert_eq!(history.spells().len(), 1);
        assert_eq!(history.league_for_season(2031), Some(10));
    }

    #[test]
    fn each_move_is_recorded_and_resolves_per_season() {
        let mut history = TeamLeagueHistory::default();
        history.record(2026, 10); // top flight
        history.record(2027, 10);
        history.record(2028, 20); // relegated
        history.record(2029, 20);
        history.record(2030, 10); // promoted straight back

        assert_eq!(history.spells().len(), 3);
        assert_eq!(history.league_for_season(2026), Some(10));
        assert_eq!(history.league_for_season(2027), Some(10));
        assert_eq!(history.league_for_season(2028), Some(20));
        assert_eq!(history.league_for_season(2029), Some(20));
        assert_eq!(history.league_for_season(2030), Some(10));
        // Beyond the last season it closed, the record is silent — the
        // side may already have moved again for the campaign in progress.
        assert_eq!(history.league_for_season(2031), None);
        assert_eq!(history.last_recorded_season(), Some(2030));
    }

    #[test]
    fn seasons_before_the_record_are_unknown_not_guessed() {
        let mut history = TeamLeagueHistory::default();
        history.record(2028, 20);
        assert_eq!(history.league_for_season(2027), None);
        assert_eq!(TeamLeagueHistory::default().league_for_season(2027), None);
    }

    #[test]
    fn re_recording_a_season_overwrites_rather_than_forks() {
        let mut history = TeamLeagueHistory::default();
        history.record(2026, 10);
        history.record(2027, 20);
        // The 2027 snapshot runs again — same season, corrected division.
        history.record(2027, 30);
        assert_eq!(history.league_for_season(2027), Some(30));
        assert_eq!(history.spells().len(), 2);
    }

    #[test]
    fn an_overwrite_that_matches_its_neighbour_collapses() {
        let mut history = TeamLeagueHistory::default();
        history.record(2026, 10);
        history.record(2027, 20);
        history.record(2027, 10);
        assert_eq!(history.spells().len(), 1);
        assert_eq!(history.league_for_season(2027), Some(10));
    }
}
