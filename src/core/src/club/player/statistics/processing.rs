use crate::PlayerStatistics;
use crate::TeamInfo;
use crate::club::player::player::Player;
use crate::continent::competitions::{
    CHAMPIONS_LEAGUE_SLUG, CONFERENCE_LEAGUE_SLUG, COPA_LIBERTADORES_SLUG, EUROPA_LEAGUE_SLUG,
};
use crate::league::Season;
use chrono::{Datelike, NaiveDate};

/// True for the four continental club competitions whose per-season
/// appearances are folded into the player history page's league line.
/// Domestic cups (real `League`s with their own slugs) are deliberately
/// excluded — only international competitions are merged.
fn is_continental_slug(slug: &str) -> bool {
    matches!(
        slug,
        CHAMPIONS_LEAGUE_SLUG | EUROPA_LEAGUE_SLUG | CONFERENCE_LEAGUE_SLUG | COPA_LIBERTADORES_SLUG
    )
}

impl Player {
    /// Clear the cup tally as a unit: the rolled-up aggregate *and* the
    /// per-competition breakdown it's rebuilt from. They must always
    /// reset together, otherwise the aggregate would keep summing buckets
    /// that have been wiped (or vice-versa).
    fn reset_cup_statistics(&mut self) {
        self.cup_statistics = PlayerStatistics::default();
        self.cup_statistics_by_competition.clear();
    }

    /// Aggregate the player's current-spell statistics across the four
    /// continental club competitions (Champions League, Europa League,
    /// Conference League, Copa Libertadores). These live in the per-spell
    /// cup breakdown until the spell closes; the player history page folds
    /// them into the season's league line, and [`Self::record_continental_spell`]
    /// freezes them into the per-season ledger when the spell ends.
    pub fn continental_cup_statistics(&self) -> PlayerStatistics {
        let mut total = PlayerStatistics::default();
        for comp in &self.cup_statistics_by_competition {
            if is_continental_slug(&comp.competition_slug) {
                total.merge_from(&comp.statistics);
            }
        }
        total
    }

    /// Freeze the current spell's continental-cup statistics into the
    /// per-season ledger for `team`, attributing them to the season that
    /// contains `date`. One ledger row per continental tournament — the
    /// History page tooltip surfaces Champions League / Europa League /
    /// Conference League / Copa Libertadores individually instead of as
    /// one aggregated line. Called immediately before the live cup bucket
    /// is reset so a transfer / loan / season boundary doesn't discard the
    /// player's continental appearances.
    fn record_continental_spell(&mut self, season_year: u16, team: &TeamInfo) {
        // Snapshot to avoid borrowing `self` immutably while calling a
        // mutating helper.
        let slices: Vec<(String, PlayerStatistics)> = self
            .cup_statistics_by_competition
            .iter()
            .filter(|c| is_continental_slug(&c.competition_slug))
            .map(|c| (c.competition_slug.clone(), c.statistics.clone()))
            .collect();
        for (slug, stats) in slices {
            self.statistics_history
                .record_continental(season_year, team, slug, stats);
        }
    }

    /// Freeze the current spell's domestic-cup statistics into the
    /// per-season ledger for `team`. One ledger row per domestic cup
    /// (FA Cup, League Cup, etc.) instead of an aggregated line. Paired
    /// with [`Self::record_continental_spell`] at every boundary that
    /// resets the live cup buckets.
    fn record_domestic_cup_spell(&mut self, season_year: u16, team: &TeamInfo) {
        let slices: Vec<(String, PlayerStatistics)> = self
            .cup_statistics_by_competition
            .iter()
            .filter(|c| !is_continental_slug(&c.competition_slug))
            .map(|c| (c.competition_slug.clone(), c.statistics.clone()))
            .collect();
        for (slug, stats) in slices {
            self.statistics_history
                .record_domestic_cup(season_year, team, slug, stats);
        }
    }

    /// Freeze the current spell's friendly-bucket statistics into the
    /// per-season ledger under `team`. `source_slug` is the competition
    /// slug stamped on the ledger entry — `team.league_slug` for the
    /// senior path, the youth team's league slug for a U18..U23 aliased
    /// spell so the breakdown labels the row with the youth league name.
    fn record_friendly_spell_with_source(
        &mut self,
        season_year: u16,
        team: &TeamInfo,
        source_slug: String,
    ) {
        let friendly = self.friendly_statistics.clone();
        self.statistics_history
            .record_friendly(season_year, team, source_slug, friendly);
    }

    /// The single chokepoint for "this spell is done — freeze its match
    /// stats and reset the live buckets for the next spell." Every
    /// inter-spell event (transfer, loan, loan-return, release,
    /// cancel-loan, manual-*) and every season-end (senior + youth)
    /// flows through here, so the rule that EVERY bucket must be
    /// recorded before being cleared lives in one place — not eight.
    ///
    /// The duplicated per-handler ritual the old code used to keep was
    /// a foot-gun: a previously-shipped `on_loan_return` cleared
    /// nothing, leaking the loan period's friendlies into the parent
    /// spell. Centralising the drain removes the failure mode entirely.
    ///
    /// Returns the drained League counter so the caller hands it off to
    /// the matching `statistics_history.record_*` method without
    /// re-reaching into `self.statistics`.
    ///
    /// Source resolution for the canonical Friendly ledger entry, in
    /// priority order:
    ///   1. explicit `friendly_source_slug` argument (the non-senior
    ///      season-end path passes the youth team's league_slug),
    ///   2. `player.friendly_source_slug` captured at match-record time
    ///      (the only path that knows the actual youth league a senior
    ///      loanee played friendlies in),
    ///   3. `team.league_slug` (senior pre-season friendlies — the row
    ///      then renders as the generic "Friendly").
    fn drain_match_stats(
        &mut self,
        team: &TeamInfo,
        season_year: u16,
        friendly_source_slug: Option<String>,
    ) -> PlayerStatistics {
        self.record_continental_spell(season_year, team);
        self.record_domestic_cup_spell(season_year, team);
        let recorded_source = self.friendly_source_slug.take();
        let friendly_slug = friendly_source_slug
            .or(recorded_source)
            .unwrap_or_else(|| team.league_slug.clone());
        self.record_friendly_spell_with_source(season_year, team, friendly_slug);
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.reset_cup_statistics();
        stats
    }

    /// Record a permanent transfer (called by transfer execution).
    pub fn on_transfer(&mut self, from: &TeamInfo, to: &TeamInfo, fee: f64, date: NaiveDate) {
        let stats = self.drain_match_stats(from, Season::from_date(date).start_year, None);
        self.statistics_history
            .record_transfer(stats, from, to, fee, date);
        self.last_transfer_date = Some(date);
    }

    /// Player reassigned across teams of the same club (Main ↔ B / Second /
    /// Reserve / youth). Closes the previous spell and opens a new one on
    /// the destination so future match stats accumulate against the team
    /// the player actually plays for. No fee, no `last_transfer_date`
    /// touch — this isn't a market move.
    ///
    /// `from_senior` / `to_senior` decide which sides land in career
    /// history. Only senior squads (Main, B, Second) are eligible — any
    /// involvement of Reserve / U18..U23 is treated as silent for the
    /// player history table even though stats are still drained.
    ///
    /// `friendly_statistics` and `cup_statistics` are intentionally NOT
    /// cleared: a soft same-club move shouldn't discard buckets the
    /// player page would otherwise show. Inter-club moves (transfer /
    /// loan) reset them via their own `drain_match_stats` calls.
    pub fn on_intra_club_move(
        &mut self,
        from: &TeamInfo,
        to: &TeamInfo,
        from_senior: bool,
        to_senior: bool,
        date: NaiveDate,
    ) {
        // Drain `player.statistics` only when leaving a SENIOR team —
        // those games belong to the FROM spell and `record_intra_club_move`
        // needs them to close the row. For non-senior moves the live
        // counter holds senior-callup games earned while on the youth
        // squad; leave them in place so the next senior or non-senior
        // season-end drain routes them into the Main-alias row.
        let stats = if from_senior {
            std::mem::take(&mut self.statistics)
        } else {
            PlayerStatistics::default()
        };
        self.statistics_history.record_intra_club_move(
            stats,
            from,
            to,
            from_senior,
            to_senior,
            date,
        );
    }

    /// Season-end snapshot for a player sitting on a non-senior squad
    /// (Reserve, U18..U23). Non-owning teams never appear under their
    /// own slug — instead the player gets a row under the parent club's
    /// main team.
    ///
    /// `player.statistics` accumulates ONLY senior-league appearances
    /// because youth leagues are created with `friendly: true` (see
    /// `database/src/generators/generator/leagues.rs`) — that routes
    /// U21/Reserve/U18 league matches to `player.friendly_statistics`
    /// instead. So whatever sits in `player.statistics` at this point
    /// is exclusively Main-team callup games the youth player earned
    /// during the season, and those MUST flow into the Main-team row.
    ///
    /// Friendly (youth-league) and cup buckets are reset because they
    /// don't roll into career history — only `player.statistics` does.
    pub fn on_non_senior_season_end(
        &mut self,
        season: Season,
        main_team_info: &TeamInfo,
        youth_team_info: &TeamInfo,
        _date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        // Drain under the Main-aliased team but stamp the Friendly
        // entry with the YOUTH league slug so the History breakdown
        // labels the row with the actual youth league name (e.g.
        // "Russian Premier League U19") rather than the senior parent.
        let stats = self.drain_match_stats(
            main_team_info,
            season.start_year,
            Some(youth_team_info.league_slug.clone()),
        );
        self.statistics_history.record_season_end(
            season,
            stats,
            main_team_info,
            is_loan,
            self.last_transfer_date,
        );
        // Buy-back protection only needs to last one season — same
        // contract as `on_season_end`.
        self.sold_from = None;
    }

    /// Record a loan move (called by loan execution).
    pub fn on_loan(&mut self, from: &TeamInfo, to: &TeamInfo, loan_fee: f64, date: NaiveDate) {
        let stats = self.drain_match_stats(from, Season::from_date(date).start_year, None);
        self.statistics_history
            .record_loan(stats, from, to, loan_fee, date);
        self.last_transfer_date = Some(date);
    }

    /// Record a loan return (called at end of loan period). The
    /// borrowing club is treated as the source spell — its friendlies /
    /// cups are frozen under the BORROWING team before the live buckets
    /// reset; otherwise the loan-period games leak into the parent
    /// spell that starts fresh on return.
    pub fn on_loan_return(&mut self, borrowing: &TeamInfo, parent: &TeamInfo, date: NaiveDate) {
        let stats = self.drain_match_stats(borrowing, Season::from_date(date).start_year, None);
        self.statistics_history
            .record_loan_return(stats, borrowing, parent, date);
        self.last_transfer_date = Some(date);
    }

    /// Record season-end snapshot (called when new season starts).
    pub fn on_season_end(&mut self, season: Season, team: &TeamInfo, _date: NaiveDate) {
        let is_loan = self.is_on_loan();
        let stats = self.drain_match_stats(team, season.start_year, None);
        self.statistics_history.record_season_end(
            season,
            stats,
            team,
            is_loan,
            self.last_transfer_date,
        );
        // Preserve last_transfer_date across seasons — clearing it destroyed
        // the settling-in protection that prevents clubs from immediately
        // dumping recently-signed players.  The date is already archived in
        // statistics_history, so downstream reads are unaffected.

        // Clear sold_from at season end — the buy-back protection only needs
        // to last one season to prevent same-window or next-window re-purchases.
        self.sold_from = None;
    }

    /// Catch-up snapshot for a season whose `new_season_started` league
    /// gate never fired — the watermark loop is closing the gap N years
    /// later. Live `statistics` / `cup_statistics` / `friendly_statistics`
    /// have been carrying stats across every missed year and the *real*
    /// target year, so we cannot split them per season. Attribute the
    /// drained totals to the target-year `on_season_end` call that fires
    /// alongside this one; here we only freeze a 0-app placeholder row so
    /// the player's career thread still shows a row for the gap year
    /// (when the row survives the trivial-stint / stale-loan-seed
    /// filters in `record_season_end`).
    pub fn on_missed_season_end(
        &mut self,
        season: Season,
        team: &TeamInfo,
        _date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        self.statistics_history.record_season_end(
            season,
            PlayerStatistics::default(),
            team,
            is_loan,
            self.last_transfer_date,
        );
        self.sold_from = None;
    }

