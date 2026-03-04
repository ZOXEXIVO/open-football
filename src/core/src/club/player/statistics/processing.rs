use chrono::NaiveDate;
use crate::club::player::player::Player;
use crate::league::Season;
use crate::{ContractType, PlayerStatisticsHistoryItem};

impl Player {
    /// Snapshot current season statistics into history and reset for new season
    pub fn snapshot_season_statistics(
        &mut self,
        season: Season,
        team_name: &str,
        team_slug: &str,
        team_reputation: u16,
        league_name: &str,
        league_slug: &str,
        date: NaiveDate,
    ) {
        let is_loan = self.contract.as_ref()
            .map(|c| c.contract_type == ContractType::Loan)
            .unwrap_or(false);

        let old_stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();

        // If the player transferred to this club AFTER the season being snapshotted
        // started, any accumulated stats belong to the transfer's season entry —
        // not the old season. Merge into the existing transfer placeholder instead
        // of creating a phantom entry (e.g. "2032/33 Inter" when transfer was Aug 2033).
        if let Some(transfer_date) = self.last_transfer_date {
            let transfer_season = Season::from_date(transfer_date);
            if transfer_season.start_year > season.start_year {
                // Find the transfer placeholder and merge stats into it
                if let Some(placeholder) = self.statistics_history.items.iter_mut().find(|e| {
                    e.season.start_year == transfer_season.start_year
                        && e.team_slug == team_slug
                }) {
                    placeholder.statistics = old_stats;
                }
                // Clear the flag — merged, won't trigger again next season
                self.last_transfer_date = None;
                return;
            }
        }

        // Normal snapshot — clear transfer flag
        self.last_transfer_date = None;

        self.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
            season,
            team_name: team_name.to_string(),
            team_slug: team_slug.to_string(),
            team_reputation,
            league_name: league_name.to_string(),
            league_slug: league_slug.to_string(),
            is_loan,
            transfer_fee: None,
            statistics: old_stats,
            created_at: date,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills,
        PlayerStatistics, PlayerStatisticsHistory, PlayerStatisticsHistoryItem,
    };

    fn make_date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_player() -> crate::Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(make_date(2000, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn make_stats(played: u16, goals: u16) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.goals = goals;
        s
    }

    fn make_history_item(start_year: u16, slug: &str, is_loan: bool, played: u16) -> PlayerStatisticsHistoryItem {
        let mut stats = PlayerStatistics::default();
        stats.played = played;
        PlayerStatisticsHistoryItem {
            season: Season::new(start_year),
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 100,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan,
            transfer_fee: None,
            statistics: stats,
            created_at: make_date(start_year as i32 + 1, 6, 1),
        }
    }

    // ---------------------------------------------------------------
    // Normal snapshot: creates history entry and resets current stats
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_creates_history_entry_and_resets_stats() {
        let mut player = make_player();
        player.statistics = make_stats(20, 5);
        player.friendly_statistics = make_stats(3, 1);

        let season = Season::new(2031);
        let date = make_date(2032, 8, 1);

        player.snapshot_season_statistics(
            season, "Inter", "inter", 200, "Serie A", "serie-a", date,
        );

        // Current stats should be reset
        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.statistics.goals, 0);
        assert_eq!(player.friendly_statistics.played, 0);

        // History should have one entry with the old stats
        assert_eq!(player.statistics_history.items.len(), 1);
        let entry = &player.statistics_history.items[0];
        assert_eq!(entry.season.start_year, 2031);
        assert_eq!(entry.team_slug, "inter");
        assert_eq!(entry.league_slug, "serie-a");
        assert_eq!(entry.statistics.played, 20);
        assert_eq!(entry.statistics.goals, 5);
        assert!(!entry.is_loan);
        assert!(entry.transfer_fee.is_none());
    }

