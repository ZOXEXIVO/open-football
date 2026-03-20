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

        // Only destination added — source stats saved if entry exists (none here for fresh player)
        let juve = player.statistics_history.current.iter().find(|e| e.team_slug == "juventus");
        assert!(juve.is_some());
        assert_eq!(juve.unwrap().transfer_fee, Some(5_000_000.0));
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
        // Only loan destination added
        let torino = player.statistics_history.current.iter().find(|e| e.team_slug == "torino");
        assert!(torino.is_some());
        assert!(torino.unwrap().is_loan);
    }

    // ---------------------------------------------------------------
    // on_loan_return: merges stats into loan entry
    // ---------------------------------------------------------------

    #[test]
    fn on_loan_return_updates_stats() {
        let mut player = make_player();
        player.statistics = make_stats(15, 4);

        // Existing loan placeholder in current season
        use crate::club::player::statistics::history::CurrentSeasonEntry;
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Torino".to_string(),
            team_slug: "torino".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: true,
            transfer_fee: Some(50_000.0),
            statistics: PlayerStatistics::default(),
            departed_date: None, joined_date: make_date(2032, 1, 15),
            seq_id: 0,
        });

        let borrowing = make_team("Torino", "torino");
        player.on_loan_return(&borrowing, make_date(2032, 5, 31));

        assert_eq!(player.statistics.played, 0);
        // Upsert updates existing Torino loan entry with 15 games
        let torino = player.statistics_history.current.iter()
            .find(|e| e.team_slug == "torino" && e.is_loan)
            .unwrap();
        assert_eq!(torino.statistics.played, 15);
        assert_eq!(torino.transfer_fee, Some(50_000.0));
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

        // Simulate: loan entry + pre-loan entry already in current
        use crate::club::player::statistics::history::CurrentSeasonEntry;
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Torino".to_string(), team_slug: "torino".to_string(),
            team_reputation: 100, league_name: "Serie A".to_string(), league_slug: "serie-a".to_string(),
            is_loan: true, transfer_fee: None, statistics: make_stats(15, 0),
            departed_date: None, joined_date: make_date(2032, 1, 1), seq_id: 0,
        });
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Juventus".to_string(), team_slug: "juventus".to_string(),
            team_reputation: 100, league_name: "Serie A".to_string(), league_slug: "serie-a".to_string(),
            is_loan: false, transfer_fee: None, statistics: make_stats(10, 0),
            departed_date: None, joined_date: make_date(2031, 8, 1), seq_id: 1,
        });

        let team = make_team("Juventus", "juventus");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        // Both entries had games, both should be finalized
        assert_eq!(player.statistics_history.items.len(), 2);
        // current has 1 entry: seeded empty entry for new season
        assert_eq!(player.statistics_history.current.len(), 1);
        assert_eq!(player.statistics_history.current[0].team_slug, "juventus");
        assert_eq!(player.statistics_history.current[0].statistics.total_games(), 0);
    }

    #[test]
    fn on_season_end_merges_live_stats_into_current_team() {
        let mut player = make_player();
        player.statistics = make_stats(5, 2);

        // Two stints in current season
        use crate::club::player::statistics::history::CurrentSeasonEntry;
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Juventus".to_string(), team_slug: "juventus".to_string(),
            team_reputation: 100, league_name: "Serie A".to_string(), league_slug: "serie-a".to_string(),
            is_loan: false, transfer_fee: None, statistics: make_stats(10, 0),
            departed_date: None, joined_date: make_date(2031, 8, 1), seq_id: 0,
        });
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Torino".to_string(), team_slug: "torino".to_string(),
            team_reputation: 100, league_name: "Serie A".to_string(), league_slug: "serie-a".to_string(),
            is_loan: true, transfer_fee: None, statistics: make_stats(15, 0),
            departed_date: None, joined_date: make_date(2032, 1, 1), seq_id: 1,
        });

        let team = make_team("Juventus", "juventus");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        // Season end merges current_stats (5 games) into the Juventus current entry
        let juve = player.statistics_history.items.iter()
            .find(|e| e.team_slug == "juventus").unwrap();
        assert_eq!(juve.statistics.played, 15); // 10 + 5
    }

    // ===================================================================
    // Multi-season lifecycle: transfer near season end, then loan
    // ===================================================================
    //
    // Scenario:
    //   Season 2025/26 — player at Roma, plays full season
    //   Late May 2026 — transferred to Juventus (10 days before season end)
    //   Season 2026/27 — plays at Juventus, then loaned to Torino in January
    //   Season end — loan returns, new season starts
    //
    // These tests verify that career history is correct across season
    // boundaries with transfers and loans, and that no phantom entries appear.

    /// Helper: pretty-print history for assertion messages
    fn describe_history(items: &[PlayerStatisticsHistoryItem]) -> String {
        items.iter().enumerate().map(|(i, e)| {
            format!(
                "  [{}] {}: {} | {} | apps={} | fee={:?}",
                i, e.season.display, e.team_slug,
                if e.is_loan { "LOAN" } else { "PERM" },
                e.statistics.played,
                e.transfer_fee,
            )
        }).collect::<Vec<_>>().join("\n")
    }

    // ---------------------------------------------------------------
    // Full season at one club, transfer near season end, then loan
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_full_season_then_late_transfer_then_loan() {
        let mut player = make_player();

        let roma = make_team("Roma", "roma");
        let juve = make_team("Juventus", "juventus");
        let torino = make_team("Torino", "torino");

        // -- Season 2025/26: full season at Roma, 30 apps --
        player.statistics_history.seed_initial_team(&roma, make_date(2025, 8, 1));
        player.statistics = make_stats(30, 8);
        player.on_season_end(Season::new(2025), &roma, make_date(2026, 8, 1));

        // -- Season 2026/27: start at Roma --
        player.statistics = make_stats(3, 1);

        // Late transfer: Roma → Juventus on May 21 (10 days before season end)
        player.on_transfer(&roma, &juve, 5_000_000.0, make_date(2027, 5, 21));

        // Play 0 games at Juve (only 10 days remain)
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &juve, make_date(2027, 8, 1));

        // -- Season 2027/28: at Juventus, loaned to Torino in January --
        player.statistics = make_stats(12, 3);
        player.on_loan(&juve, &torino, 100_000.0, make_date(2028, 1, 15));

        // Play 10 games at Torino on loan
        player.statistics = make_stats(10, 2);
        player.on_loan_return(&torino, make_date(2028, 5, 31));

        // Back at Juve, 0 more games before season end
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2027), &juve, make_date(2028, 8, 1));

        // -- Verify frozen history --
        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Roma 30 apps
        let roma_2025 = history.iter().find(|e| e.season.start_year == 2025 && e.team_slug == "roma");
        assert!(roma_2025.is_some(), "Missing Roma 2025/26 entry.\n{desc}");
        assert_eq!(roma_2025.unwrap().statistics.played, 30, "Roma 2025/26 apps wrong.\n{desc}");
        assert!(!roma_2025.unwrap().is_loan, "Roma 2025/26 should not be loan.\n{desc}");

        // 2026/27: Roma 3 apps (before transfer)
        let roma_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "roma");
        assert!(roma_2026.is_some(), "Missing Roma 2026/27 entry.\n{desc}");
        assert_eq!(roma_2026.unwrap().statistics.played, 3, "Roma 2026/27 apps wrong.\n{desc}");

        // 2026/27: Juventus 0 apps (arrived 10 days before end)
        let juve_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(juve_2026.is_some(), "Missing Juve 2026/27 entry.\n{desc}");
        assert_eq!(juve_2026.unwrap().statistics.played, 0, "Juve 2026/27 apps wrong.\n{desc}");
        assert_eq!(juve_2026.unwrap().transfer_fee, Some(5_000_000.0), "Juve 2026/27 fee wrong.\n{desc}");

        // 2027/28: Juventus 12 apps (before loan)
        let juve_2027 = history.iter().find(|e| e.season.start_year == 2027 && e.team_slug == "juventus");
        assert!(juve_2027.is_some(), "Missing Juve 2027/28 entry.\n{desc}");
        assert_eq!(juve_2027.unwrap().statistics.played, 12, "Juve 2027/28 apps wrong.\n{desc}");
        assert!(!juve_2027.unwrap().is_loan, "Juve 2027/28 should not be loan.\n{desc}");

        // 2027/28: Torino 10 apps (loan)
        let torino_2027 = history.iter().find(|e| e.season.start_year == 2027 && e.team_slug == "torino");
        assert!(torino_2027.is_some(), "Missing Torino 2027/28 loan entry.\n{desc}");
        assert_eq!(torino_2027.unwrap().statistics.played, 10, "Torino 2027/28 apps wrong.\n{desc}");
        assert!(torino_2027.unwrap().is_loan, "Torino 2027/28 should be loan.\n{desc}");

        // No phantom entries — exactly 5 history rows
        assert_eq!(history.len(), 5, "Expected 5 history entries, got {}.\n{desc}", history.len());

        // Current season (2028/29) should have 1 seeded entry for Juve
        assert_eq!(player.statistics_history.current.len(), 1, "Current should have 1 seed entry");
        assert_eq!(player.statistics_history.current[0].team_slug, "juventus");
    }

    // ---------------------------------------------------------------
    // Loan across season boundary: stale seed must not create phantom
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_loan_across_season_boundary_no_phantom() {
        let mut player = make_player();

        let inter = make_team("Inter", "inter");
        let monza = make_team("Monza", "monza");

        // -- Season 2025/26: at Inter --
        player.statistics_history.seed_initial_team(&inter, make_date(2025, 8, 1));
        player.statistics = make_stats(25, 5);

        // Loaned to Monza in January
        player.on_loan(&inter, &monza, 50_000.0, make_date(2026, 1, 10));
        player.statistics = make_stats(14, 3);

        // Season end snapshot: player still on loan at Monza
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500, make_date(2026, 5, 31), 99, 0, 100,
        ));
        player.on_season_end(Season::new(2025), &monza, make_date(2026, 8, 1));

        // Loan return (happens after snapshot, just like real game)
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&monza, make_date(2026, 6, 1));
        player.contract_loan = None;

        // -- Season 2026/27: back at Inter, full season --
        player.statistics = make_stats(28, 6);
        player.on_season_end(Season::new(2026), &inter, make_date(2027, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Inter 25 apps (before loan)
        let inter_2025 = history.iter().find(|e| e.season.start_year == 2025 && e.team_slug == "inter");
        assert!(inter_2025.is_some(), "Missing Inter 2025/26.\n{desc}");
        assert_eq!(inter_2025.unwrap().statistics.played, 25, "Inter 2025/26 apps wrong.\n{desc}");

        // 2025/26: Monza 14 apps (loan)
        let monza_2025 = history.iter().find(|e| e.season.start_year == 2025 && e.team_slug == "monza");
        assert!(monza_2025.is_some(), "Missing Monza 2025/26 loan.\n{desc}");
        assert_eq!(monza_2025.unwrap().statistics.played, 14, "Monza 2025/26 apps wrong.\n{desc}");
        assert!(monza_2025.unwrap().is_loan, "Monza 2025/26 should be loan.\n{desc}");

        // 2026/27: Inter 28 apps
        let inter_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "inter");
        assert!(inter_2026.is_some(), "Missing Inter 2026/27.\n{desc}");
        assert_eq!(inter_2026.unwrap().statistics.played, 28, "Inter 2026/27 apps wrong.\n{desc}");

        // NO phantom Monza entry in 2026/27
        let monza_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "monza");
        assert!(monza_2026.is_none(), "Phantom Monza in 2026/27 — stale seed not cleaned.\n{desc}");

        assert_eq!(history.len(), 3, "Expected 3 entries, got {}.\n{desc}", history.len());
    }

    // ---------------------------------------------------------------
    // Two consecutive loans: no phantom from first loan in second season
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_two_consecutive_loans_no_phantom() {
        let mut player = make_player();

        let gzira = make_team("Gzira United", "gzira");
        let birkirkara = make_team("Birkirkara", "birkirkara");
        let marsaxlokk = make_team("Marsaxlokk", "marsaxlokk");

        // -- Setup: player at Gzira --
        player.statistics_history.seed_initial_team(&gzira, make_date(2025, 8, 1));

        // -- Season 2025/26: loaned to Birkirkara --
        player.statistics = make_stats(0, 0);
        player.on_loan(&gzira, &birkirkara, 3_000.0, make_date(2025, 8, 7));
        player.statistics = make_stats(21, 3);

        // Season end while on loan at Birkirkara
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            200, make_date(2026, 5, 31), 99, 0, 100,
        ));
        player.on_season_end(Season::new(2025), &birkirkara, make_date(2026, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&birkirkara, make_date(2026, 6, 1));
        player.contract_loan = None;

        // -- Season 2026/27: at Gzira, then loaned to Marsaxlokk --
        player.statistics = make_stats(1, 0);
        player.on_loan(&gzira, &marsaxlokk, 200.0, make_date(2027, 1, 20));
        player.statistics = make_stats(0, 0);

        // Season end while on loan at Marsaxlokk
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            200, make_date(2027, 5, 31), 99, 0, 100,
        ));
        player.on_season_end(Season::new(2026), &marsaxlokk, make_date(2027, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&marsaxlokk, make_date(2027, 6, 1));
        player.contract_loan = None;

        // -- Season 2027/28: back at Gzira, plays full season --
        player.statistics = make_stats(20, 4);
        player.on_season_end(Season::new(2027), &gzira, make_date(2028, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Gzira 0 apps for 9 days (< 3% of season) — collapsed
        let gzira_2025 = history.iter().find(|e| e.season.start_year == 2025 && e.team_slug == "gzira");
        assert!(gzira_2025.is_none(), "Gzira 2025/26 (0 apps, 9 days) should be collapsed.\n{desc}");

        // 2025/26: Birkirkara 21 apps (loan)
        let birk_2025 = history.iter().find(|e| e.season.start_year == 2025 && e.team_slug == "birkirkara");
        assert!(birk_2025.is_some(), "Missing Birkirkara 2025/26.\n{desc}");
        assert_eq!(birk_2025.unwrap().statistics.played, 21, "Birkirkara 2025/26 apps wrong.\n{desc}");
        assert!(birk_2025.unwrap().is_loan, "Birkirkara should be loan.\n{desc}");

        // 2026/27: Gzira 1 app + Marsaxlokk 0 apps (loan)
        let gzira_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "gzira");
        assert!(gzira_2026.is_some(), "Missing Gzira 2026/27.\n{desc}");
        assert_eq!(gzira_2026.unwrap().statistics.played, 1, "Gzira 2026/27 apps wrong.\n{desc}");

        let mars_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "marsaxlokk");
        assert!(mars_2026.is_some(), "Missing Marsaxlokk 2026/27.\n{desc}");
        assert!(mars_2026.unwrap().is_loan, "Marsaxlokk should be loan.\n{desc}");

        // 2027/28: Gzira 20 apps
        let gzira_2027 = history.iter().find(|e| e.season.start_year == 2027 && e.team_slug == "gzira");
        assert!(gzira_2027.is_some(), "Missing Gzira 2027/28.\n{desc}");
        assert_eq!(gzira_2027.unwrap().statistics.played, 20, "Gzira 2027/28 apps wrong.\n{desc}");

        // NO phantom Birkirkara in 2026/27 or 2027/28
        let birk_phantom = history.iter().find(|e| e.season.start_year >= 2026 && e.team_slug == "birkirkara");
        assert!(birk_phantom.is_none(), "Phantom Birkirkara in later season.\n{desc}");

        // NO phantom Marsaxlokk in 2027/28
        let mars_phantom = history.iter().find(|e| e.season.start_year == 2027 && e.team_slug == "marsaxlokk");
        assert!(mars_phantom.is_none(), "Phantom Marsaxlokk in 2027/28.\n{desc}");

        // 4 entries: Birkirkara + (Gzira + Marsaxlokk) + Gzira  (first Gzira collapsed)
        assert_eq!(history.len(), 4, "Expected 4 entries, got {}.\n{desc}", history.len());
    }

    // ---------------------------------------------------------------
    // Transfer + immediate loan in same season (0 apps at buying club)
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_transfer_then_immediate_loan_zero_apps() {
        let mut player = make_player();

        let napoli = make_team("Napoli", "napoli");
        let juve = make_team("Juventus", "juventus");
        let empoli = make_team("Empoli", "empoli");

        // -- Season 2025/26: at Napoli, 20 apps --
        player.statistics_history.seed_initial_team(&napoli, make_date(2025, 8, 1));
        player.statistics = make_stats(20, 5);
        player.on_season_end(Season::new(2025), &napoli, make_date(2026, 8, 1));

        // -- Season 2026/27: transferred to Juve, immediately loaned to Empoli --
        player.statistics = make_stats(0, 0);
        player.on_transfer(&napoli, &juve, 2_000_000.0, make_date(2026, 8, 15));
        player.on_loan(&juve, &empoli, 30_000.0, make_date(2026, 8, 20));

        // Play 18 games at Empoli
        player.statistics = make_stats(18, 4);
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            300, make_date(2027, 5, 31), 99, 0, 100,
        ));
        player.on_season_end(Season::new(2026), &empoli, make_date(2027, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&empoli, make_date(2027, 6, 1));
        player.contract_loan = None;

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Napoli 20 apps
        let napoli_2025 = history.iter().find(|e| e.season.start_year == 2025 && e.team_slug == "napoli");
        assert!(napoli_2025.is_some(), "Missing Napoli 2025/26.\n{desc}");
        assert_eq!(napoli_2025.unwrap().statistics.played, 20);

        // 2026/27: Juve 0 apps (bought, never played, loaned out same week)
        let juve_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(juve_2026.is_some(), "Missing Juve 2026/27 — player was bought even if 0 apps.\n{desc}");
        assert_eq!(juve_2026.unwrap().statistics.played, 0, "Juve should have 0 apps.\n{desc}");
        assert!(!juve_2026.unwrap().is_loan, "Juve entry should be permanent.\n{desc}");
        assert_eq!(juve_2026.unwrap().transfer_fee, Some(2_000_000.0), "Juve fee wrong.\n{desc}");

        // 2026/27: Empoli 18 apps (loan)
        let empoli_2026 = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "empoli");
        assert!(empoli_2026.is_some(), "Missing Empoli 2026/27 loan.\n{desc}");
        assert_eq!(empoli_2026.unwrap().statistics.played, 18, "Empoli apps wrong.\n{desc}");
        assert!(empoli_2026.unwrap().is_loan, "Empoli should be loan.\n{desc}");

        // No phantom Empoli in future seasons
        let empoli_phantom = history.iter().find(|e| e.season.start_year == 2027 && e.team_slug == "empoli");
        assert!(empoli_phantom.is_none(), "Phantom Empoli in 2027/28.\n{desc}");
    }

    // ---------------------------------------------------------------
    // Collapse: loan returns 5 days before season end → parent club
    // stint with 0 apps should be dropped (< 3% of season)
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_brief_return_before_season_end_is_collapsed() {
        let mut player = make_player();

        let gzira = make_team("Gzira United", "gzira");
        let mosta = make_team("Mosta", "mosta");

        // -- Season 2025/26: at Gzira, loaned to Mosta early --
        player.statistics_history.seed_initial_team(&gzira, make_date(2025, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan(&gzira, &mosta, 200.0, make_date(2025, 8, 10));

        // Play 18 games at Mosta
        player.statistics = make_stats(18, 5);
        player.on_loan_return(&mosta, make_date(2026, 5, 26));
        player.contract_loan = None;

        // Back at Gzira for just 5 days, 0 games (season ends May 31)
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2025), &gzira, make_date(2026, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Mosta loan: 18 apps — must be kept
        let mosta_entry = history.iter().find(|e| e.team_slug == "mosta");
        assert!(mosta_entry.is_some(), "Missing Mosta loan entry.\n{desc}");
        assert_eq!(mosta_entry.unwrap().statistics.played, 18, "Mosta apps wrong.\n{desc}");
        assert!(mosta_entry.unwrap().is_loan, "Mosta should be loan.\n{desc}");

        // Gzira 0 apps for 5 days (< 3% of season) — should be COLLAPSED (dropped)
        let gzira_brief = history.iter().find(|e| {
            e.season.start_year == 2025 && e.team_slug == "gzira"
                && e.statistics.played == 0 && e.transfer_fee.is_none()
        });
        assert!(gzira_brief.is_none(),
            "Brief 0-app return to Gzira before season end should be collapsed.\n{desc}");
    }

    // ---------------------------------------------------------------
    // Collapse does NOT drop entries with apps or transfer fees
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_brief_stint_with_apps_is_kept() {
        let mut player = make_player();

        let gzira = make_team("Gzira United", "gzira");
        let mosta = make_team("Mosta", "mosta");

        player.statistics_history.seed_initial_team(&gzira, make_date(2025, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan(&gzira, &mosta, 200.0, make_date(2025, 8, 10));

        player.statistics = make_stats(18, 5);
        player.on_loan_return(&mosta, make_date(2026, 5, 26));
        player.contract_loan = None;

        // Back at Gzira for 5 days BUT played 1 game (sub appearance)
        player.statistics = make_stats(0, 0);
        player.statistics.played_subs = 1;
        player.on_season_end(Season::new(2025), &gzira, make_date(2026, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Gzira entry with 1 sub appearance — must be KEPT despite short stay
        let gzira_entry = history.iter().find(|e| {
            e.season.start_year == 2025 && e.team_slug == "gzira" && !e.is_loan
        });
        assert!(gzira_entry.is_some(),
            "Gzira with 1 sub app should be kept even for brief stint.\n{desc}");
        // played_subs merged into played at drain time
        assert_eq!(gzira_entry.unwrap().statistics.played, 1,
            "Gzira apps wrong (sub should be merged).\n{desc}");
    }

    // ---------------------------------------------------------------
    // Collapse: transfer fee protects a 0-app entry from being dropped
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_brief_stint_with_fee_is_kept() {
        let mut player = make_player();

        let napoli = make_team("Napoli", "napoli");
        let juve = make_team("Juventus", "juventus");
        let torino = make_team("Torino", "torino");

        player.statistics_history.seed_initial_team(&napoli, make_date(2025, 8, 1));
        player.statistics = make_stats(20, 5);
        player.on_season_end(Season::new(2025), &napoli, make_date(2026, 8, 1));

        // Transferred to Juve 3 days before season end, 0 apps
        player.statistics = make_stats(2, 0);
        player.on_transfer(&napoli, &juve, 10_000_000.0, make_date(2027, 5, 28));
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &juve, make_date(2027, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Juve 0 apps, only 3 days, BUT has a 10M transfer fee — must be kept
        let juve_entry = history.iter().find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(juve_entry.is_some(),
            "Juve with transfer fee must be kept even for 0 apps / 3 days.\n{desc}");
        assert_eq!(juve_entry.unwrap().transfer_fee, Some(10_000_000.0),
            "Juve fee wrong.\n{desc}");
    }

    // ---------------------------------------------------------------
    // Long 0-app parent stint (>3% of season) is NOT collapsed
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_long_zero_app_stint_is_kept() {
        let mut player = make_player();

        let roma = make_team("Roma", "roma");
        let torino = make_team("Torino", "torino");

        // Season 2025/26: at Roma, loaned to Torino, returns 2 months early
        player.statistics_history.seed_initial_team(&roma, make_date(2025, 8, 1));
        player.statistics = make_stats(2, 0);
        player.on_loan(&roma, &torino, 30_000.0, make_date(2025, 9, 1));

        player.statistics = make_stats(15, 3);
        player.on_loan_return(&torino, make_date(2026, 3, 31));
        player.contract_loan = None;

        // Back at Roma for ~60 days (April + May), 0 games — but 20% of season
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2025), &roma, make_date(2026, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Roma 0 apps for 60 days (~20% of season) — should be KEPT
        let roma_entries: Vec<_> = history.iter()
            .filter(|e| e.season.start_year == 2025 && e.team_slug == "roma")
            .collect();
        assert!(!roma_entries.is_empty(),
            "Roma 0-app entry for 60 days (20%% of season) should be kept.\n{desc}");
    }
}
