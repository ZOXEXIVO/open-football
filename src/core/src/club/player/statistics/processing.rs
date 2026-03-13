use chrono::NaiveDate;
use crate::club::player::player::Player;
use crate::league::Season;
use crate::TeamInfo;

impl Player {
    /// Record a permanent transfer (called by transfer execution).
    /// Resets stats, saves history for both clubs, sets transfer date.
    pub fn on_transfer(&mut self, from: &TeamInfo, to: &TeamInfo, fee: f64, date: NaiveDate) {
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.statistics_history.record_transfer(stats, from, to, fee, date);
        self.last_transfer_date = Some(date);
    }

    /// Record a loan move (called by loan execution).
    /// Resets stats, saves history for parent + loan club, sets transfer date.
    pub fn on_loan(&mut self, from: &TeamInfo, to: &TeamInfo, loan_fee: f64, date: NaiveDate) {
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.statistics_history.record_loan(stats, from, to, loan_fee, date);
        self.last_transfer_date = Some(date);
    }

    /// Record a loan return (called at end of loan period).
    /// Merges remaining stats into the loan entry, sets transfer date.
    pub fn on_loan_return(&mut self, borrowing: &TeamInfo, date: NaiveDate) {
        let stats = std::mem::take(&mut self.statistics);
        self.statistics_history.record_loan_return(stats, borrowing, date);
        self.last_transfer_date = Some(date);
    }

    /// Record season-end snapshot (called when new season starts).
    /// Saves stats to history and resets for new season.
    pub fn on_season_end(&mut self, season: Season, team: &TeamInfo, date: NaiveDate) {
        let is_loan = self.is_on_loan();
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.statistics_history.record_season_end(
            season, stats, team, is_loan, self.last_transfer_date,
        );
        self.last_transfer_date = None;
    }

    /// Record a cancel-loan from the web UI.
    /// Snapshots borrowing club stats, cleans stale entries, creates parent placeholder.
    pub fn on_cancel_loan(
        &mut self,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.statistics_history.record_cancel_loan(stats, borrowing, parent, is_loan, date);
        self.last_transfer_date = Some(date);
    }

    /// Record a manual transfer from the web UI.
    /// Snapshots source club stats, cleans stale entries, creates destination placeholder.
    pub fn on_manual_transfer(
        &mut self,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: Option<f64>,
        date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.statistics_history.record_departure_transfer(stats, from, to, fee, is_loan, date);
        self.last_transfer_date = Some(date);
    }

    /// Record a manual loan from the web UI.
    /// Snapshots source stats, cleans stale entries, creates parent + destination placeholders.
    pub fn on_manual_loan(
        &mut self,
        from: &TeamInfo,
        parent: &TeamInfo,
        to: &TeamInfo,
        date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.statistics_history.record_departure_loan(stats, from, parent, to, is_loan, date);
        self.last_transfer_date = Some(date);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills,
        PlayerStatistics, PlayerStatisticsHistoryItem,
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

    fn make_team(name: &str, slug: &str) -> TeamInfo {
        TeamInfo {
            name: name.to_string(),
            slug: slug.to_string(),
            reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
        }
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
            seq_id: 0,
        }
    }

    // ---------------------------------------------------------------
    // on_transfer: resets stats and creates history
    // ---------------------------------------------------------------

    #[test]
    fn on_transfer_resets_and_creates_history() {
        let mut player = make_player();
        player.statistics = make_stats(20, 5);

        let from = make_team("Inter", "inter");
        let to = make_team("Juventus", "juventus");

        player.on_transfer(&from, &to, 5_000_000.0, make_date(2032, 1, 15));

        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.statistics.goals, 0);
        assert_eq!(player.last_transfer_date, Some(make_date(2032, 1, 15)));