    // ---------------------------------------------------------------
    // Loan contract: is_loan flag set correctly
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_marks_loan_when_contract_is_loan() {
        let mut player = make_player();
        player.statistics = make_stats(10, 2);

        let loan_contract = crate::PlayerClubContract::new_loan(
            500, make_date(2032, 5, 31), 99,
        );
        player.contract = Some(loan_contract);

        player.snapshot_season_statistics(
            Season::new(2031), "Torino", "torino", 100, "Serie A", "serie-a",
            make_date(2032, 8, 1),
        );

        assert_eq!(player.statistics_history.items.len(), 1);
        assert!(player.statistics_history.items[0].is_loan);
    }

    // ---------------------------------------------------------------
    // No contract: is_loan defaults to false
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_not_loan_when_no_contract() {
        let mut player = make_player();
        player.contract = None;
        player.statistics = make_stats(5, 0);

        player.snapshot_season_statistics(
            Season::new(2031), "Bologna", "bologna", 80, "Serie A", "serie-a",
            make_date(2032, 8, 1),
        );

        assert!(!player.statistics_history.items[0].is_loan);
    }

    // ---------------------------------------------------------------
    // Transfer date in SAME season: normal snapshot, flag cleared
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_with_transfer_in_same_season_creates_normal_entry() {
        let mut player = make_player();
        player.statistics = make_stats(15, 3);

        // Transferred in Jan 2032 — same season 2031/32
        player.last_transfer_date = Some(make_date(2032, 1, 15));

        player.snapshot_season_statistics(
            Season::new(2031), "Juventus", "juventus", 250, "Serie A", "serie-a",
            make_date(2032, 8, 1),
        );

        // Should create normal entry (transfer_season 2031 == season 2031)
        assert_eq!(player.statistics_history.items.len(), 1);
        assert_eq!(player.statistics_history.items[0].statistics.played, 15);
        assert!(player.last_transfer_date.is_none()); // cleared
    }

    // ---------------------------------------------------------------
    // Transfer date in NEXT season: merges stats into placeholder
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_with_transfer_in_next_season_merges_into_placeholder() {
        let mut player = make_player();
        player.statistics = make_stats(8, 2);

        // Transferred Aug 2033 — season 2033/34
        player.last_transfer_date = Some(make_date(2033, 8, 15));

        // Pre-existing placeholder from the transfer system
        let placeholder = make_history_item(2033, "inter", false, 0);
        player.statistics_history.items.push(placeholder);

        // Snapshot for season 2032/33 (the OLD season)
        player.snapshot_season_statistics(
            Season::new(2032), "Inter", "inter", 200, "Serie A", "serie-a",
            make_date(2033, 8, 1),
        );

        // Should NOT create a new entry for 2032/33 — stats merged into 2033/34 placeholder
        assert_eq!(player.statistics_history.items.len(), 1);
        assert_eq!(player.statistics_history.items[0].season.start_year, 2033);
        assert_eq!(player.statistics_history.items[0].statistics.played, 8);
        assert_eq!(player.statistics_history.items[0].statistics.goals, 2);
        assert!(player.last_transfer_date.is_none()); // cleared
    }

    // ---------------------------------------------------------------
    // Transfer in next season but no matching placeholder: stats lost
    // (edge case — placeholder should always exist if transfer happened)
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_with_transfer_next_season_no_placeholder_loses_stats() {
        let mut player = make_player();
        player.statistics = make_stats(5, 1);

        // Transferred Aug 2033, but placeholder slug doesn't match
        player.last_transfer_date = Some(make_date(2033, 8, 15));

        let placeholder = make_history_item(2033, "other-team", false, 0);
        player.statistics_history.items.push(placeholder);

        player.snapshot_season_statistics(
            Season::new(2032), "Inter", "inter", 200, "Serie A", "serie-a",
            make_date(2033, 8, 1),
        );

        // No match found — stats not merged, placeholder unchanged
        assert_eq!(player.statistics_history.items.len(), 1);
        assert_eq!(player.statistics_history.items[0].statistics.played, 0);
        assert!(player.last_transfer_date.is_none()); // still cleared
    }