    /// Evaluate whether a club should become a favourite based on career history.
    /// Called at season end. Mirrors FM logic:
    /// - Youth club: first club where player was aged 16-21, after 2+ seasons
    /// - Long service: 100+ appearances at a club
    /// - Legend status: 50+ goals or 15+ player-of-the-match awards
    /// - Strong impact: average rating >= 7.3 over 30+ games
    /// Max 3 favourite clubs per player.
    pub fn evaluate_favorite_club(&mut self, club_id: u32, team_slug: &str, _date: NaiveDate) {
        const MAX_FAVORITE_CLUBS: usize = 3;

        if self.favorite_clubs.len() >= MAX_FAVORITE_CLUBS {
            return;
        }
        if self.favorite_clubs.contains(&club_id) {
            return;
        }

        // Aggregate stats across all history items for this club via
        // the ledger-aware merge so cameo-heavy spells weight less than
        // full-starter ones. Loyalty calc is then anchored on the
        // sample-size-regressed value (a 4-game farewell season at
        // raw 7.5 shouldn't carry the same weight as a proven 30-game
        // year).
        let mut total_apps: u16 = 0;
        let mut total_goals: u16 = 0;
        let mut total_pom: u16 = 0;
        let mut combined_stats = PlayerStatistics::default();
        let mut seasons_at_club: u16 = 0;
        let mut first_season_year: Option<u16> = None;

        for item in &self.statistics_history.items {
            if item.team_slug != team_slug {
                continue;
            }
            let games = item.statistics.played + item.statistics.played_subs;
            total_apps += games;
            total_goals += item.statistics.goals;
            total_pom += item.statistics.player_of_the_match as u16;
            combined_stats.merge_from(&item.statistics);
            seasons_at_club += 1;
            if first_season_year.is_none() || item.season.start_year < first_season_year.unwrap() {
                first_season_year = Some(item.season.start_year);
            }
        }

        // Also count current-season entries
        for entry in &self.statistics_history.current {
            if entry.team_slug != team_slug {
                continue;
            }
            let games = entry.statistics.played + entry.statistics.played_subs;
            total_apps += games;
            total_goals += entry.statistics.goals;
            total_pom += entry.statistics.player_of_the_match as u16;
            combined_stats.merge_from(&entry.statistics);
        }

        let pos = self.position().position_group();
        let avg_rating = combined_stats.average_rating_realistic(pos);

        // Youth club: first club where player was aged 16-21, after 2+ seasons
        if let Some(first_year) = first_season_year {
            let age_at_first = first_year as i32 - self.birth_date.year();
            if (16..=21).contains(&age_at_first) && seasons_at_club >= 2 {
                self.favorite_clubs.push(club_id);
                return;
            }
        }

        // Long service: 100+ competitive appearances
        if total_apps >= 100 {
            self.favorite_clubs.push(club_id);
            return;
        }

        // Legend: prolific scorer or multiple POM awards
        if total_goals >= 50 || total_pom >= 15 {
            self.favorite_clubs.push(club_id);
            return;
        }

        // Strong impact: consistently high performer over a meaningful
        // sample. Threshold is on total apps now (was rated_games before
        // the ledger merge); the regression already protects against
        // small-sample inflated averages.
        if total_apps >= 30 && avg_rating >= 7.3 {
            self.favorite_clubs.push(club_id);
        }
    }

    /// Record a cancel-loan from the web UI.
    pub fn on_cancel_loan(&mut self, borrowing: &TeamInfo, parent: &TeamInfo, date: NaiveDate) {
        let is_loan = self.is_on_loan();
        let stats = self.drain_match_stats(borrowing, Season::from_date(date).start_year, None);
        self.statistics_history
            .record_cancel_loan(stats, borrowing, parent, is_loan, date);
        self.last_transfer_date = Some(date);
    }

    /// Record a manual transfer from the web UI.
    pub fn on_manual_transfer(
        &mut self,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: Option<f64>,
        date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        let stats = self.drain_match_stats(from, Season::from_date(date).start_year, None);
        self.statistics_history
            .record_departure_transfer(stats, from, to, fee, is_loan, date);
        self.last_transfer_date = Some(date);
        self.is_force_match_selection = false;
    }

    /// React to being released into the free-agent pool. Snapshots the
    /// in-flight match stats onto the source club's career entry and
    /// marks it as departed, so games the player accumulated before
    /// the release stay attributed to the club where they were played —
    /// not to a synthetic "Free Agent" row at the next signing. Caller
    /// is responsible for clearing contract / statuses / happiness;
    /// this method only owns the history side.
    pub fn on_release(&mut self, from: &TeamInfo, date: NaiveDate) {
        let stats = self.drain_match_stats(from, Season::from_date(date).start_year, None);
        self.statistics_history.record_release(stats, from, date);
        self.last_transfer_date = Some(date);
        self.is_force_match_selection = false;
    }

    /// Record a manual signing of a free agent. There is no source club
    /// to attribute stats to: the prior club's `on_release` already
    /// drained the live buckets, and a player sitting in the free-agent
    /// pool plays no matches. This invariant — live cup / friendly
    /// buckets MUST be empty here — is checked in debug builds; release
    /// builds clear defensively so a soft regression cannot silently
    /// orphan a non-League slice.
    pub fn on_free_agent_signing(&mut self, to: &TeamInfo, date: NaiveDate) {
        debug_assert!(
            self.friendly_statistics.total_games() == 0
                && self.cup_statistics_by_competition.is_empty(),
            "on_free_agent_signing invariant violated: live non-League buckets must be \
             empty (on_release should have drained them); friendly_games={}, cup_slices={}",
            self.friendly_statistics.total_games(),
            self.cup_statistics_by_competition.len(),
        );
        let stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();
        self.reset_cup_statistics();
        self.statistics_history
            .record_free_agent_signing(stats, to, date);
        self.last_transfer_date = Some(date);
        self.is_force_match_selection = false;
    }