        assert_eq!(player.statistics_history.items.len(), 2);
        assert_eq!(player.statistics_history.items[0].team_slug, "inter");
        assert_eq!(player.statistics_history.items[0].statistics.played, 20);
        assert_eq!(player.statistics_history.items[1].team_slug, "juventus");
        assert_eq!(player.statistics_history.items[1].transfer_fee, Some(5_000_000.0));
    }

    // ---------------------------------------------------------------
    // on_loan: creates parent + loan entries
    // ---------------------------------------------------------------

    #[test]
    fn on_loan_creates_entries() {
        let mut player = make_player();
        player.statistics = make_stats(10, 2);

        let from = make_team("Juventus", "juventus");
        let to = make_team("Torino", "torino");

        player.on_loan(&from, &to, 50_000.0, make_date(2032, 1, 15));

        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.statistics_history.items.len(), 2);
        assert!(!player.statistics_history.items[0].is_loan);
        assert!(player.statistics_history.items[1].is_loan);
    }

    // ---------------------------------------------------------------
    // on_loan_return: merges stats into loan entry
    // ---------------------------------------------------------------

    #[test]
    fn on_loan_return_merges_stats() {
        let mut player = make_player();
        player.statistics = make_stats(15, 4);

        // Existing loan placeholder
        let mut placeholder = make_history_item(2031, "torino", true, 0);
        placeholder.transfer_fee = Some(50_000.0);
        player.statistics_history.items.push(placeholder);

        let borrowing = make_team("Torino", "torino");
        player.on_loan_return(&borrowing, make_date(2032, 5, 31));

        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.statistics_history.items.len(), 1);
        assert_eq!(player.statistics_history.items[0].statistics.played, 15);
        assert_eq!(player.statistics_history.items[0].transfer_fee, Some(50_000.0));
    }

    // ---------------------------------------------------------------
    // on_season_end: snapshots and resets
    // ---------------------------------------------------------------

    #[test]
    fn on_season_end_snapshots_and_resets() {
        let mut player = make_player();
        player.statistics = make_stats(30, 10);
        player.friendly_statistics = make_stats(3, 1);

        let team = make_team("Inter", "inter");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.friendly_statistics.played, 0);
        assert!(player.last_transfer_date.is_none());

        assert_eq!(player.statistics_history.items.len(), 1);
        let entry = &player.statistics_history.items[0];
        assert_eq!(entry.season.start_year, 2031);
        assert_eq!(entry.statistics.played, 30);
        assert_eq!(entry.statistics.goals, 10);
    }

    #[test]
    fn on_season_end_marks_loan() {
        let mut player = make_player();
        player.statistics = make_stats(10, 2);
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500, make_date(2032, 5, 31), 99, 0, 100,
        ));

        let team = make_team("Torino", "torino");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        assert!(player.statistics_history.items[0].is_loan);
    }

    #[test]
    fn on_season_end_multiple_seasons() {
        let mut player = make_player();

        let team = make_team("Roma", "roma");

        player.statistics = make_stats(30, 10);
        player.on_season_end(Season::new(2030), &team, make_date(2031, 8, 1));

        player.statistics = make_stats(28, 7);
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        assert_eq!(player.statistics_history.items.len(), 2);
        assert_eq!(player.statistics_history.items[0].statistics.played, 30);
        assert_eq!(player.statistics_history.items[1].statistics.played, 28);
        assert_eq!(player.statistics.played, 0);
    }

    #[test]
    fn on_season_end_no_phantom_after_loan_return() {
        let mut player = make_player();
        player.statistics = make_stats(0, 0);

        // Loan entry + pre-loan entry
        player.statistics_history.items.push(make_history_item(2031, "torino", true, 15));
        player.statistics_history.items.push(make_history_item(2031, "juventus", false, 10));

        player.last_transfer_date = Some(make_date(2032, 5, 31));

        let team = make_team("Juventus", "juventus");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        // Should NOT create a 3rd entry
        assert_eq!(player.statistics_history.items.len(), 2);
    }

    #[test]
    fn on_season_end_merges_two_stints() {
        let mut player = make_player();
        player.statistics = make_stats(5, 2);

        player.statistics_history.items.push(make_history_item(2031, "juventus", false, 10));
        player.statistics_history.items.push(make_history_item(2031, "torino", true, 15));

        let team = make_team("Juventus", "juventus");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        let juve = player.statistics_history.items.iter()
            .find(|e| e.team_slug == "juventus").unwrap();
        assert_eq!(juve.statistics.played, 15); // 10 + 5
    }
}