    // ---------------------------------------------------------------
    // Multiple seasons: snapshot accumulates history entries
    // ---------------------------------------------------------------

    #[test]
    fn snapshot_multiple_seasons_accumulates() {
        let mut player = make_player();

        // Season 1
        player.statistics = make_stats(30, 10);
        player.snapshot_season_statistics(
            Season::new(2030), "Roma", "roma", 150, "Serie A", "serie-a",
            make_date(2031, 8, 1),
        );

        // Season 2
        player.statistics = make_stats(28, 7);
        player.snapshot_season_statistics(
            Season::new(2031), "Roma", "roma", 150, "Serie A", "serie-a",
            make_date(2032, 8, 1),
        );

        assert_eq!(player.statistics_history.items.len(), 2);
        assert_eq!(player.statistics_history.items[0].season.start_year, 2030);
        assert_eq!(player.statistics_history.items[0].statistics.played, 30);
        assert_eq!(player.statistics_history.items[1].season.start_year, 2031);
        assert_eq!(player.statistics_history.items[1].statistics.played, 28);
        assert_eq!(player.statistics.played, 0); // reset
    }

    // ---------------------------------------------------------------
    // push_or_replace: replaces 0-game placeholder with actual stats
    // ---------------------------------------------------------------

    #[test]
    fn push_or_replace_replaces_zero_game_placeholder() {
        let mut history = PlayerStatisticsHistory::new();

        // Transfer placeholder with 0 games and fee
        let mut placeholder = make_history_item(2031, "inter", false, 0);
        placeholder.transfer_fee = Some(5_000_000.0);
        placeholder.is_loan = true;
        history.push_or_replace(placeholder);

        // Season snapshot with actual stats
        let entry = make_history_item(2031, "inter", false, 25);
        history.push_or_replace(entry);

        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].statistics.played, 25);
        // Preserves original transfer_fee and is_loan
        assert_eq!(history.items[0].transfer_fee, Some(5_000_000.0));
        assert!(history.items[0].is_loan);
    }

    // ---------------------------------------------------------------
    // push_or_replace: does NOT replace when both have 0 games
    // ---------------------------------------------------------------

    #[test]
    fn push_or_replace_no_duplicate_zero_game_entries() {
        let mut history = PlayerStatisticsHistory::new();

        let item1 = make_history_item(2031, "inter", false, 0);
        history.push_or_replace(item1);

        let item2 = make_history_item(2031, "inter", false, 0);
        history.push_or_replace(item2);

        // Should still be just 1 entry (dedup)
        assert_eq!(history.items.len(), 1);
    }

    // ---------------------------------------------------------------
    // push_or_replace: different slugs are separate entries
    // ---------------------------------------------------------------

    #[test]
    fn push_or_replace_different_teams_separate() {
        let mut history = PlayerStatisticsHistory::new();

        history.push_or_replace(make_history_item(2031, "inter", false, 10));
        history.push_or_replace(make_history_item(2031, "juventus", false, 15));

        assert_eq!(history.items.len(), 2);
    }

    // ---------------------------------------------------------------
    // push_or_replace: different seasons are separate entries
    // ---------------------------------------------------------------

    #[test]
    fn push_or_replace_different_seasons_separate() {
        let mut history = PlayerStatisticsHistory::new();

        history.push_or_replace(make_history_item(2030, "inter", false, 10));
        history.push_or_replace(make_history_item(2031, "inter", false, 15));

        assert_eq!(history.items.len(), 2);
    }

    // ---------------------------------------------------------------
    // push_or_replace: new entry with games + existing with games = both kept
    // ---------------------------------------------------------------

    #[test]
    fn push_or_replace_both_with_games_keeps_both() {
        let mut history = PlayerStatisticsHistory::new();

        history.push_or_replace(make_history_item(2031, "inter", false, 10));
        history.push_or_replace(make_history_item(2031, "inter", false, 5));

        // Both have games, first has games so no zero-game match → second pushed
        assert_eq!(history.items.len(), 2);
    }
}