    /// Record a manual loan from the web UI.
    pub fn on_manual_loan(
        &mut self,
        from: &TeamInfo,
        parent: &TeamInfo,
        to: &TeamInfo,
        date: NaiveDate,
    ) {
        let is_loan = self.is_on_loan();
        let stats = self.drain_match_stats(from, Season::from_date(date).start_year, None);
        self.statistics_history
            .record_departure_loan(stats, from, parent, to, is_loan, date);
        self.last_transfer_date = Some(date);
        self.is_force_match_selection = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills, PlayerStatistics,
        PlayerStatisticsHistoryItem,
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
        let juve = player
            .statistics_history
            .current
            .iter()
            .find(|e| e.team_slug == "juventus");
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
        let torino = player
            .statistics_history
            .current
            .iter()
            .find(|e| e.team_slug == "torino");
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
            departed_date: None,
            joined_date: make_date(2032, 1, 15),
            seq_id: 0,
        });

        let borrowing = make_team("Torino", "torino");
        let parent = make_team("Juventus", "juventus");
        player.on_loan_return(&borrowing, &parent, make_date(2032, 5, 31));

        assert_eq!(player.statistics.played, 0);
        // Upsert updates existing Torino loan entry with 15 games
        let torino = player
            .statistics_history
            .current
            .iter()
            .find(|e| e.team_slug == "torino" && e.is_loan)
            .unwrap();
        assert_eq!(torino.statistics.played, 15);
        assert_eq!(torino.transfer_fee, Some(50_000.0));
    }

    #[test]
    fn on_loan_return_freezes_borrowing_club_friendly_and_cup_stats() {
        // User-reported bug: a returning loanee lost their loan-period
        // friendly / cup stats because on_loan_return didn't freeze
        // them under the borrowing club before the live buckets reset
        // (and now would, via drain_match_stats). Verifies the drain
        // chokepoint runs on this path too.
        let mut player = make_player();
        player.statistics = make_stats(15, 4);
        player.friendly_statistics = make_stats(3, 1);
        player.cup_statistics_by_competition.push(
            crate::CompetitionStatistics {
                competition_slug: CHAMPIONS_LEAGUE_SLUG.to_string(),
                statistics: make_stats(5, 2),
            },
        );

        let borrowing = make_team("Torino", "torino");
        let parent = make_team("Juventus", "juventus");
        player.on_loan_return(&borrowing, &parent, make_date(2032, 5, 31));

        // Live buckets cleared.
        assert_eq!(player.friendly_statistics.played, 0);
        assert!(player.cup_statistics_by_competition.is_empty());

        // Borrowing club's friendly + continental survive in the
        // canonical ledger under Torino — NOT under Juventus.
        let friendly_under_torino = player
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| {
                e.team_slug == "torino"
                    && e.competition_kind
                        == crate::PlayerStatCompetitionKind::Friendly
                    && e.statistics.played == 3
            });
        assert!(
            friendly_under_torino,
            "loan-period Friendly must be frozen under the borrowing club"
        );
        let continental_under_torino = player
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| {
                e.team_slug == "torino"
                    && e.competition_kind
                        == crate::PlayerStatCompetitionKind::ContinentalCup
                    && e.statistics.played == 5
            });
        assert!(
            continental_under_torino,
            "loan-period Continental must be frozen under the borrowing club"
        );
        // Nothing got mis-attributed to the parent club.
        let any_under_parent_non_league = player
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| {
                e.team_slug == "juventus"
                    && e.competition_kind != crate::PlayerStatCompetitionKind::League
            });
        assert!(
            !any_under_parent_non_league,
            "no loan-period non-League entry may land under the parent club"
        );
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
            500,
            make_date(2032, 5, 31),
            99,
            0,
            100,
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
            team_name: "Torino".to_string(),
            team_slug: "torino".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: true,
            transfer_fee: None,
            statistics: make_stats(15, 0),
            departed_date: None,
            joined_date: make_date(2032, 1, 1),
            seq_id: 0,
        });
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Juventus".to_string(),
            team_slug: "juventus".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: make_stats(10, 0),
            departed_date: None,
            joined_date: make_date(2031, 8, 1),
            seq_id: 1,
        });

        let team = make_team("Juventus", "juventus");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        // Both entries had games, both should be finalized
        assert_eq!(player.statistics_history.items.len(), 2);
        // current has 1 entry: seeded empty entry for new season
        assert_eq!(player.statistics_history.current.len(), 1);
        assert_eq!(player.statistics_history.current[0].team_slug, "juventus");
        assert_eq!(
            player.statistics_history.current[0]
                .statistics
                .total_games(),
            0
        );
    }

    #[test]
    fn on_season_end_merges_live_stats_into_current_team() {
        let mut player = make_player();
        player.statistics = make_stats(5, 2);

        // Two stints in current season
        use crate::club::player::statistics::history::CurrentSeasonEntry;
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Juventus".to_string(),
            team_slug: "juventus".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: make_stats(10, 0),
            departed_date: None,
            joined_date: make_date(2031, 8, 1),
            seq_id: 0,
        });
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Torino".to_string(),
            team_slug: "torino".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: true,
            transfer_fee: None,
            statistics: make_stats(15, 0),
            departed_date: None,
            joined_date: make_date(2032, 1, 1),
            seq_id: 1,
        });

        let team = make_team("Juventus", "juventus");
        player.on_season_end(Season::new(2031), &team, make_date(2032, 8, 1));

        // Season end merges current_stats (5 games) into the Juventus current entry
        let juve = player
            .statistics_history
            .items
            .iter()
            .find(|e| e.team_slug == "juventus")
            .unwrap();
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
        items
            .iter()
            .enumerate()
            .map(|(i, e)| {
                format!(
                    "  [{}] {}: {} | {} | apps={} | fee={:?}",
                    i,
                    e.season.display,
                    e.team_slug,
                    if e.is_loan { "LOAN" } else { "PERM" },
                    e.statistics.played,
                    e.transfer_fee,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        player
            .statistics_history
            .seed_initial_team(&roma, make_date(2025, 8, 1), false);
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
        player.on_loan_return(&torino, &juve, make_date(2028, 5, 31));

        // Back at Juve, 0 more games before season end
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2027), &juve, make_date(2028, 8, 1));

        // -- Verify frozen history --
        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Roma 30 apps
        let roma_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "roma");
        assert!(roma_2025.is_some(), "Missing Roma 2025/26 entry.\n{desc}");
        assert_eq!(
            roma_2025.unwrap().statistics.played,
            30,
            "Roma 2025/26 apps wrong.\n{desc}"
        );
        assert!(
            !roma_2025.unwrap().is_loan,
            "Roma 2025/26 should not be loan.\n{desc}"
        );

        // 2026/27: Roma 3 apps (before transfer)
        let roma_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "roma");
        assert!(roma_2026.is_some(), "Missing Roma 2026/27 entry.\n{desc}");
        assert_eq!(
            roma_2026.unwrap().statistics.played,
            3,
            "Roma 2026/27 apps wrong.\n{desc}"
        );

        // 2026/27: Juventus 0 apps (arrived 10 days before end)
        let juve_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(juve_2026.is_some(), "Missing Juve 2026/27 entry.\n{desc}");
        assert_eq!(
            juve_2026.unwrap().statistics.played,
            0,
            "Juve 2026/27 apps wrong.\n{desc}"
        );
        assert_eq!(
            juve_2026.unwrap().transfer_fee,
            Some(5_000_000.0),
            "Juve 2026/27 fee wrong.\n{desc}"
        );

        // 2027/28: Juventus 12 apps (before loan)
        let juve_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "juventus");
        assert!(juve_2027.is_some(), "Missing Juve 2027/28 entry.\n{desc}");
        assert_eq!(
            juve_2027.unwrap().statistics.played,
            12,
            "Juve 2027/28 apps wrong.\n{desc}"
        );
        assert!(
            !juve_2027.unwrap().is_loan,
            "Juve 2027/28 should not be loan.\n{desc}"
        );

        // 2027/28: Torino 10 apps (loan)
        let torino_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "torino");
        assert!(
            torino_2027.is_some(),
            "Missing Torino 2027/28 loan entry.\n{desc}"
        );
        assert_eq!(
            torino_2027.unwrap().statistics.played,
            10,
            "Torino 2027/28 apps wrong.\n{desc}"
        );
        assert!(
            torino_2027.unwrap().is_loan,
            "Torino 2027/28 should be loan.\n{desc}"
        );

        // No phantom entries — exactly 5 history rows
        assert_eq!(
            history.len(),
            5,
            "Expected 5 history entries, got {}.\n{desc}",
            history.len()
        );

        // Current season (2028/29) should have 1 seeded entry for Juve
        assert_eq!(
            player.statistics_history.current.len(),
            1,
            "Current should have 1 seed entry"
        );
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
        player
            .statistics_history
            .seed_initial_team(&inter, make_date(2025, 8, 1), false);
        player.statistics = make_stats(25, 5);

        // Loaned to Monza in January
        player.on_loan(&inter, &monza, 50_000.0, make_date(2026, 1, 10));
        player.statistics = make_stats(14, 3);

        // Season end snapshot: player still on loan at Monza
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2026, 5, 31),
            99,
            0,
            100,
        ));
        player.on_season_end(Season::new(2025), &monza, make_date(2026, 8, 1));

        // Loan return (happens after snapshot, just like real game)
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&monza, &inter, make_date(2026, 6, 1));
        player.contract_loan = None;

        // -- Season 2026/27: back at Inter, full season --
        player.statistics = make_stats(28, 6);
        player.on_season_end(Season::new(2026), &inter, make_date(2027, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Inter 25 apps (before loan)
        let inter_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "inter");
        assert!(inter_2025.is_some(), "Missing Inter 2025/26.\n{desc}");
        assert_eq!(
            inter_2025.unwrap().statistics.played,
            25,
            "Inter 2025/26 apps wrong.\n{desc}"
        );

        // 2025/26: Monza 14 apps (loan)
        let monza_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "monza");
        assert!(monza_2025.is_some(), "Missing Monza 2025/26 loan.\n{desc}");
        assert_eq!(
            monza_2025.unwrap().statistics.played,
            14,
            "Monza 2025/26 apps wrong.\n{desc}"
        );
        assert!(
            monza_2025.unwrap().is_loan,
            "Monza 2025/26 should be loan.\n{desc}"
        );

        // 2026/27: Inter 28 apps
        let inter_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "inter");
        assert!(inter_2026.is_some(), "Missing Inter 2026/27.\n{desc}");
        assert_eq!(
            inter_2026.unwrap().statistics.played,
            28,
            "Inter 2026/27 apps wrong.\n{desc}"
        );

        // NO phantom Monza entry in 2026/27
        let monza_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "monza");
        assert!(
            monza_2026.is_none(),
            "Phantom Monza in 2026/27 — stale seed not cleaned.\n{desc}"
        );

        assert_eq!(
            history.len(),
            3,
            "Expected 3 entries, got {}.\n{desc}",
            history.len()
        );
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
        player
            .statistics_history
            .seed_initial_team(&gzira, make_date(2025, 8, 1), false);

        // -- Season 2025/26: loaned to Birkirkara --
        player.statistics = make_stats(0, 0);
        player.on_loan(&gzira, &birkirkara, 3_000.0, make_date(2025, 8, 7));
        player.statistics = make_stats(21, 3);

        // Season end while on loan at Birkirkara
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            200,
            make_date(2026, 5, 31),
            99,
            0,
            100,
        ));
        player.on_season_end(Season::new(2025), &birkirkara, make_date(2026, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&birkirkara, &gzira, make_date(2026, 6, 1));
        player.contract_loan = None;

        // -- Season 2026/27: at Gzira, then loaned to Marsaxlokk --
        player.statistics = make_stats(1, 0);
        player.on_loan(&gzira, &marsaxlokk, 200.0, make_date(2027, 1, 20));
        player.statistics = make_stats(0, 0);

        // Season end while on loan at Marsaxlokk
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            200,
            make_date(2027, 5, 31),
            99,
            0,
            100,
        ));
        player.on_season_end(Season::new(2026), &marsaxlokk, make_date(2027, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&marsaxlokk, &gzira, make_date(2027, 6, 1));
        player.contract_loan = None;

        // -- Season 2027/28: back at Gzira, plays full season --
        player.statistics = make_stats(20, 4);
        player.on_season_end(Season::new(2027), &gzira, make_date(2028, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Gzira 0 apps for 9 days — kept as first career record
        let gzira_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "gzira");
        assert!(
            gzira_2025.is_some(),
            "First career record at Gzira should be kept even with 0 apps.\n{desc}"
        );

        // 2025/26: Birkirkara 21 apps (loan)
        let birk_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "birkirkara");
        assert!(birk_2025.is_some(), "Missing Birkirkara 2025/26.\n{desc}");
        assert_eq!(
            birk_2025.unwrap().statistics.played,
            21,
            "Birkirkara 2025/26 apps wrong.\n{desc}"
        );
        assert!(
            birk_2025.unwrap().is_loan,
            "Birkirkara should be loan.\n{desc}"
        );

        // 2026/27: Gzira 1 app + Marsaxlokk 0 apps (loan)
        let gzira_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "gzira");
        assert!(gzira_2026.is_some(), "Missing Gzira 2026/27.\n{desc}");
        assert_eq!(
            gzira_2026.unwrap().statistics.played,
            1,
            "Gzira 2026/27 apps wrong.\n{desc}"
        );

        let mars_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "marsaxlokk");
        assert!(mars_2026.is_some(), "Missing Marsaxlokk 2026/27.\n{desc}");
        assert!(
            mars_2026.unwrap().is_loan,
            "Marsaxlokk should be loan.\n{desc}"
        );

        // 2027/28: Gzira 20 apps
        let gzira_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "gzira");
        assert!(gzira_2027.is_some(), "Missing Gzira 2027/28.\n{desc}");
        assert_eq!(
            gzira_2027.unwrap().statistics.played,
            20,
            "Gzira 2027/28 apps wrong.\n{desc}"
        );

        // NO phantom Birkirkara in 2026/27 or 2027/28
        let birk_phantom = history
            .iter()
            .find(|e| e.season.start_year >= 2026 && e.team_slug == "birkirkara");
        assert!(
            birk_phantom.is_none(),
            "Phantom Birkirkara in later season.\n{desc}"
        );

        // NO phantom Marsaxlokk in 2027/28
        let mars_phantom = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "marsaxlokk");
        assert!(
            mars_phantom.is_none(),
            "Phantom Marsaxlokk in 2027/28.\n{desc}"
        );

        // 5 entries: Gzira(initial) + Birkirkara + (Gzira + Marsaxlokk) + Gzira
        assert_eq!(
            history.len(),
            5,
            "Expected 5 entries, got {}.\n{desc}",
            history.len()
        );
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
        player
            .statistics_history
            .seed_initial_team(&napoli, make_date(2025, 8, 1), false);
        player.statistics = make_stats(20, 5);
        player.on_season_end(Season::new(2025), &napoli, make_date(2026, 8, 1));

        // -- Season 2026/27: transferred to Juve, immediately loaned to Empoli --
        player.statistics = make_stats(0, 0);
        player.on_transfer(&napoli, &juve, 2_000_000.0, make_date(2026, 8, 15));
        player.on_loan(&juve, &empoli, 30_000.0, make_date(2026, 8, 20));

        // Play 18 games at Empoli
        player.statistics = make_stats(18, 4);
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            300,
            make_date(2027, 5, 31),
            99,
            0,
            100,
        ));
        player.on_season_end(Season::new(2026), &empoli, make_date(2027, 8, 1));
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&empoli, &juve, make_date(2027, 6, 1));
        player.contract_loan = None;

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Napoli 20 apps
        let napoli_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "napoli");
        assert!(napoli_2025.is_some(), "Missing Napoli 2025/26.\n{desc}");
        assert_eq!(napoli_2025.unwrap().statistics.played, 20);

        // 2026/27: Juve 0 apps (bought, never played, loaned out same week)
        let juve_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(
            juve_2026.is_some(),
            "Missing Juve 2026/27 — player was bought even if 0 apps.\n{desc}"
        );
        assert_eq!(
            juve_2026.unwrap().statistics.played,
            0,
            "Juve should have 0 apps.\n{desc}"
        );
        assert!(
            !juve_2026.unwrap().is_loan,
            "Juve entry should be permanent.\n{desc}"
        );
        assert_eq!(
            juve_2026.unwrap().transfer_fee,
            Some(2_000_000.0),
            "Juve fee wrong.\n{desc}"
        );

        // 2026/27: Empoli 18 apps (loan)
        let empoli_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "empoli");
        assert!(
            empoli_2026.is_some(),
            "Missing Empoli 2026/27 loan.\n{desc}"
        );
        assert_eq!(
            empoli_2026.unwrap().statistics.played,
            18,
            "Empoli apps wrong.\n{desc}"
        );
        assert!(
            empoli_2026.unwrap().is_loan,
            "Empoli should be loan.\n{desc}"
        );

        // No phantom Empoli in future seasons
        let empoli_phantom = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "empoli");
        assert!(
            empoli_phantom.is_none(),
            "Phantom Empoli in 2027/28.\n{desc}"
        );
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
        player
            .statistics_history
            .seed_initial_team(&gzira, make_date(2025, 8, 1), false);
        player.statistics = make_stats(0, 0);
        player.on_loan(&gzira, &mosta, 200.0, make_date(2025, 8, 10));

        // Play 18 games at Mosta
        player.statistics = make_stats(18, 5);
        player.on_loan_return(&mosta, &gzira, make_date(2026, 5, 26));
        player.contract_loan = None;

        // Back at Gzira for just 5 days, 0 games (season ends May 31)
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2025), &gzira, make_date(2026, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Mosta loan: 18 apps — must be kept
        let mosta_entry = history.iter().find(|e| e.team_slug == "mosta");
        assert!(mosta_entry.is_some(), "Missing Mosta loan entry.\n{desc}");
        assert_eq!(
            mosta_entry.unwrap().statistics.played,
            18,
            "Mosta apps wrong.\n{desc}"
        );
        assert!(
            mosta_entry.unwrap().is_loan,
            "Mosta should be loan.\n{desc}"
        );

        // Gzira 0 apps for 5 days — kept as the player's first career record
        let gzira_brief = history.iter().find(|e| {
            e.season.start_year == 2025
                && e.team_slug == "gzira"
                && e.statistics.played == 0
                && e.transfer_fee.is_none()
        });
        assert!(
            gzira_brief.is_some(),
            "First career record at Gzira should be kept even with 0 apps.\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Collapse does NOT drop entries with apps or transfer fees
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_brief_stint_with_apps_is_kept() {
        let mut player = make_player();

        let gzira = make_team("Gzira United", "gzira");
        let mosta = make_team("Mosta", "mosta");

        player
            .statistics_history
            .seed_initial_team(&gzira, make_date(2025, 8, 1), false);
        player.statistics = make_stats(0, 0);
        player.on_loan(&gzira, &mosta, 200.0, make_date(2025, 8, 10));

        player.statistics = make_stats(18, 5);
        player.on_loan_return(&mosta, &gzira, make_date(2026, 5, 26));
        player.contract_loan = None;

        // Back at Gzira for 5 days BUT played 1 game (sub appearance)
        player.statistics = make_stats(0, 0);
        player.statistics.played_subs = 1;
        player.on_season_end(Season::new(2025), &gzira, make_date(2026, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Gzira entry with 1 sub appearance — must be KEPT despite short stay
        let gzira_entry = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "gzira" && !e.is_loan);
        assert!(
            gzira_entry.is_some(),
            "Gzira with 1 sub app should be kept even for brief stint.\n{desc}"
        );
        // played_subs merged into played at drain time
        assert_eq!(
            gzira_entry.unwrap().statistics.played,
            1,
            "Gzira apps wrong (sub should be merged).\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Collapse: transfer fee protects a 0-app entry from being dropped
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_brief_stint_with_fee_is_kept() {
        let mut player = make_player();

        let napoli = make_team("Napoli", "napoli");
        let juve = make_team("Juventus", "juventus");

        player
            .statistics_history
            .seed_initial_team(&napoli, make_date(2025, 8, 1), false);
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
        let juve_entry = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(
            juve_entry.is_some(),
            "Juve with transfer fee must be kept even for 0 apps / 3 days.\n{desc}"
        );
        assert_eq!(
            juve_entry.unwrap().transfer_fee,
            Some(10_000_000.0),
            "Juve fee wrong.\n{desc}"
        );
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
        player
            .statistics_history
            .seed_initial_team(&roma, make_date(2025, 8, 1), false);
        player.statistics = make_stats(2, 0);
        player.on_loan(&roma, &torino, 30_000.0, make_date(2025, 9, 1));

        player.statistics = make_stats(15, 3);
        player.on_loan_return(&torino, &roma, make_date(2026, 3, 31));
        player.contract_loan = None;

        // Back at Roma for ~60 days (April + May), 0 games — but 20% of season
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2025), &roma, make_date(2026, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Roma 0 apps for 60 days (~20% of season) — should be KEPT
        let roma_entries: Vec<_> = history
            .iter()
            .filter(|e| e.season.start_year == 2025 && e.team_slug == "roma")
            .collect();
        assert!(
            !roma_entries.is_empty(),
            "Roma 0-app entry for 60 days (20%% of season) should be kept.\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Cross-country loan: Floriana (Malta) → Spartak (Russia)
    // Simulates: loan return in Russia, then snapshot in Malta
    // The loan entry must survive regardless of processing order.
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_cross_country_loan_free_0_games() {
        let mut player = make_player();

        let floriana = TeamInfo {
            name: "Floriana".to_string(),
            slug: "floriana".to_string(),
            reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "maltese-premier-league".to_string(),
        };
        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 500,
            league_name: "Russian Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };

        // Season start: player at Floriana
        player
            .statistics_history
            .seed_initial_team(&floriana, make_date(2026, 8, 1), false);

        // Immediate loan to Spartak on Aug 1 (free loan)
        player.statistics = make_stats(0, 0);
        player.on_loan(&floriana, &spartak, 0.0, make_date(2026, 8, 1));

        // Player sits on bench all season — 0 games at Spartak
        player.statistics = make_stats(0, 0);

        // Loan return (Russia processes first, moves player back to Floriana)
        player.on_loan_return(&spartak, &floriana, make_date(2027, 5, 31));
        player.contract_loan = None;

        // Malta snapshot runs — player is at Floriana now
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &floriana, make_date(2027, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Spartak loan entry must exist (even with 0 games)
        let spartak_entry = history.iter().find(|e| e.team_slug == "spartak-moscow");
        assert!(
            spartak_entry.is_some(),
            "Missing Spartak Moscow loan entry.\n{desc}"
        );
        assert!(
            spartak_entry.unwrap().is_loan,
            "Spartak entry should be a loan.\n{desc}"
        );

        // Floriana entry can exist (0 games, parent club)
        // The important thing is that BOTH entries are present
    }

    #[test]
    fn lifecycle_cross_country_loan_with_games() {
        let mut player = make_player();

        let floriana = TeamInfo {
            name: "Floriana".to_string(),
            slug: "floriana".to_string(),
            reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "maltese-premier-league".to_string(),
        };
        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 500,
            league_name: "Russian Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };

        player
            .statistics_history
            .seed_initial_team(&floriana, make_date(2026, 8, 1), false);

        player.statistics = make_stats(0, 0);
        player.on_loan(&floriana, &spartak, 0.0, make_date(2026, 8, 1));

        // Player plays 15 games at Spartak
        player.statistics = make_stats(15, 3);

        // Loan return
        player.on_loan_return(&spartak, &floriana, make_date(2027, 5, 31));
        player.contract_loan = None;

        // Malta snapshot
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &floriana, make_date(2027, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        let spartak_entry = history.iter().find(|e| e.team_slug == "spartak-moscow");
        assert!(
            spartak_entry.is_some(),
            "Missing Spartak Moscow loan entry.\n{desc}"
        );
        assert_eq!(
            spartak_entry.unwrap().statistics.played,
            15,
            "Spartak apps wrong.\n{desc}"
        );
        assert_eq!(
            spartak_entry.unwrap().statistics.goals,
            3,
            "Spartak goals wrong.\n{desc}"
        );
        assert!(spartak_entry.unwrap().is_loan, "Should be loan.\n{desc}");
    }

    // ---------------------------------------------------------------
    // Manual 2-season loan: both seasons must appear in history
    // Reproduces: Spartak → Floriana (1 season) then Spartak → Floriana (2 seasons)
    // User reports missing 2027/28 entry, only 2028/29 shows
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_manual_two_season_loan_both_seasons_visible() {
        let mut player = make_player();

        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 500,
            league_name: "Russian Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let floriana = TeamInfo {
            name: "Floriana".to_string(),
            slug: "floriana".to_string(),
            reputation: 100,
            league_name: "Maltese Premier League".to_string(),
            league_slug: "maltese-premier-league".to_string(),
        };

        // -- Season 2025/26: player at Spartak, plays 25 games --
        player
            .statistics_history
            .seed_initial_team(&spartak, make_date(2025, 8, 1), false);
        player.statistics = make_stats(25, 5);
        player.on_season_end(Season::new(2025), &spartak, make_date(2026, 8, 1));

        // -- Manual loan 1: Spartak → Floriana, 01.08.2026, 1 season --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&spartak, &spartak, &floriana, make_date(2026, 8, 1));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2027, 5, 31),
            99,
            0,
            100,
        ));

        // Player plays 20 games at Floriana in season 2026/27
        player.statistics = make_stats(20, 4);

        // Loan return (before season end, like real game flow)
        player.on_loan_return(&floriana, &spartak, make_date(2027, 5, 31));
        player.contract_loan = None;

        // Season end 2026/27 — player is back at Spartak (Russia processes)
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &spartak, make_date(2027, 8, 1));

        // -- Manual loan 2: Spartak → Floriana, 16.08.2027, 2 seasons --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&spartak, &spartak, &floriana, make_date(2027, 8, 16));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2029, 5, 31),
            99,
            0,
            100,
        ));

        // -- Season 2027/28: player at Floriana, 22 games --
        player.statistics = make_stats(22, 6);
        // Malta processes season end (player still on loan at Floriana)
        player.on_season_end(Season::new(2027), &floriana, make_date(2028, 8, 1));

        // -- Season 2028/29: player still at Floriana, 18 games --
        player.statistics = make_stats(18, 3);
        // Malta processes season enda
        player.on_season_end(Season::new(2028), &floriana, make_date(2029, 8, 1));

        // Loan return after season end
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&floriana, &spartak, make_date(2029, 5, 31));
        player.contract_loan = None;

        // -- Verify history --
        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Spartak 25 apps
        let spartak_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "spartak-moscow");
        assert!(spartak_2025.is_some(), "Missing Spartak 2025/26.\n{desc}");
        assert_eq!(
            spartak_2025.unwrap().statistics.played,
            25,
            "Spartak 2025/26 apps wrong.\n{desc}"
        );

        // 2026/27: Floriana 20 apps (loan 1)
        let floriana_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "floriana");
        assert!(
            floriana_2026.is_some(),
            "Missing Floriana 2026/27 (loan 1).\n{desc}"
        );
        assert_eq!(
            floriana_2026.unwrap().statistics.played,
            20,
            "Floriana 2026/27 apps wrong.\n{desc}"
        );
        assert!(
            floriana_2026.unwrap().is_loan,
            "Floriana 2026/27 should be loan.\n{desc}"
        );

        // 2027/28: Floriana 22 apps (loan 2, season 1) ← THIS IS THE ONE USER SAYS IS MISSING
        let floriana_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "floriana");
        assert!(
            floriana_2027.is_some(),
            "Missing Floriana 2027/28 (loan 2, season 1) — THIS IS THE BUG.\n{desc}"
        );
        assert_eq!(
            floriana_2027.unwrap().statistics.played,
            22,
            "Floriana 2027/28 apps wrong.\n{desc}"
        );
        assert!(
            floriana_2027.unwrap().is_loan,
            "Floriana 2027/28 should be loan.\n{desc}"
        );

        // 2028/29: Floriana 18 apps (loan 2, season 2)
        let floriana_2028 = history
            .iter()
            .find(|e| e.season.start_year == 2028 && e.team_slug == "floriana");
        assert!(
            floriana_2028.is_some(),
            "Missing Floriana 2028/29 (loan 2, season 2).\n{desc}"
        );
        assert_eq!(
            floriana_2028.unwrap().statistics.played,
            18,
            "Floriana 2028/29 apps wrong.\n{desc}"
        );
        assert!(
            floriana_2028.unwrap().is_loan,
            "Floriana 2028/29 should be loan.\n{desc}"
        );
    }

    /// Reproduces the exact scenario: when Russia's Season(2026) snapshot hasn't
    /// drained current before the user does the second manual loan, the old
    /// Floriana entry from loan 1 may get reused by loan 2.
    #[test]
    fn lifecycle_manual_two_season_loan_delayed_snapshot() {
        let mut player = make_player();

        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 500,
            league_name: "Russian Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let floriana = TeamInfo {
            name: "Floriana".to_string(),
            slug: "floriana".to_string(),
            reputation: 100,
            league_name: "Maltese Premier League".to_string(),
            league_slug: "maltese-premier-league".to_string(),
        };

        // -- Season 2025/26: player at Spartak, plays 25 games --
        player
            .statistics_history
            .seed_initial_team(&spartak, make_date(2025, 8, 1), false);
        player.statistics = make_stats(25, 5);
        player.on_season_end(Season::new(2025), &spartak, make_date(2026, 8, 1));

        // -- Manual loan 1: Spartak → Floriana, 01.08.2026, 1 season --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&spartak, &spartak, &floriana, make_date(2026, 8, 1));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2027, 5, 31),
            99,
            0,
            100,
        ));

        // Player plays 20 games at Floriana in season 2026/27
        player.statistics = make_stats(20, 4);

        // Loan return (before season end snapshot)
        player.on_loan_return(&floriana, &spartak, make_date(2027, 5, 31));
        player.contract_loan = None;

        // *** KEY DIFFERENCE: Russia's Season(2026) snapshot has NOT run yet ***
        // The user immediately does manual loan 2 on Aug 16, before Russia processes
        // its new season. current still has old entries from loan 1.

        // -- Manual loan 2: Spartak → Floriana, 16.08.2027, 2 seasons --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&spartak, &spartak, &floriana, make_date(2027, 8, 16));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2029, 5, 31),
            99,
            0,
            100,
        ));

        // NOW Russia's snapshot runs (late) for Season(2026)
        // But the player is at Floriana (Malta), so Russia doesn't process them.
        // Simulating: no on_season_end call from Russia for this player.

        // -- Season 2027/28: player at Floriana, 22 games --
        player.statistics = make_stats(22, 6);
        // Malta processes season end
        player.on_season_end(Season::new(2027), &floriana, make_date(2028, 8, 1));

        // -- Season 2028/29: player still at Floriana, 18 games --
        player.statistics = make_stats(18, 3);
        player.on_season_end(Season::new(2028), &floriana, make_date(2029, 8, 1));

        // Loan return
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&floriana, &spartak, make_date(2029, 5, 31));
        player.contract_loan = None;

        // -- Verify history --
        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2025/26: Spartak 25 apps
        let spartak_2025 = history
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "spartak-moscow");
        assert!(spartak_2025.is_some(), "Missing Spartak 2025/26.\n{desc}");

        // 2026/27: Floriana 20 apps (loan 1) — should exist as a separate season entry
        let floriana_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "floriana");
        assert!(
            floriana_2026.is_some(),
            "Missing Floriana 2026/27 (loan 1) — entries from 2026/27 not separately frozen.\n{desc}"
        );
        assert_eq!(
            floriana_2026.unwrap().statistics.played,
            20,
            "Floriana 2026/27 apps wrong.\n{desc}"
        );

        // 2027/28: Floriana 22 apps (loan 2, season 1)
        let floriana_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "floriana");
        assert!(
            floriana_2027.is_some(),
            "Missing Floriana 2027/28 (loan 2, season 1).\n{desc}"
        );
        assert_eq!(
            floriana_2027.unwrap().statistics.played,
            22,
            "Floriana 2027/28 apps wrong.\n{desc}"
        );

        // 2028/29: Floriana 18 apps (loan 2, season 2)
        let floriana_2028 = history
            .iter()
            .find(|e| e.season.start_year == 2028 && e.team_slug == "floriana");
        assert!(
            floriana_2028.is_some(),
            "Missing Floriana 2028/29 (loan 2, season 2).\n{desc}"
        );
        assert_eq!(
            floriana_2028.unwrap().statistics.played,
            18,
            "Floriana 2028/29 apps wrong.\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Multi-league country: snapshot runs multiple times for same season
    // when different leagues start new seasons on different dates
    // (e.g., Italy: Serie A starts Aug 20, Serie B starts Aug 26).
    // Must not create duplicate history entries.
    // ---------------------------------------------------------------

    #[test]
    fn multi_league_double_snapshot_no_duplicate() {
        let mut player = make_player();

        let floriana = TeamInfo {
            name: "Floriana".to_string(),
            slug: "floriana".to_string(),
            reputation: 100,
            league_name: "Maltese Premier League".to_string(),
            league_slug: "maltese-premier-league".to_string(),
        };
        let bari = TeamInfo {
            name: "Bari".to_string(),
            slug: "bari".to_string(),
            reputation: 300,
            league_name: "Serie B".to_string(),
            league_slug: "italian-serie-b".to_string(),
        };

        // -- Season 2025/26: player at Floriana --
        player
            .statistics_history
            .seed_initial_team(&floriana, make_date(2025, 8, 1), false);
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2025), &floriana, make_date(2026, 8, 1));

        // -- Manual 3-season loan: Floriana → Bari --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&floriana, &floriana, &bari, make_date(2026, 8, 15));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2029, 5, 31),
            99,
            0,
            100,
        ));

        // -- Season 2026/27: player at Bari, plays 15 games --
        player.statistics = make_stats(15, 3);
        // Italy snapshot (Serie A starts Aug 20) — first snapshot
        player.on_season_end(Season::new(2026), &bari, make_date(2027, 8, 20));

        // -- Season 2027/28: player at Bari, plays 10 games --
        player.statistics = make_stats(10, 2);

        // Italy snapshot #1: Serie A starts new season (Aug 20)
        player.on_season_end(Season::new(2027), &bari, make_date(2028, 8, 20));

        // Player plays 1 more game between Serie A and Serie B season starts
        player.statistics = make_stats(1, 0);

        // Italy snapshot #2: Serie B starts new season (Aug 26) — DUPLICATE!
        player.on_season_end(Season::new(2027), &bari, make_date(2028, 8, 26));

        // -- Verify: only ONE entry for 2027/28, with merged stats (10+1=11) --
        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        let bari_2027: Vec<_> = history
            .iter()
            .filter(|e| e.season.start_year == 2027 && e.team_slug == "bari")
            .collect();
        assert_eq!(
            bari_2027.len(),
            1,
            "Expected exactly 1 Bari entry for 2027/28, got {}.\n{desc}",
            bari_2027.len()
        );
        assert_eq!(
            bari_2027[0].statistics.played, 11,
            "Bari 2027/28 should have 11 apps (10 + 1 merged).\n{desc}"
        );
        assert!(bari_2027[0].is_loan, "Should be loan.\n{desc}");
    }

    #[test]
    fn multi_league_double_snapshot_zero_games_between() {
        let mut player = make_player();

        let bari = TeamInfo {
            name: "Bari".to_string(),
            slug: "bari".to_string(),
            reputation: 300,
            league_name: "Serie B".to_string(),
            league_slug: "italian-serie-b".to_string(),
        };

        // Seed and play a season
        player
            .statistics_history
            .seed_initial_team(&bari, make_date(2026, 8, 1), false);
        player.statistics = make_stats(20, 5);
        player.on_season_end(Season::new(2026), &bari, make_date(2027, 8, 20));

        // -- Season 2027/28: plays 12 games --
        player.statistics = make_stats(12, 3);

        // First snapshot (Serie A starts)
        player.on_season_end(Season::new(2027), &bari, make_date(2028, 8, 20));

        // Zero games between snapshots
        player.statistics = make_stats(0, 0);

        // Second snapshot (Serie B starts) — 0 remaining games
        player.on_season_end(Season::new(2027), &bari, make_date(2028, 8, 26));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        let bari_2027: Vec<_> = history
            .iter()
            .filter(|e| e.season.start_year == 2027 && e.team_slug == "bari")
            .collect();
        assert_eq!(
            bari_2027.len(),
            1,
            "Expected exactly 1 Bari entry for 2027/28, got {}.\n{desc}",
            bari_2027.len()
        );
        assert_eq!(
            bari_2027[0].statistics.played, 12,
            "Bari 2027/28 should have 12 apps (no merge needed).\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // 2-season loan: stats from first season must survive into frozen history
    // ---------------------------------------------------------------

    #[test]
    fn two_season_loan_preserves_first_season_stats() {
        let mut player = make_player();

        let parent = make_team("Sporting CP", "sporting");
        let zabbar = make_team("Zabbar St. Patrick", "zabbar");

        // -- Setup: player at Sporting CP --
        player
            .statistics_history
            .seed_initial_team(&parent, make_date(2025, 8, 1), false);
        player.statistics = make_stats(10, 2);
        player.on_season_end(Season::new(2025), &parent, make_date(2026, 8, 25));

        // -- Season 2026/27: manually loaned to Zabbar for 2 seasons --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&parent, &parent, &zabbar, make_date(2026, 9, 1));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            200,
            make_date(2028, 4, 30),
            99,
            0,
            100,
        ));

        // Player plays 20 matches at Zabbar in 2026/27
        player.statistics = make_stats(20, 3);

        // Season end 2026/27 → should freeze 20 apps
        player.on_season_end(Season::new(2026), &zabbar, make_date(2027, 8, 25));

        // Verify: frozen 2026/27 entry must have 20 games
        let zabbar_2026 = player
            .statistics_history
            .items
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "zabbar");
        assert!(
            zabbar_2026.is_some(),
            "Missing Zabbar 2026/27 entry.\n{}",
            describe_history(&player.statistics_history.items)
        );
        assert_eq!(
            zabbar_2026.unwrap().statistics.played,
            20,
            "Zabbar 2026/27 should have 20 apps.\n{}",
            describe_history(&player.statistics_history.items)
        );

        // -- Season 2027/28: continues at Zabbar, plays 15 matches --
        player.statistics = make_stats(15, 2);

        // View during season: both seasons should be visible
        let view = player
            .statistics_history
            .view_items(Some(&player.statistics), make_date(2028, 1, 15));
        let view_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "zabbar");
        assert!(view_2026.is_some(), "2026/27 Zabbar should be in view.\n");
        assert_eq!(
            view_2026.unwrap().statistics.played,
            20,
            "2026/27 Zabbar view should still show 20 apps"
        );

        let view_2027 = view
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "zabbar");
        assert!(view_2027.is_some(), "2027/28 Zabbar should be in view");
        assert_eq!(
            view_2027.unwrap().statistics.played,
            15,
            "2027/28 Zabbar view should show 15 live apps"
        );

        // Season end 2027/28
        player.on_season_end(Season::new(2027), &zabbar, make_date(2028, 8, 25));

        // Verify both seasons frozen correctly
        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        let zabbar_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "zabbar");
        assert!(zabbar_2026.is_some(), "Missing Zabbar 2026/27.\n{desc}");
        assert_eq!(
            zabbar_2026.unwrap().statistics.played,
            20,
            "Zabbar 2026/27 should have 20 apps after second season end.\n{desc}"
        );
        assert!(
            zabbar_2026.unwrap().is_loan,
            "Zabbar 2026/27 should be loan.\n{desc}"
        );

        let zabbar_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "zabbar");
        assert!(zabbar_2027.is_some(), "Missing Zabbar 2027/28.\n{desc}");
        assert_eq!(
            zabbar_2027.unwrap().statistics.played,
            15,
            "Zabbar 2027/28 should have 15 apps.\n{desc}"
        );
        assert!(
            zabbar_2027.unwrap().is_loan,
            "Zabbar 2027/28 should be loan.\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Loan return mid-season: no phantom parent entry after return
    // ---------------------------------------------------------------

    #[test]
    fn loan_return_no_phantom_parent_entry() {
        let mut player = make_player();

        let floriana = make_team("Floriana", "floriana");
        let zabbar = make_team("Zabbar St. Patrick", "zabbar");

        // -- Setup: player at Floriana --
        player
            .statistics_history
            .seed_initial_team(&floriana, make_date(2027, 8, 1), false);
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2027), &floriana, make_date(2028, 8, 25));

        // -- Season 2028/29: loaned to Zabbar --
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&floriana, &floriana, &zabbar, make_date(2028, 9, 1));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            200,
            make_date(2030, 4, 30),
            99,
            0,
            100,
        ));
        player.statistics = make_stats(23, 5);
        player.on_season_end(Season::new(2028), &zabbar, make_date(2029, 8, 25));

        // -- Season 2029/30: continues at Zabbar --
        player.statistics = make_stats(20, 3);

        // Loan expires in May → player returns mid-season
        player.on_loan_return(&zabbar, &floriana, make_date(2030, 5, 1));
        player.contract_loan = None;

        // -- Season end snapshot: player is now at Floriana --
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2029), &floriana, make_date(2030, 8, 25));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2028/29: Zabbar 23 apps (loan)
        let zabbar_2028 = history
            .iter()
            .find(|e| e.season.start_year == 2028 && e.team_slug == "zabbar");
        assert!(zabbar_2028.is_some(), "Missing Zabbar 2028/29.\n{desc}");
        assert_eq!(
            zabbar_2028.unwrap().statistics.played,
            23,
            "Zabbar 2028/29.\n{desc}"
        );

        // 2029/30: Zabbar 20 apps (loan) — from loan_return snapshot
        let zabbar_2029 = history
            .iter()
            .find(|e| e.season.start_year == 2029 && e.team_slug == "zabbar");
        assert!(zabbar_2029.is_some(), "Missing Zabbar 2029/30.\n{desc}");
        assert_eq!(
            zabbar_2029.unwrap().statistics.played,
            20,
            "Zabbar 2029/30.\n{desc}"
        );

        // NO phantom Floriana 2029/30 — player only spent a few weeks there
        let floriana_2029 = history
            .iter()
            .find(|e| e.season.start_year == 2029 && e.team_slug == "floriana");
        assert!(
            floriana_2029.is_none(),
            "Phantom Floriana 2029/30 should be dropped (0 apps, arrived late).\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Cross-country loan + later transfer: fee must survive
    // Reproduces: Dynamo Kyiv → Deportivo Tachira (loan), return,
    // then Dynamo → Kryvbas (permanent with fee).
    // The transfer fee must appear in career statistics.
    // ---------------------------------------------------------------

    #[test]
    fn lifecycle_cross_country_loan_then_transfer_fee_preserved() {
        let mut player = make_player();

        let dynamo = TeamInfo {
            name: "Dynamo Kyiv".to_string(),
            slug: "dynamo-kyiv".to_string(),
            reputation: 400,
            league_name: "Ukrainian Premier League".to_string(),
            league_slug: "ukrainian-premier-league".to_string(),
        };
        let deportivo = TeamInfo {
            name: "Deportivo Tachira".to_string(),
            slug: "deportivo-tachira".to_string(),
            reputation: 200,
            league_name: "Primera Division".to_string(),
            league_slug: "venezuelan-primera".to_string(),
        };
        let kryvbas = TeamInfo {
            name: "Kryvbas".to_string(),
            slug: "kryvbas".to_string(),
            reputation: 250,
            league_name: "Ukrainian Premier League".to_string(),
            league_slug: "ukrainian-premier-league".to_string(),
        };

        // -- Season 2025/26: player at Dynamo --
        player
            .statistics_history
            .seed_initial_team(&dynamo, make_date(2025, 8, 1), false);
        player.statistics = make_stats(10, 2);
        player.on_season_end(Season::new(2025), &dynamo, make_date(2026, 8, 1));

        // -- Season 2026/27: plays 1 game at Dynamo, then loaned to Deportivo --
        player.statistics = make_stats(1, 0);
        player.on_loan(&dynamo, &deportivo, 52_000.0, make_date(2026, 8, 6));

        // Player plays 0 games at Deportivo
        player.statistics = make_stats(0, 0);

        // Venezuela snapshot (new season in e.g. Feb 2027) — player still at Deportivo
        // ended_season = 2025/26 (Season::from_date(Feb 2027) = 2026/27 → ended = 2025/26)
        // Wait, this should be for 2026/27 if called later. Let's simulate both scenarios.
        // First: normal snapshot for 2026/27
        player.on_season_end(Season::new(2026), &deportivo, make_date(2027, 2, 1));

        // Loan return (May 2027)
        player.on_loan_return(&deportivo, &dynamo, make_date(2027, 5, 31));
        player.contract_loan = None;

        // -- Season 2027/28: player back at Dynamo --
        // Player plays 0 games at Dynamo, then transfers to Kryvbas
        player.statistics = make_stats(0, 0);
        player.on_transfer(&dynamo, &kryvbas, 610_000.0, make_date(2028, 6, 21));

        // Player plays 20 games at Kryvbas
        player.statistics = make_stats(20, 1);
        player.on_season_end(Season::new(2027), &kryvbas, make_date(2028, 8, 1));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // 2027/28 Kryvbas: must have the 610K fee
        let kryvbas_2027 = history
            .iter()
            .find(|e| e.season.start_year == 2027 && e.team_slug == "kryvbas");
        assert!(
            kryvbas_2027.is_some(),
            "Missing Kryvbas 2027/28 entry.\n{desc}"
        );
        assert_eq!(
            kryvbas_2027.unwrap().transfer_fee,
            Some(610_000.0),
            "Kryvbas 2027/28 transfer fee must be 610K.\n{desc}"
        );
        assert_eq!(
            kryvbas_2027.unwrap().statistics.played,
            20,
            "Kryvbas 2027/28 apps.\n{desc}"
        );

        // 2026/27 Deportivo: should show as loan
        let deportivo_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "deportivo-tachira");
        assert!(
            deportivo_2026.is_some(),
            "Missing Deportivo 2026/27 entry.\n{desc}"
        );
        assert!(
            deportivo_2026.unwrap().is_loan,
            "Deportivo should be loan.\n{desc}"
        );

        // 2026/27 Dynamo: should have 1 app
        let dynamo_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "dynamo-kyiv");
        assert!(
            dynamo_2026.is_some(),
            "Missing Dynamo 2026/27 entry.\n{desc}"
        );
        assert_eq!(
            dynamo_2026.unwrap().statistics.played,
            1,
            "Dynamo 2026/27 apps.\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // Duplicate season guard with mid-season transfer: fee must survive
    // Simulates the guard firing when the season was already frozen,
    // but current has a transfer entry with a fee.
    // ---------------------------------------------------------------

    #[test]
    fn duplicate_season_guard_preserves_transfer_fee() {
        let mut player = make_player();

        let roma = make_team("Roma", "roma");
        let juve = make_team("Juventus", "juventus");

        // -- Season 2025/26: at Roma --
        player
            .statistics_history
            .seed_initial_team(&roma, make_date(2025, 8, 1), false);
        player.statistics = make_stats(20, 5);
        player.on_season_end(Season::new(2025), &roma, make_date(2026, 8, 1));

        // -- Season 2026/27: transfer to Juve with fee --
        player.statistics = make_stats(3, 1);
        player.on_transfer(&roma, &juve, 8_000_000.0, make_date(2027, 1, 15));
        player.statistics = make_stats(10, 2);

        // First snapshot (Serie A): freezes 2026/27
        player.on_season_end(Season::new(2026), &juve, make_date(2027, 8, 20));

        // Transfer to another club AFTER first snapshot but before second
        let napoli = make_team("Napoli", "napoli");
        player.statistics = make_stats(0, 0);
        player.on_transfer(&juve, &napoli, 12_000_000.0, make_date(2027, 8, 22));

        // Second snapshot (Serie B): triggers duplicate guard
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &napoli, make_date(2027, 8, 26));

        let history = &player.statistics_history.items;
        let desc = describe_history(history);

        // Juve 2026/27: should have the 8M fee (frozen in first snapshot)
        let juve_2026 = history
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "juventus");
        assert!(juve_2026.is_some(), "Missing Juve 2026/27.\n{desc}");
        assert_eq!(
            juve_2026.unwrap().transfer_fee,
            Some(8_000_000.0),
            "Juve 2026/27 fee wrong.\n{desc}"
        );

        // Napoli: should have the 12M fee (was in current when guard fired)
        let napoli_entry = history
            .iter()
            .find(|e| e.team_slug == "napoli" && e.transfer_fee == Some(12_000_000.0));
        assert!(
            napoli_entry.is_some(),
            "Napoli entry with 12M fee must survive the duplicate season guard.\n{desc}"
        );
    }

    // ---------------------------------------------------------------
    // U21 → Main promotion preserves senior-callup stats end-to-end
    // ---------------------------------------------------------------

    #[test]
    fn u21_promoted_midseason_keeps_callup_games_in_career_history() {
        // Real-game scenario: a U21 player gets called up for five
        // Main-team matches early in the season (those games land in
        // `player.statistics` because the Main league is non-friendly).
        // Squad rebalance later promotes them to Main. Before the
        // `on_intra_club_move` fix, the mid-season promotion drained
        // `player.statistics` and discarded the games entirely — at
        // season end the Main row showed 0 apps even though the player
        // really did play five times.
        let mut player = make_player();

        let main = make_team("Spartak Moscow", "spartak");
        let u21 = make_team("Spartak U21", "spartak-u21");

        // Player seeded under the Main alias because they started on U21.
        player
            .statistics_history
            .seed_initial_team(&main, make_date(2025, 8, 1), false);

        // Five senior callup games while rostered on U21.
        player.statistics = make_stats(5, 2);

        // Mid-season promotion U21 → Main.
        player.on_intra_club_move(&u21, &main, false, true, make_date(2025, 11, 15));

        // The callup games must NOT have been wiped from
        // player.statistics — the next senior season-end is what
        // turns them into a frozen Main row.
        assert_eq!(
            player.statistics.played, 5,
            "U21 → Main promotion must not drain senior callup stats"
        );
        assert_eq!(player.statistics.goals, 2);

        // Player plays another 12 Main-team games before season ends.
        player.statistics.played += 12;
        player.statistics.goals += 4;

        player.on_season_end(Season::new(2025), &main, make_date(2026, 8, 1));

        let main_row = player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2025 && i.team_slug == "spartak")
            .expect("Main row must exist after season end");
        assert_eq!(
            main_row.statistics.played, 17,
            "Main row must carry both pre-promotion callups and post-promotion games"
        );
        assert_eq!(main_row.statistics.goals, 6);
    }

    #[test]
    fn lateral_youth_move_keeps_callup_games_for_next_snapshot() {
        // U18 → U19 lateral move shouldn't touch career history
        // (both teams alias to Main), AND shouldn't drain the
        // player's accumulated senior callup stats. The next
        // non-senior season-end is responsible for routing them to
        // the Main alias row.
        let mut player = make_player();
        let main = make_team("Spartak Moscow", "spartak");
        let u18 = make_team("Spartak U18", "spartak-u18");
        let u19 = make_team("Spartak U19", "spartak-u19");

        player
            .statistics_history
            .seed_initial_team(&main, make_date(2025, 8, 1), false);

        // 3 senior callup games accrued while on U18.
        player.statistics = make_stats(3, 1);

        // Mid-season birthday → squad rebalance moves player U18 → U19.
        player.on_intra_club_move(&u18, &u19, false, false, make_date(2026, 1, 10));

        // Stats stay on player.statistics — neither side of the
        // intra-club move wrote anything to history, and we did not
        // discard the games.
        assert_eq!(
            player.statistics.played, 3,
            "lateral youth move must not drain stats"
        );

        // No youth slugs in current — only the Main alias.
        let non_main: Vec<&str> = player
            .statistics_history
            .current
            .iter()
            .map(|e| e.team_slug.as_str())
            .filter(|s| *s != "spartak")
            .collect();
        assert!(
            non_main.is_empty(),
            "no youth slug must leak into current history (got: {:?})",
            non_main
        );
    }

    // ---------------------------------------------------------------
    // U21 player with DB-loaded prior history, seeded mid-season,
    // plays 0 senior callups all season — the season row must survive
    // the trivial-stint filter so the player's history always shows
    // at least one row per season they existed at the club.
    // (Bug repro: a U21 player loaded with prior items, seeded on a
    // late-season date, was losing the row for the just-ended season
    // because joined_date pushed time_pct under the 45% threshold.)
    // ---------------------------------------------------------------

    #[test]
    fn u21_player_with_db_history_and_late_seed_keeps_season_row() {
        use crate::club::player::statistics::PlayerStatistics;
        use crate::club::player::statistics::history::PlayerStatisticsHistoryItem;

        let mut player = make_player();

        // Simulate DB-loaded prior history: 2 senior seasons at "spartak"
        // before the simulator started.
        let prior_2023 = PlayerStatisticsHistoryItem {
            season: Season::new(2023),
            team_name: "Spartak".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: "Russian Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: make_stats(18, 2),
            seq_id: 0,
        };
        let prior_2024 = PlayerStatisticsHistoryItem {
            season: Season::new(2024),
            team_name: "Spartak".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: "Russian Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: make_stats(22, 3),
            seq_id: 1,
        };
        player.statistics_history =
            crate::PlayerStatisticsHistory::from_items(vec![prior_2023, prior_2024]);

        let main = make_team("Spartak", "spartak");

        // Simulator starts in the middle of the 2025/26 season — seed runs
        // with the live game date, NOT the season start.
        player
            .statistics_history
            .seed_initial_team(&main, make_date(2026, 4, 1), false);

        // Player spends the remainder of 2025/26 rostered on U21 with no
        // senior callups. `record_season_end` is invoked from the youth
        // alias path so `team` is the Main team and stats are zero.
        player.statistics = PlayerStatistics::default();
        player.on_season_end(Season::new(2025), &main, make_date(2026, 8, 1));

        // The 2025/26 row must exist even though the player played 0
        // senior games and was seeded only ~60 days before season end.
        let row_2025 = player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2025 && i.team_slug == "spartak");
        assert!(
            row_2025.is_some(),
            "2025/26 Main alias row missing — every season the player \
             existed at the club must show at least one row, even with \
             0 senior callups and a mid-season seed date. Items: {:?}",
            player
                .statistics_history
                .items
                .iter()
                .map(|i| format!("{}:{}", i.season.start_year, i.team_slug))
                .collect::<Vec<_>>()
        );
        assert_eq!(row_2025.unwrap().statistics.played, 0);

        // Next season also runs with 0 senior callups — still must keep
        // a row, since the seeded entry now has a season-start joined
        // date and the merge function design preserves the lone Main
        // row in a quiet season.
        player.statistics = PlayerStatistics::default();
        player.on_season_end(Season::new(2026), &main, make_date(2027, 8, 1));

        let row_2026 = player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2026 && i.team_slug == "spartak");
        assert!(
            row_2026.is_some(),
            "2026/27 row missing after a second quiet season."
        );
    }

    // ---------------------------------------------------------------
    // RENDER-LEVEL repro of the user report: Spartak → Zenit loan that
    // spans into the next season. The rendered history (player_history_rows)
    // shows the active Spartak season and the first loan season, but the
    // MIDDLE season vanishes entirely — no row of any kind. Every season
    // the player existed at a club must surface a row.
    // ---------------------------------------------------------------
    #[test]
    fn loan_spanning_two_seasons_keeps_middle_season_row() {
        use crate::club::player::statistics::projection::PlayerStatisticsProjection;

        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let zenit = TeamInfo {
            name: "Zenit".to_string(),
            slug: "zenit".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };

        let mut player = make_player();
        player
            .statistics_history
            .seed_initial_team(&spartak, make_date(2025, 8, 1), false);

        // Loan to Zenit early in 2025/26 (0 Spartak apps before the move).
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&spartak, &spartak, &zenit, make_date(2025, 8, 10));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            500,
            make_date(2027, 5, 31),
            99,
            0,
            100,
        ));

        // 18 apps at Zenit in 2025/26; season ends while still on loan.
        player.statistics = make_stats(18, 7);
        player.on_season_end(Season::new(2025), &zenit, make_date(2026, 8, 1));

        // 2026/27: still on loan at Zenit, 0 further apps; season ends on loan.
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &zenit, make_date(2027, 8, 1));

        // Loan returns to Spartak for 2027/28.
        player.statistics = make_stats(0, 0);
        player.on_loan_return(&zenit, &spartak, make_date(2027, 6, 1));
        player.contract_loan = None;

        // 2027/28 in progress: the active Spartak spell (1 sub app live).
        let mut live = make_stats(0, 0);
        live.played_subs = 1;
        let empty = PlayerStatistics::default();
        let live_input = crate::PlayerLiveStatsInput {
            league: &live,
            friendly: &empty,
            cups: &[],
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_history_rows(
            &player.statistics_history,
            &live_input,
            make_date(2027, 10, 1),
        );

        let years: Vec<u16> = rows.iter().map(|r| r.season.start_year).collect();
        let desc: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "{}:{}{}",
                    r.season.start_year,
                    r.team_slug,
                    if r.is_loan { "(loan)" } else { "" }
                )
            })
            .collect();
        assert!(
            years.contains(&2026),
            "2026/27 must surface at least one row — got {:?}",
            desc
        );
        // The 2026/27 row is the truthful Zenit loan spell (the player
        // was on loan there all season, 0 apps), not a fabricated
        // parent-club placeholder.
        let middle = rows
            .iter()
            .find(|r| r.season.start_year == 2026)
            .expect("2026/27 row");
        assert_eq!(middle.team_slug, "zenit", "got rows {:?}", desc);
        assert!(middle.is_loan, "2026/27 must be the Zenit loan, got {:?}", desc);
        assert_eq!(middle.statistics.played, 0);
        // And every season in the career span is now present.
        assert!(years.contains(&2025) && years.contains(&2027), "got {:?}", desc);
    }

    #[test]
    fn active_loan_with_games_visible_before_season_end_snapshot() {
        // Regression for the "where's the Rostov row on 31 Jul?" report:
        // an in-progress loan that has produced games must render on the
        // last day of the season (before the season-end snapshot fires on
        // 1 Aug), whether the loan is still active or has just returned.
        // `Season::from_date(31 Jul 2028)` is 2027, so the row sits under
        // 2027/28 in both cases.
        use crate::club::player::statistics::projection::PlayerStatisticsProjection;
        let spartak = TeamInfo { name: "Spartak Moscow".to_string(), slug: "spartak".to_string(), reputation: 5000, league_name: "Premier League".to_string(), league_slug: "russian-premier-league".to_string() };
        let rostov = TeamInfo { name: "Rostov".to_string(), slug: "rostov".to_string(), reputation: 5000, league_name: "Premier League".to_string(), league_slug: "russian-premier-league".to_string() };

        for returned in [false, true] {
            let mut player = make_player();
            player
                .statistics_history
                .seed_initial_team(&spartak, make_date(2027, 8, 1), false);
            player.statistics = make_stats(0, 0);
            player.on_manual_loan(&spartak, &spartak, &rostov, make_date(2027, 9, 9));
            player.contract_loan = Some(crate::PlayerClubContract::new_loan(
                500,
                make_date(2028, 5, 31),
                99,
                0,
                100,
            ));
            player.statistics = make_stats(19, 3);
            if returned {
                player.on_loan_return(&rostov, &spartak, make_date(2028, 6, 1));
                player.contract_loan = None;
            }

            let live = player.statistics.clone();
            let empty = PlayerStatistics::default();
            let live_input = crate::PlayerLiveStatsInput {
                league: &live,
                friendly: &empty,
                cups: &[],
                friendly_source_slug: "",
            };
            let rows = PlayerStatisticsProjection::player_history_rows(
                &player.statistics_history,
                &live_input,
                make_date(2028, 7, 31),
            );
            let rostov_row = rows
                .iter()
                .find(|r| r.team_slug == "rostov")
                .unwrap_or_else(|| panic!("Rostov loan must be visible on 31 Jul (returned={returned})"));
            assert_eq!(rostov_row.season.start_year, 2027);
            assert!(rostov_row.is_loan);
            assert_eq!(rostov_row.statistics.played, 19);
        }
    }

    #[test]
    fn zero_game_loan_row_survives_into_next_season() {
        // User rule: a loan the player went on but never featured in must
        // still show after the season freezes — it doesn't vanish just
        // because apps are 0. Loan to Zenit in 2026/27 with 0 games, then
        // 2027/28 at Spartak: the 2026/27 Zenit loan row must remain.
        use crate::club::player::statistics::projection::PlayerStatisticsProjection;
        let spartak = TeamInfo { name: "Spartak Moscow".to_string(), slug: "spartak".to_string(), reputation: 5000, league_name: "Premier League".to_string(), league_slug: "rpl".to_string() };
        let zenit = TeamInfo { name: "Zenit".to_string(), slug: "zenit".to_string(), reputation: 5000, league_name: "Premier League".to_string(), league_slug: "rpl".to_string() };

        let mut player = make_player();
        player.statistics_history.seed_initial_team(&spartak, make_date(2026, 8, 1), false);
        player.statistics = make_stats(0, 0);
        player.on_manual_loan(&spartak, &spartak, &zenit, make_date(2026, 9, 1));
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(500, make_date(2027, 5, 31), 99, 0, 100));
        // 0 games at Zenit; season ends while on loan.
        player.statistics = make_stats(0, 0);
        player.on_season_end(Season::new(2026), &zenit, make_date(2027, 8, 1));
        // Back at Spartak for 2027/28.
        player.on_loan_return(&zenit, &spartak, make_date(2027, 8, 2));
        player.contract_loan = None;

        let empty = PlayerStatistics::default();
        let live_input = crate::PlayerLiveStatsInput { league: &empty, friendly: &empty, cups: &[], friendly_source_slug: "" };
        let rows = PlayerStatisticsProjection::player_history_rows(&player.statistics_history, &live_input, make_date(2027, 10, 1));
        let desc: Vec<String> = rows.iter().map(|r| format!("{}:{}{}", r.season.start_year, r.team_slug, if r.is_loan {"(loan)"} else {""})).collect();
        assert!(
            rows.iter().any(|r| r.season.start_year == 2026 && r.team_slug == "zenit" && r.is_loan),
            "0-game Zenit loan must remain visible after the season ends; got {:?}", desc
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Spec-mandated drain contract:
//
//   Every lifecycle boundary must freeze the live Friendly + cup buckets
//   into the canonical ledger BEFORE clearing them. The raw ledger
//   records WHERE matches were earned (team + competition_slug); the
//   projection decides HOW to display / fold / filter.
//
// These tests pin down the per-handler drain contract end-to-end so a
// regression in any one handler surfaces as a focused failure rather
// than a vague rendering bug down the line.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod drain_invariants_tests {
    use super::*;
    use crate::CompetitionStatistics;
    use crate::LiveCupSlice;
    use crate::PlayerLiveStatsInput;
    use crate::PlayerStatCompetitionKind;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::projection::PlayerStatisticsProjection;
    use crate::continent::competitions::CHAMPIONS_LEAGUE_SLUG;
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills, PlayerStatistics};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn stats(played: u16, goals: u16) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.goals = goals;
        s
    }

    fn player() -> crate::Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(d(2000, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn team(name: &str, slug: &str, league_slug: &str) -> TeamInfo {
        TeamInfo {
            name: name.to_string(),
            slug: slug.to_string(),
            reputation: 100,
            league_name: "League".to_string(),
            league_slug: league_slug.to_string(),
        }
    }

    fn has_ledger_entry(
        player: &crate::Player,
        team_slug: &str,
        kind: PlayerStatCompetitionKind,
        competition_slug: &str,
        played: u16,
    ) -> bool {
        player
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| {
                e.team_slug == team_slug
                    && e.competition_kind == kind
                    && e.competition_slug == competition_slug
                    && e.statistics.played == played
            })
    }

    // ── Per-handler drain contract ────────────────────────────────────

    #[test]
    fn on_manual_transfer_freezes_source_friendly_and_cup_under_source_team() {
        let mut p = player();
        let from = team("Juventus", "juventus", "serie-a");
        let to = team("Lazio", "lazio", "serie-a");

        p.statistics = stats(8, 2);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: CHAMPIONS_LEAGUE_SLUG.to_string(),
            statistics: stats(3, 1),
        });
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "coppa-italia".to_string(),
            statistics: stats(1, 0),
        });

        p.on_manual_transfer(&from, &to, Some(5_000_000.0), d(2026, 11, 1));

        // Live buckets cleared.
        assert_eq!(p.statistics.played, 0);
        assert_eq!(p.friendly_statistics.played, 0);
        assert!(p.cup_statistics_by_competition.is_empty());
        // Source spell's friendly + per-cup entries frozen under SOURCE team.
        assert!(has_ledger_entry(&p, "juventus", PlayerStatCompetitionKind::Friendly, "serie-a", 2));
        assert!(has_ledger_entry(
            &p,
            "juventus",
            PlayerStatCompetitionKind::ContinentalCup,
            CHAMPIONS_LEAGUE_SLUG,
            3,
        ));
        assert!(has_ledger_entry(&p, "juventus", PlayerStatCompetitionKind::DomesticCup, "coppa-italia", 1));
        // Nothing under destination.
        assert!(!p
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| e.team_slug == "lazio"
                && e.competition_kind != PlayerStatCompetitionKind::League));
    }

    #[test]
    fn on_manual_loan_freezes_source_friendly_and_cup_under_source_team() {
        let mut p = player();
        let from = team("Juventus", "juventus", "serie-a");
        let to = team("Empoli", "empoli", "serie-a");

        p.statistics = stats(6, 1);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "coppa-italia".to_string(),
            statistics: stats(2, 0),
        });

        p.on_manual_loan(&from, &from, &to, d(2026, 11, 5));

        assert!(p.cup_statistics_by_competition.is_empty());
        assert!(has_ledger_entry(&p, "juventus", PlayerStatCompetitionKind::Friendly, "serie-a", 2));
        assert!(has_ledger_entry(&p, "juventus", PlayerStatCompetitionKind::DomesticCup, "coppa-italia", 2));
        assert!(!p
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| e.team_slug == "empoli"
                && e.competition_kind != PlayerStatCompetitionKind::League));
    }

    #[test]
    fn on_cancel_loan_freezes_borrowing_friendly_and_cup_under_borrowing_team() {
        let mut p = player();
        let parent = team("Spartak", "spartak", "rpl");
        let borrowing = team("Pari", "pari", "rpl");

        // Player has the borrowing-club live buckets populated.
        p.statistics = stats(9, 0);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "russia-cup".to_string(),
            statistics: stats(1, 0),
        });

        // Existing borrowing-club current entry so the League snapshot lands somewhere.
        p.statistics_history
            .current
            .push(crate::club::player::statistics::history::CurrentSeasonEntry {
                team_name: borrowing.name.clone(),
                team_slug: borrowing.slug.clone(),
                team_reputation: borrowing.reputation,
                league_name: borrowing.league_name.clone(),
                league_slug: borrowing.league_slug.clone(),
                is_loan: true,
                transfer_fee: Some(0.0),
                statistics: PlayerStatistics::default(),
                joined_date: d(2026, 8, 1),
                departed_date: None,
                seq_id: 1,
            });

        p.on_cancel_loan(&borrowing, &parent, d(2026, 12, 1));

        assert_eq!(p.friendly_statistics.played, 0);
        assert!(p.cup_statistics_by_competition.is_empty());
        assert!(has_ledger_entry(&p, "pari", PlayerStatCompetitionKind::Friendly, "rpl", 2));
        assert!(has_ledger_entry(&p, "pari", PlayerStatCompetitionKind::DomesticCup, "russia-cup", 1));
        // No parent-club non-League leakage.
        assert!(!p
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| e.team_slug == "spartak"
                && e.competition_kind != PlayerStatCompetitionKind::League));
    }

    #[test]
    fn on_release_freezes_source_friendly_and_cup_under_source_team() {
        let mut p = player();
        let from = team("Marseille", "marseille", "ligue-1");

        p.statistics = stats(4, 1);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: crate::continent::competitions::EUROPA_LEAGUE_SLUG.to_string(),
            statistics: stats(3, 0),
        });

        p.on_release(&from, d(2026, 12, 30));

        assert!(p.cup_statistics_by_competition.is_empty());
        assert!(has_ledger_entry(&p, "marseille", PlayerStatCompetitionKind::Friendly, "ligue-1", 2));
        assert!(has_ledger_entry(
            &p,
            "marseille",
            PlayerStatCompetitionKind::ContinentalCup,
            crate::continent::competitions::EUROPA_LEAGUE_SLUG,
            3,
        ));
    }

    #[test]
    fn on_season_end_freezes_friendly_under_team_league_slug() {
        // For a senior season-end, source_slug defaults to the team's
        // league_slug, so the breakdown labels Friendly with the senior
        // league name (the web layer then renders the generic "Friendly").
        let mut p = player();
        let main = team("Inter", "inter", "serie-a");
        p.statistics = stats(30, 8);
        p.friendly_statistics = stats(4, 1);
        p.on_season_end(Season::new(2026), &main, d(2027, 8, 1));

        assert!(has_ledger_entry(&p, "inter", PlayerStatCompetitionKind::Friendly, "serie-a", 4));
        assert_eq!(p.friendly_statistics.played, 0);
    }

    #[test]
    fn on_non_senior_season_end_freezes_friendly_under_youth_league_slug() {
        // The drain stamps the Friendly competition_slug with the YOUTH
        // team's league_slug, while the row anchor stays under the Main
        // alias. The History breakdown can then label the Friendly row
        // "Russian Premier League U19" — not the senior parent.
        let mut p = player();
        let main = team("Krasnodar", "krasnodar", "russian-premier-league");
        let youth = team(
            "Krasnodar U19",
            "krasnodar-u19",
            "russian-premier-league-u19",
        );
        p.statistics_history
            .seed_initial_team(&main, d(2026, 8, 1), false);
        p.statistics = PlayerStatistics::default();
        p.friendly_statistics = stats(5, 2);
        p.on_non_senior_season_end(Season::new(2026), &main, &youth, d(2027, 8, 1));

        // Friendly is anchored under MAIN team_slug but stamped with the
        // YOUTH league_slug.
        let frozen = p
            .statistics_history
            .season_ledger
            .iter()
            .find(|e| {
                e.team_slug == "krasnodar"
                    && e.competition_kind == PlayerStatCompetitionKind::Friendly
            })
            .expect("youth-aliased Friendly entry missing");
        assert_eq!(frozen.competition_slug, "russian-premier-league-u19");
        assert_eq!(frozen.statistics.played, 5);
    }

    // ── End-to-end: lifecycle scenarios from the spec ─────────────────

    /// Spec scenario: loan out, play League + Friendly + Cup, then
    /// cancel-loan mid-season. The History breakdown must still show
    /// League + Friendly + Cup under the loan-team row.
    #[test]
    fn lifecycle_loan_play_all_buckets_then_cancel_keeps_breakdown_under_loan_team() {
        let mut p = player();
        let parent = team("Spartak", "spartak", "russian-premier-league");
        let pari = team("Pari", "pari", "russian-premier-league");

        // Initial state.
        p.statistics_history
            .seed_initial_team(&parent, d(2026, 8, 1), false);

        // Loan to Pari.
        p.statistics = PlayerStatistics::default();
        p.on_loan(&parent, &pari, 50_000.0, d(2026, 9, 1));

        // While at Pari: 9 League games, 2 Friendly games (youth bucket),
        // 1 Russia Cup game.
        p.statistics = stats(9, 0);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "russia-cup".to_string(),
            statistics: stats(1, 0),
        });

        // Mid-season cancel.
        p.on_cancel_loan(&pari, &parent, d(2026, 12, 1));

        // Project the breakdowns (loan row at Pari).
        let empty = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &empty,
            friendly: &empty,
            cups: &[],
            friendly_source_slug: "",
        };
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2026, 12, 15),
        );
        let pari = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "pari")
            .expect("Pari breakdown must exist after cancel-loan");
        assert!(pari.is_loan, "Pari row should be labelled loan");
        let kinds: Vec<PlayerStatCompetitionKind> =
            pari.competitions.iter().map(|c| c.competition_kind).collect();
        assert!(kinds.contains(&PlayerStatCompetitionKind::League));
        assert!(kinds.contains(&PlayerStatCompetitionKind::Friendly));
        assert!(kinds.contains(&PlayerStatCompetitionKind::DomesticCup));
    }

    /// Spec scenario: transfer after playing Friendly + Cup. The source
    /// team's row keeps those stats; the destination starts clean.
    #[test]
    fn lifecycle_transfer_with_cup_and_friendly_keeps_source_breakdown() {
        let mut p = player();
        let a = team("Club A", "club-a", "premier-league");
        let b = team("Club B", "club-b", "premier-league");

        p.statistics_history
            .seed_initial_team(&a, d(2026, 8, 1), false);
        // At A: 8 League apps, 3 Friendly, 2 UCL, 1 FA-Cup.
        p.statistics = stats(8, 1);
        p.friendly_statistics = stats(3, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: CHAMPIONS_LEAGUE_SLUG.to_string(),
            statistics: stats(2, 0),
        });
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "fa-cup".to_string(),
            statistics: stats(1, 0),
        });

        p.on_transfer(&a, &b, 1_000_000.0, d(2026, 11, 1));

        // Destination starts clean.
        assert_eq!(p.statistics.played, 0);
        assert_eq!(p.friendly_statistics.played, 0);
        assert!(p.cup_statistics_by_competition.is_empty());

        // Project: A breakdown shows League + Friendly + Continental + Domestic.
        let empty = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &empty,
            friendly: &empty,
            cups: &[],
            friendly_source_slug: "",
        };
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2026, 12, 1),
        );
        let a_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "club-a")
            .expect("source-club A breakdown missing after transfer");
        let kinds: Vec<PlayerStatCompetitionKind> =
            a_bd.competitions.iter().map(|c| c.competition_kind).collect();
        assert!(kinds.contains(&PlayerStatCompetitionKind::League));
        assert!(kinds.contains(&PlayerStatCompetitionKind::Friendly));
        assert!(kinds.contains(&PlayerStatCompetitionKind::ContinentalCup));
        assert!(kinds.contains(&PlayerStatCompetitionKind::DomesticCup));

        // Destination row exists but only carries a 0-app League stub.
        let b_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "club-b")
            .expect("destination B breakdown must exist");
        assert!(
            !b_bd
                .competitions
                .iter()
                .any(|c| c.competition_kind != PlayerStatCompetitionKind::League),
            "destination must not inherit any source non-League slice"
        );
    }

    /// Spec scenario: loan out, play Friendly + Cup, RETURN from loan.
    /// The departed loan row keeps those stats. (Variant of the existing
    /// `on_loan_return_freezes_borrowing_club_friendly_and_cup_stats`
    /// test that additionally checks the breakdown projection.)
    #[test]
    fn lifecycle_loan_play_then_return_keeps_breakdown_under_loan_team() {
        let mut p = player();
        let parent = team("Juventus", "juventus", "serie-a");
        let torino = team("Torino", "torino", "serie-a");

        p.statistics_history
            .seed_initial_team(&parent, d(2026, 8, 1), false);
        p.on_loan(&parent, &torino, 30_000.0, d(2027, 1, 15));

        // Loan-period stats.
        p.statistics = stats(12, 2);
        p.friendly_statistics = stats(1, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "coppa-italia".to_string(),
            statistics: stats(2, 1),
        });

        p.on_loan_return(&torino, &parent, d(2027, 5, 31));

        let empty = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &empty,
            friendly: &empty,
            cups: &[],
            friendly_source_slug: "",
        };
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2027, 6, 5),
        );
        let torino = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "torino")
            .expect("Torino loan breakdown missing after return");
        assert!(torino.is_loan);
        let kinds: Vec<PlayerStatCompetitionKind> = torino
            .competitions
            .iter()
            .map(|c| c.competition_kind)
            .collect();
        assert!(kinds.contains(&PlayerStatCompetitionKind::League));
        assert!(kinds.contains(&PlayerStatCompetitionKind::Friendly));
        assert!(kinds.contains(&PlayerStatCompetitionKind::DomesticCup));
        // And no Torino non-League stats leaked to parent.
        assert!(!p
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| e.team_slug == "juventus"
                && e.competition_kind != PlayerStatCompetitionKind::League));
    }

    /// Spec edge case: same player, same season, same team, same league
    /// but different loan state — Friendly/cup stats must NOT orphan.
    /// (Loan→cancel→parent within one season; breakdown grouping ignores
    /// is_loan, so all non-League slices for a given (year, team, league)
    /// land under the row with the latest League seq's loan flag.)
    #[test]
    fn lifecycle_loan_then_cancel_same_team_same_league_no_orphan() {
        let mut p = player();
        // Pari has loaned the player in from his parent club; for this
        // edge case the parent and loan team happen to share a league
        // slug. The drain still attributes cups/friendlies to the team
        // they were earned at via `team_slug`.
        let parent = team("Spartak", "spartak", "rpl");
        let pari = team("Pari", "pari", "rpl");

        p.statistics_history
            .seed_initial_team(&parent, d(2026, 8, 1), false);
        p.on_loan(&parent, &pari, 0.0, d(2026, 9, 1));

        // Loan-period: League + Friendly + DomesticCup.
        p.statistics = stats(9, 0);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "russia-cup".to_string(),
            statistics: stats(1, 0),
        });

        p.on_cancel_loan(&pari, &parent, d(2026, 12, 1));

        // Pari row carries the cup + friendly under team_slug=pari.
        assert!(has_ledger_entry(&p, "pari", PlayerStatCompetitionKind::Friendly, "rpl", 2));
        assert!(has_ledger_entry(&p, "pari", PlayerStatCompetitionKind::DomesticCup, "russia-cup", 1));

        // Render the breakdowns: pari row's is_loan flag is true (League
        // entry says so) and the cup/friendly entries — written with
        // is_loan=false on the ledger — still group under it because
        // grouping ignores is_loan.
        let empty = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &empty,
            friendly: &empty,
            cups: &[],
            friendly_source_slug: "",
        };
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2026, 12, 15),
        );
        let pari = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "pari")
            .expect("Pari breakdown missing");
        assert!(pari.is_loan);
        // No orphan non-loan Pari breakdown holding the cup/friendly.
        assert_eq!(
            breakdowns
                .iter()
                .filter(|b| b.season_start_year == 2026 && b.team_slug == "pari")
                .count(),
            1,
        );
    }

    /// Spec edge case: chained intra-club move + loan. Intra-club move
    /// deliberately does NOT drain friendly/cup (soft same-club move),
    /// but the next inter-club boundary (loan) MUST freeze whatever has
    /// accumulated.
    #[test]
    fn intra_club_then_loan_drains_carried_buckets_under_loan_source() {
        let mut p = player();
        let main = team("Spartak", "spartak", "rpl");
        let second = team("Spartak 2", "spartak-2", "second-div");
        let loan_to = team("Pari", "pari", "rpl");

        // Player at Main, plays cups/friendlies + a League game, then is
        // moved to Spartak-2 (intra-club, both senior). Buckets carry.
        p.statistics_history
            .seed_initial_team(&main, d(2026, 8, 1), false);
        p.statistics = stats(1, 0);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "russia-cup".to_string(),
            statistics: stats(1, 0),
        });
        p.on_intra_club_move(&main, &second, true, true, d(2026, 9, 1));

        // Intra-club preserved the friendly + cup buckets (see comment
        // on on_intra_club_move).
        assert_eq!(p.friendly_statistics.played, 2);
        assert_eq!(p.cup_statistics_by_competition.len(), 1);

        // Player plays one more friendly at Spartak-2, then gets loaned.
        p.friendly_statistics.played += 1;
        p.on_loan(&second, &loan_to, 0.0, d(2026, 10, 1));

        // After loan: live buckets drained, and the friendly+cup are
        // attributed to the SECOND team (the team they were on at the
        // time of the loan — the drain's team_info parameter).
        assert_eq!(p.friendly_statistics.played, 0);
        assert!(p.cup_statistics_by_competition.is_empty());
        assert!(has_ledger_entry(
            &p,
            "spartak-2",
            PlayerStatCompetitionKind::Friendly,
            "second-div",
            3,
        ));
        assert!(has_ledger_entry(
            &p,
            "spartak-2",
            PlayerStatCompetitionKind::DomesticCup,
            "russia-cup",
            1,
        ));
    }

    /// `on_free_agent_signing` does not drain because a free agent in
    /// the pool plays no matches — the prior club's `on_release`
    /// already drained their live buckets. This test pins down the
    /// invariant: when the buckets are empty (the normal flow), the
    /// signing runs cleanly without leaking stats.
    #[test]
    fn on_free_agent_signing_invariant_holds_when_buckets_clean() {
        let mut p = player();
        // Simulate the prior `on_release` having drained — buckets empty.
        assert_eq!(p.friendly_statistics.total_games(), 0);
        assert!(p.cup_statistics_by_competition.is_empty());

        let to = team("Marseille", "marseille", "ligue-1");
        p.on_free_agent_signing(&to, d(2027, 1, 1));

        // Stays clean; no synthetic non-League ledger rows appeared.
        assert_eq!(p.friendly_statistics.total_games(), 0);
        assert!(p.cup_statistics_by_competition.is_empty());
        assert!(!p
            .statistics_history
            .season_ledger
            .iter()
            .any(|e| e.competition_kind != PlayerStatCompetitionKind::League));
    }

    // ── No-double-count regressions ───────────────────────────────────

    /// Loan out, return to same team in same season, play more games:
    /// the departed loan row's frozen non-League stats AND the active
    /// spell's live non-League stats must each be shown exactly once,
    /// never twice.
    #[test]
    fn loan_then_return_same_team_no_double_count_in_same_season() {
        let mut p = player();
        let parent = team("Juventus", "juventus", "serie-a");
        let torino = team("Torino", "torino", "serie-a");

        p.statistics_history
            .seed_initial_team(&parent, d(2026, 8, 1), false);

        // Loan to Torino, play League + Coppa Italia + Friendly.
        p.on_loan(&parent, &torino, 0.0, d(2026, 9, 1));
        p.statistics = stats(10, 1);
        p.friendly_statistics = stats(2, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "coppa-italia".to_string(),
            statistics: stats(3, 1),
        });

        // Return to Juventus mid-season — drain freezes everything under
        // Torino.
        p.on_loan_return(&torino, &parent, d(2027, 1, 15));

        // At Juventus, play more — both league and a new cup tie + friendly.
        p.statistics = stats(8, 2);
        p.friendly_statistics = stats(1, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "coppa-italia".to_string(),
            statistics: stats(2, 1),
        });

        // Project the breakdowns mid-season.
        let live = PlayerLiveStatsInput {
            league: &p.statistics,
            friendly: &p.friendly_statistics,
            cups: &[LiveCupSlice {
                competition_slug: "coppa-italia",
                competition_name: "Coppa Italia".to_string(),
                statistics: &p.cup_statistics_by_competition[0].statistics,
            }],
            friendly_source_slug: "",
        };
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2027, 3, 1),
        );

        // Torino row: frozen loan stats — League 10, Friendly 2, Coppa Italia 3.
        let torino_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "torino")
            .expect("Torino breakdown must exist");
        let torino_league = torino_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::League)
            .unwrap();
        assert_eq!(torino_league.statistics.played, 10);
        let torino_friendly = torino_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
            .unwrap();
        assert_eq!(torino_friendly.statistics.played, 2);
        let torino_cup = torino_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::DomesticCup)
            .unwrap();
        assert_eq!(torino_cup.statistics.played, 3);

        // Juventus row: live stats only — League 8, Friendly 1, Coppa Italia 2.
        // Critically: Torino's frozen entries must NOT appear here.
        let juve_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "juventus")
            .expect("Juventus breakdown must exist");
        let juve_league = juve_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::League)
            .unwrap();
        assert_eq!(juve_league.statistics.played, 8);
        let juve_friendly = juve_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
            .unwrap();
        assert_eq!(juve_friendly.statistics.played, 1);
        let juve_cup = juve_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::DomesticCup)
            .unwrap();
        assert_eq!(juve_cup.statistics.played, 2);
    }

    /// Transfer away from a team, then back to the same team in the
    /// same season. Each spell's non-League stats must surface exactly
    /// once and stay attributed to the spell that earned them.
    #[test]
    fn transfer_away_then_back_same_team_same_season_no_double_count() {
        let mut p = player();
        let a = team("Club A", "club-a", "league-a");
        let b = team("Club B", "club-b", "league-b");

        p.statistics_history
            .seed_initial_team(&a, d(2026, 8, 1), false);

        // First A spell: League 5, Friendly 2.
        p.statistics = stats(5, 1);
        p.friendly_statistics = stats(2, 0);
        p.on_transfer(&a, &b, 1_000_000.0, d(2026, 10, 1));

        // At B: League 3, Friendly 1, UCL 2.
        p.statistics = stats(3, 0);
        p.friendly_statistics = stats(1, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: CHAMPIONS_LEAGUE_SLUG.to_string(),
            statistics: stats(2, 0),
        });
        p.on_transfer(&b, &a, 2_000_000.0, d(2027, 1, 15));

        // Back at A: live League 7 + live Friendly 3.
        p.statistics = stats(7, 2);
        p.friendly_statistics = stats(3, 0);

        let live = PlayerLiveStatsInput {
            league: &p.statistics,
            friendly: &p.friendly_statistics,
            cups: &[],
            friendly_source_slug: "",
        };

        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2027, 3, 1),
        );

        // A row groups both A spells (live + departed first spell).
        let a_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "club-a")
            .expect("Club A breakdown");
        let a_league = a_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::League)
            .unwrap();
        // 5 (first spell, frozen on current) + 7 (live active spell).
        assert_eq!(a_league.statistics.played, 12);
        let a_friendly = a_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
            .unwrap();
        // 2 (frozen first-spell drain) + 3 (live active spell). Must
        // be 5, not 8 (no double counting).
        assert_eq!(a_friendly.statistics.played, 5);

        // B row: 3 League + 1 Friendly + 2 UCL — each exactly once.
        let b_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "club-b")
            .expect("Club B breakdown");
        let b_league = b_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::League)
            .unwrap();
        assert_eq!(b_league.statistics.played, 3);
        let b_friendly = b_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
            .unwrap();
        assert_eq!(b_friendly.statistics.played, 1);
        let b_cup = b_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::ContinentalCup)
            .unwrap();
        assert_eq!(b_cup.statistics.played, 2);
    }

    /// Cancel-loan then continue at parent in same season, with
    /// friendly/cup played on both sides. Frozen borrowing-team stats
    /// must not leak into the parent's row.
    #[test]
    fn cancel_loan_then_play_at_parent_no_cross_row_leakage() {
        let mut p = player();
        let parent = team("Spartak", "spartak", "rpl");
        let pari = team("Pari", "pari", "rpl-second");

        p.statistics_history
            .seed_initial_team(&parent, d(2026, 8, 1), false);
        p.on_loan(&parent, &pari, 0.0, d(2026, 9, 1));

        // At Pari: League 6, Friendly 3, RussiaCup 1.
        p.statistics = stats(6, 0);
        p.friendly_statistics = stats(3, 0);
        p.cup_statistics_by_competition.push(CompetitionStatistics {
            competition_slug: "russia-cup".to_string(),
            statistics: stats(1, 0),
        });
        p.on_cancel_loan(&pari, &parent, d(2026, 12, 1));

        // Back at Spartak: live League 4 + live Friendly 1 (different youth slug).
        p.statistics = stats(4, 1);
        p.friendly_statistics = stats(1, 0);

        let live = PlayerLiveStatsInput {
            league: &p.statistics,
            friendly: &p.friendly_statistics,
            cups: &[],
            friendly_source_slug: "",
        };
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &p.statistics_history,
            &live,
            d(2027, 2, 1),
        );

        // Pari row: only the frozen loan-period stats. Pari's league
        // slug is rpl-second so it groups separately from the parent's
        // rpl row.
        let pari_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "pari")
            .expect("Pari breakdown");
        assert!(pari_bd.is_loan);
        assert_eq!(
            pari_bd
                .competitions
                .iter()
                .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
                .unwrap()
                .statistics
                .played,
            3,
            "Pari friendly stays under Pari"
        );
        assert_eq!(
            pari_bd
                .competitions
                .iter()
                .find(|c| c.competition_kind == PlayerStatCompetitionKind::DomesticCup)
                .unwrap()
                .statistics
                .played,
            1,
        );

        // Spartak row: ONLY live stats — Pari frozen entries must not
        // have leaked here.
        let spartak_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "spartak")
            .expect("Spartak breakdown");
        assert!(!spartak_bd.is_loan);
        assert_eq!(
            spartak_bd
                .competitions
                .iter()
                .find(|c| c.competition_kind == PlayerStatCompetitionKind::League)
                .unwrap()
                .statistics
                .played,
            4,
        );
        let spartak_friendly = spartak_bd
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
            .unwrap();
        assert_eq!(spartak_friendly.statistics.played, 1);
        // No DomesticCup leakage from Pari.
        assert!(
            !spartak_bd
                .competitions
                .iter()
                .any(|c| c.competition_kind == PlayerStatCompetitionKind::DomesticCup),
            "Pari domestic cup must not leak to Spartak"
        );
    }

    /// User-reported repro: a young player on loan at a senior team
    /// plays U19 friendlies. Live view shows "Premier League U19" in
    /// the breakdown. After cancel-loan, that line collapses to the
    /// generic "Friendly" because the drain stamps the canonical
    /// ledger entry with the senior team's league_slug instead of the
    /// U19 league slug. Match-record time captures the actual league
    /// slug on `player.friendly_source_slug`; drain consumes it.
    #[test]
    fn cancel_loan_keeps_youth_friendly_source_slug() {
        let mut p = player();
        let parent = team("Spartak", "spartak", "russian-premier-league");
        let cska = team("CSKA Moscow", "cska", "russian-premier-league");

        p.statistics_history
            .seed_initial_team(&parent, d(2026, 8, 1), false);
        p.on_loan(&parent, &cska, 0.0, d(2026, 9, 1));

        // Simulate match recording: 1 U19 friendly appearance for CSKA.
        // This is what `record_match_appearance` would do on its own
        // when the match-engine emits a MatchOutcome with is_friendly=true
        // and competition_slug="russian-premier-league-u19".
        p.friendly_statistics = stats(1, 0);
        p.friendly_source_slug = Some("russian-premier-league-u19".to_string());

        // Cancel-loan with no explicit override — the drain reads from
        // the player's own `friendly_source_slug` field.
        p.on_cancel_loan(&cska, &parent, d(2026, 12, 1));

        // Ledger entry was stamped with the U19 league slug, NOT the
        // senior "russian-premier-league" — so the breakdown won't
        // collapse to the generic "Friendly".
        assert!(has_ledger_entry(
            &p,
            "cska",
            PlayerStatCompetitionKind::Friendly,
            "russian-premier-league-u19",
            1,
        ));
        // And the live recorded slug is consumed (so it doesn't leak
        // into the next spell after the drain).
        assert!(p.friendly_source_slug.is_none());
    }
}
