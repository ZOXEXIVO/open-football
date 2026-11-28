use crate::MatchRuntime;
use crate::club::team::NearestFixtures;
use crate::context::GlobalContext;
use crate::context::{HomeLeagueTable, TournamentClocks};
use crate::continent::ContinentalCommitments;
use crate::country::builder::CountryBuilder;
use crate::country::national::NationalTeam;
use crate::country::process::{
    CountryClubFanout, CountryLeagueFanout, CountryMatchdayResults, CountryTicks,
    CountryTransferMarketTick,
};
use crate::country::{CountryResult, RetiredPlayers};
use crate::league::LeagueBuildOutput;
use crate::league::LeagueCollection;
use crate::league::LeaguePendingState;
use crate::league::result::WorldSnapshot;
use crate::league::simulation::context::MatchdaySide;
use crate::league::simulation::matchday::MatchdayCommitments;
use crate::league::{
    CompetitionLevel, FixtureRest, KickoffClock, KnockoutStakes, KnockoutTieReading,
    OpenedMatchday, Schedule, TableStakes,
};
use crate::r#match::Match;
use crate::r#match::MatchResult;
use crate::shared::CountryLookupIndex;
use crate::transfers::market::TransferMarket;
use crate::transfers::market::map::CountryTransferProfile;
use crate::utils::PerformanceProfiler;
use crate::{Club, ClubResult, Player};
use chrono::{Datelike, NaiveDate};
use log::debug;
use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::prelude::IntoParallelRefMutIterator;

use crate::Team;
use crate::country::{
    CountryEconomicFactors, CountryGeneratorData, CountryRegulations, CountrySettings,
    InternationalCompetition, MediaCoverage,
};
use crate::league::DomesticCup;
use crate::league::League;
use crate::league::LeagueResult;
use crate::league::playoff::{GroupStanding, LeaguePlayoff, StandingRow};
use rustc_hash::FxHashMap;

/// State stashed between [`Country::simulate_build`] and
/// [`Country::simulate_process`] for one tick. The build pass mutates
/// the country's leagues and cup up to (and including) schedule
/// regeneration, hands the assembled `Match` objects to the caller for
/// a batched dispatch, and packages here whatever per-league /
/// per-cup state the process pass needs to resume.
pub struct CountryPendingState {
    /// One slot per entry in `self.leagues.leagues`, in the same order.
    /// `Some` when today produced league matches to play; `None` when
    /// the league had no fixtures (its `LeagueResult` is already in
    /// `immediate_results`).
    pub leagues: Vec<Option<LeaguePendingState>>,
    /// Cup pending state — `Some` when today produced cup matches.
    pub cup: Option<LeaguePendingState>,
    /// One slot per entry in `self.playoffs`, in the same order. `Some`
    /// when that playoff produced knockout matches today.
    pub playoffs: Vec<Option<LeaguePendingState>>,
    /// `LeagueResult`s that finalised during the build pass — leagues
    /// without matches today, plus cups on a non-fixture day. Merged
    /// with the post-process `LeagueResult`s in [`simulate_process`].
    pub immediate_results: Vec<LeagueResult>,
}

#[derive(Clone)]
pub struct Country {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    pub background_color: String,
    pub foreground_color: String,
    pub continent_id: u32,
    pub leagues: LeagueCollection,
    /// Domestic knockout cup (FA Cup, Copa del Rey, …). Stored apart from
    /// `leagues` so the standings programme stays purely round-robin; the
    /// cup is a first-class competition with its own knockout bracket.
    /// `None` only for countries with no enabled leagues.
    pub domestic_cup: Option<DomesticCup>,
    /// End-of-season playoffs for grouped competitions (MLS Cup, …). One
    /// entry per competition whose `league_group` declares a `playoff`.
    /// Empty for countries with no grouped competitions. Stored apart from
    /// `leagues` (like `domestic_cup`) so the round-robin programme is
    /// untouched; each playoff runs its own knockout bracket seeded from
    /// the member groups' final standings.
    pub playoffs: Vec<LeaguePlayoff>,
    pub clubs: Vec<Club>,
    pub reputation: u16,
    pub settings: CountrySettings,
    pub generator_data: CountryGeneratorData,

    pub national_team: NationalTeam,
    /// Under-21 national team — a parallel youth side selected from a
    /// separate (younger) candidate pool, with its own squad, schedule,
    /// caps, and match-day statuses. Mirrors `national_team` but at
    /// `NationalTeamLevel::Under21`.
    pub u21_national_team: NationalTeam,

    pub transfer_market: TransferMarket,
    pub economic_factors: CountryEconomicFactors,
    pub international_competitions: Vec<InternationalCompetition>,
    pub media_coverage: MediaCoverage,
    pub(crate) scout_visits: crate::club::news::sightings::ScoutVisitLog,
    pub regulations: CountryRegulations,

    /// The country's transfer-market card: who its clubs import from, where
    /// its nationals go, where its diaspora lives, and how foreign its top
    /// division runs. Shipped data, normalised at load. Empty for a country
    /// the data does not name, in which case every pair it takes part in
    /// falls to [`crate::transfers::CorridorPrior::derive`].
    pub transfer_profile: CountryTransferProfile,

    pub retired_players: RetiredPlayers,

    /// Positions of this country's players, teams and clubs for its own
    /// result fan-out, kept between ticks and reconciled before each use.
    pub lookup: CountryLookupIndex,

    /// `start_year` of the most recent season for which the per-player
    /// statistics snapshot has fired. `None` before the first
    /// snapshot runs. Used as a watermark so the snapshot can catch
    /// up on missed seasons (any league failing the
    /// `new_season_started` gate would otherwise leave a gap in
    /// every player's career history table). Updated by
    /// `snapshot_player_season_statistics` once for each season it
    /// processes.
    pub last_snapshotted_season_year: Option<u16>,

    /// Months until the confederation's next major tournament, `u8::MAX`
    /// for none in view. Stamped by the continent tick from the same
    /// arithmetic that fills [`crate::country::CountryContext`].
    ///
    /// Denormalised onto the country because the transfer market runs
    /// outside `GlobalContext`: a squad-edge international eleven months
    /// from a tournament weighs a money move very differently from the
    /// same man in the summer after one, and personal terms had no way to
    /// know which he was.
    pub months_to_tournament: u8,
    /// Every confederation's calendar, republished on this country each
    /// tick. The transfer market runs outside `GlobalContext`, and a
    /// player's tournament clock belongs to his PASSPORT — see
    /// [`crate::context::TournamentClocks`].
    pub tournament_clocks: TournamentClocks,
    /// What every country's best league is worth, republished here each
    /// tick for the same reason the clocks are: the transfer market runs
    /// outside `GlobalContext`, and "is his HOME league worth going back
    /// to?" is a question about a passport that no single country can
    /// answer. See [`crate::context::HomeLeagueTable`].
    pub home_leagues: HomeLeagueTable,
}

impl Country {
    /// Months to the tournament THIS player might be picked for. Falls
    /// back to the country's own confederation when his passport has
    /// never been stamped, which is the honest no-view and the right
    /// answer for a native.
    pub fn months_to_tournament_for(&self, nationality_continent_id: Option<u32>) -> u8 {
        self.tournament_clocks
            .months_for(nationality_continent_id)
            .unwrap_or(self.months_to_tournament)
    }
}

/// Season boundary dates derived from a country's primary league settings.
#[derive(Debug, Clone, Copy)]
pub struct SeasonDates {
    /// Day/month when the season ends (from season_ending_half.to_day/to_month)
    pub end_day: u8,
    pub end_month: u8,
    /// Day/month when the new season starts (from season_starting_half.from_day/from_month)
    pub start_day: u8,
    pub start_month: u8,
}

impl Default for SeasonDates {
    fn default() -> Self {
        SeasonDates {
            end_day: 31,
            end_month: 5,
            start_day: 20,
            start_month: 8,
        }
    }
}

impl SeasonDates {
    /// Check if the given date is the season end day.
    pub fn is_season_end(&self, date: NaiveDate) -> bool {
        date.day() as u8 == self.end_day && date.month() as u8 == self.end_month
    }

    /// Check if the given date falls in the off-season (between season end and season start).
    pub fn is_off_season(&self, date: NaiveDate) -> bool {
        let m = date.month() as u8;
        let d = date.day() as u8;
        let after_end = m > self.end_month || (m == self.end_month && d > self.end_day);
        let before_start = m < self.start_month || (m == self.start_month && d < self.start_day);
        after_end && before_start
    }
}

impl Country {
    pub fn builder() -> CountryBuilder {
        CountryBuilder::default()
    }

    // ── Country-local accessors ─────────────────────────────────────
    // Linear-scan lookups scoped to a single country. Used by the
    // post-Phase-A result processors that run inside
    // `countries.par_iter_mut()` — they only have `&mut Country`, not
    // `&mut SimulatorData`. Linear scan is fine for the world sizes
    // we care about (≤ 30 clubs, ≤ 150 teams, ≤ 700 players); if a
    // profile flags any of these as hot we promote them to per-country
    // HashMap indexes.
    pub fn club(&self, id: u32) -> Option<&Club> {
        self.clubs.iter().find(|c| c.id == id)
    }
    pub fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        self.clubs.iter_mut().find(|c| c.id == id)
    }
    pub fn team(&self, id: u32) -> Option<&Team> {
        for club in &self.clubs {
            for team in &club.teams.teams {
                if team.id == id {
                    return Some(team);
                }
            }
        }
        None
    }
    pub fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        for club in &mut self.clubs {
            for team in &mut club.teams.teams {
                if team.id == id {
                    return Some(team);
                }
            }
        }
        None
    }
    pub fn player(&self, id: u32) -> Option<&Player> {
        for club in &self.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.id == id {
                        return Some(player);
                    }
                }
            }
        }
        None
    }
    pub fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
        for club in &mut self.clubs {
            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    if player.id == id {
                        return Some(player);
                    }
                }
            }
        }
        None
    }
    pub fn league(&self, id: u32) -> Option<&League> {
        // The domestic cup lives outside `leagues` but is still a `League`
        // under the hood, so id-based lookups (stat routing, schedule
        // updates, web) must see it too — otherwise cup matches would be
        // classified as league games (`is_cup == false`).
        self.leagues
            .leagues
            .iter()
            .find(|l| l.id == id)
            .or_else(|| {
                self.domestic_cup
                    .as_ref()
                    .filter(|c| c.id() == id)
                    .map(|c| &c.league)
            })
    }
    pub fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        if self.leagues.leagues.iter().any(|l| l.id == id) {
            return self.leagues.leagues.iter_mut().find(|l| l.id == id);
        }
        self.domestic_cup
            .as_mut()
            .filter(|c| c.id() == id)
            .map(|c| &mut c.league)
    }

    /// True iff `club_id` belongs to this country. Used as a cheap
    /// guard for "is this entity in my country" checks that were
    /// previously `data.country_by_club(id).map(|c| c.id) == Some(self.id)`.
    pub fn owns_club(&self, club_id: u32) -> bool {
        self.clubs.iter().any(|c| c.id == club_id)
    }

    /// Get season dates from the country's primary (tier-1, non-friendly) league.
    /// Falls back to May 31 / Aug 20 if no league is found.
    pub fn season_dates(&self) -> SeasonDates {
        self.leagues
            .leagues
            .iter()
            .find(|l| !l.friendly && l.settings.tier == 1)
            .or_else(|| self.leagues.leagues.iter().find(|l| !l.friendly))
            .map(|l| SeasonDates {
                end_day: l.settings.season_ending_half.to_day,
                end_month: l.settings.season_ending_half.to_month,
                start_day: l.settings.season_starting_half.from_day,
                start_month: l.settings.season_starting_half.from_month,
            })
            .unwrap_or_default()
    }

    /// Every fixture list the country keeps: its divisions (youth included),
    /// the domestic cup and the grouped-competition playoffs.
    pub(crate) fn schedules(&self) -> impl Iterator<Item = &Schedule> {
        self.leagues
            .leagues
            .iter()
            .map(|l| &l.schedule)
            .chain(self.domestic_cup.iter().map(|c| &c.league.schedule))
            .chain(self.playoffs.iter().map(|p| &p.league.schedule))
    }

    /// The competitions a first team plays in — every schedule but the
    /// youth divisions — with each one's position in [`schedules`].
    fn competitive_schedules(&self) -> impl Iterator<Item = (usize, &Schedule)> {
        let friendly: Vec<bool> = self.leagues.leagues.iter().map(|l| l.friendly).collect();
        self.schedules()
            .enumerate()
            .filter(move |(index, _)| !friendly.get(*index).copied().unwrap_or(false))
    }

    /// The schedule at `index` in [`schedules`] order.
    fn schedule_mut(&mut self, index: usize) -> &mut Schedule {
        let leagues = self.leagues.leagues.len();
        let cup = self.domestic_cup.is_some() as usize;
        if index < leagues {
            &mut self.leagues.leagues[index].schedule
        } else if index < leagues + cup {
            &mut self.domestic_cup.as_mut().unwrap().league.schedule
        } else {
            &mut self.playoffs[index - leagues - cup].league.schedule
        }
    }

    /// Rearrange unplayed domestic fixtures that leave a first team short of
    /// rest. Any fixture inside the rest gap of one of its club's continental
    /// nights moves: the Saturday game after a Thursday tie goes to Sunday, a
    /// cup tie on a Champions League night to the next free midweek. A cup or
    /// playoff tie inside the rest gap of any other fixture of either side
    /// moves too; league fixtures give way to nothing domestic. Both sides'
    /// other fixtures are respected; the fixture keeps its id, round and
    /// venue.
    pub(crate) fn rearrange_for_rest(
        &mut self,
        commitments: &ContinentalCommitments,
        today: NaiveDate,
    ) {
        let continental: FxHashMap<u32, &[NaiveDate]> = self
            .clubs
            .iter()
            .filter(|club| !commitments.dates(club.id).is_empty())
            .filter_map(|club| {
                club.teams
                    .main_team_id()
                    .map(|team_id| (team_id, commitments.dates(club.id)))
            })
            .collect();
        let leagues = self.leagues.leagues.len();
        let knockout_pending = self.schedules().skip(leagues).any(|schedule| {
            schedule
                .tours
                .iter()
                .flat_map(|t| &t.items)
                .any(|item| item.result.is_none() && item.date.date() >= today)
        });
        if continental.is_empty() && !knockout_pending {
            return;
        }

        let mut busy: FxHashMap<u32, Vec<NaiveDate>> = FxHashMap::default();
        for (_, schedule) in self.competitive_schedules() {
            for item in schedule.tours.iter().flat_map(|t| &t.items) {
                let day = item.date.date();
                busy.entry(item.home_team_id).or_default().push(day);
                busy.entry(item.away_team_id).or_default().push(day);
            }
        }

        let mut clashes: Vec<(NaiveDate, String, usize, u32, u32)> = Vec::new();
        for (index, schedule) in self.competitive_schedules() {
            for item in schedule.tours.iter().flat_map(|t| &t.items) {
                let day = item.date.date();
                if item.result.is_some() || day < today {
                    continue;
                }
                let sides = [item.home_team_id, item.away_team_id];
                let clashes_with_europe = sides
                    .iter()
                    .filter_map(|team| continental.get(team))
                    .any(|nights| !FixtureRest::is_rested(day, nights));
                // Its own date is among the side's days, so a second one
                // inside the gap is the clash.
                let crowded_knockout = index >= leagues
                    && sides.iter().filter_map(|team| busy.get(team)).any(|days| {
                        days.iter()
                            .filter(|d| (day - **d).num_days().abs() < FixtureRest::MIN_REST_DAYS)
                            .count()
                            > 1
                    });
                if clashes_with_europe || crowded_knockout {
                    clashes.push((day, item.id.clone(), index, sides[0], sides[1]));
                }
            }
        }
        if clashes.is_empty() {
            return;
        }
        for (team, nights) in &continental {
            busy.entry(*team).or_default().extend(nights.iter());
        }
        clashes.sort();

        for (nominal, id, index, home, away) in clashes {
            let others = |team: u32| -> Vec<NaiveDate> {
                let mut dates = busy.get(&team).cloned().unwrap_or_default();
                if let Some(own) = dates.iter().position(|d| *d == nominal) {
                    dates.swap_remove(own);
                }
                dates
            };
            let (home_busy, away_busy) = (others(home), others(away));
            if FixtureRest::is_rested(nominal, &home_busy)
                && FixtureRest::is_rested(nominal, &away_busy)
            {
                continue;
            }
            let day = FixtureRest::best_day(nominal, today, &home_busy, &away_busy);
            if day == nominal {
                continue;
            }

            let schedule = self.schedule_mut(index);
            let slot = schedule.teams_playing_on(day).count() / 2;
            schedule.move_fixture(&id, KickoffClock::at(day, CompetitionLevel::Senior, slot));
            debug!("📅 fixture {id} rearranged from {nominal} to {day} for rest");

            for team in [home, away] {
                let dates = busy.entry(team).or_default();
                if let Some(own) = dates.iter().position(|d| *d == nominal) {
                    dates[own] = day;
                }
                if let Some(club) = self
                    .clubs
                    .iter_mut()
                    .find(|club| club.teams.teams.iter().any(|t| t.id == team))
                {
                    club.on_fixture_rearranged(team, nominal, day, today);
                }
            }
        }
    }

    /// Every one of the country's teams with a fixture on `date`.
    pub(crate) fn matchday_commitments(&self, date: NaiveDate) -> MatchdayCommitments {
        MatchdayCommitments::gather(self.schedules(), date)
    }

    /// Build (but do not play) today's matches across every league and
    /// the domestic cup. Mutates each league's state up to (and
    /// including) schedule regeneration, then hands the assembled
    /// `Match` objects to the caller for a batched engine dispatch.
    /// The returned [`CountryPendingState`] is the resume token for
    /// [`simulate_process`].
    ///
    /// The caller — `Continent::simulate` — collects build outputs
    /// from every country into a `ContinentBuildOutput`, then the
    /// simulator aggregates every continent's `ContinentBuildOutput`
    /// into ONE `WorldMatchdayResult` and dispatches via
    /// `engine_pool().play(..)` exactly once per tick. With external
    /// workers this turns N tiny per-league round-trips into a single
    /// world-wide batch that fans across the entire fleet — the whole
    /// point of the split.
    pub fn simulate_build(
        &mut self,
        ctx: &GlobalContext<'_>,
        _world: WorldSnapshot<'_>,
    ) -> (Vec<Match>, CountryPendingState) {
        debug!(
            "Building matchday for country: {} (Reputation: {})",
            self.name, self.reputation
        );

        let teams_by_league = self.teams_by_league();
        let (league_outputs, commitments) = self.build_league_outputs(ctx, &teams_by_league);
        let cup_output = self.build_cup_output(ctx, &commitments);
        let playoff_outputs = self.build_playoff_outputs(ctx, &commitments);
        Self::assemble_build_outputs(league_outputs, cup_output, playoff_outputs)
    }

    fn teams_by_league(&self) -> FxHashMap<u32, Vec<u32>> {
        let mut teams_by_league: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        for team in self.clubs.iter().flat_map(|club| &club.teams.teams) {
            if let Some(league_id) = team.league_id {
                teams_by_league.entry(league_id).or_default().push(team.id);
            }
        }
        teams_by_league
    }

    fn build_league_outputs(
        &mut self,
        ctx: &GlobalContext<'_>,
        teams_by_league: &FxHashMap<u32, Vec<u32>>,
    ) -> (Vec<LeagueBuildOutput>, MatchdayCommitments) {
        let clubs = &self.clubs;
        let league_ctx = |league: &League| {
            let league_team_ids: &[u32] = teams_by_league
                .get(&league.id)
                .map(|team_ids| team_ids.as_slice())
                .unwrap_or(&[]);
            ctx.with_league(
                league.id,
                league.slug.clone(),
                league_team_ids,
                league.reputation,
            )
        };
        let opened: Vec<OpenedMatchday> = self
            .leagues
            .leagues
            .par_iter_mut()
            .map(|league| {
                let league_ctx = league_ctx(league);
                league.open_matchday(clubs, &league_ctx)
            })
            .collect();
        let commitments = self.matchday_commitments(ctx.simulation.date.date());
        let clubs = &self.clubs;
        let outputs = self
            .leagues
            .leagues
            .par_iter_mut()
            .zip(opened)
            .map(|(league, opened)| {
                let league_ctx = league_ctx(league);
                league.build_matchday(opened, clubs, &league_ctx, &commitments)
            })
            .collect();
        (outputs, commitments)
    }

    fn build_cup_output(
        &mut self,
        ctx: &GlobalContext<'_>,
        commitments: &MatchdayCommitments,
    ) -> Option<LeagueBuildOutput> {
        let (cup_id, cup_slug, cup_rep) = self.domestic_cup.as_ref().map(|cup| {
            (
                cup.league.id,
                cup.league.slug.clone(),
                cup.league.reputation,
            )
        })?;
        let cup_ctx = ctx.with_league(cup_id, cup_slug, &[], cup_rep);
        let cup = self.domestic_cup.as_mut().expect("cup checked above");
        Some(cup.simulate_build(&self.clubs, &cup_ctx, commitments))
    }

    fn build_playoff_outputs(
        &mut self,
        ctx: &GlobalContext<'_>,
        commitments: &MatchdayCommitments,
    ) -> Vec<LeagueBuildOutput> {
        if self.playoffs.is_empty() {
            return Vec::new();
        }
        let group_snaps = self.group_standings();
        let mut outputs = Vec::with_capacity(self.playoffs.len());
        for playoff in &mut self.playoffs {
            let playoff_ctx = ctx.with_league(
                playoff.league.id,
                playoff.league.slug.clone(),
                &[],
                playoff.league.reputation,
            );
            outputs.push(playoff.simulate_build(
                &self.clubs,
                &group_snaps,
                &playoff_ctx,
                commitments,
            ));
        }
        outputs
    }

    fn group_standings(&self) -> Vec<GroupStanding> {
        self.leagues
            .leagues
            .iter()
            .map(|league| {
                let first_stage_rows = if league.settings.split_season {
                    match &league.split_first_table {
                        Some(frozen) => frozen.iter().map(StandingRow::from_table_row).collect(),
                        None => league
                            .table
                            .rows
                            .iter()
                            .map(StandingRow::from_table_row)
                            .collect(),
                    }
                } else {
                    Vec::new()
                };
                GroupStanding {
                    league_id: league.id,
                    complete: league.schedule.fully_played(),
                    rows: league
                        .table
                        .rows
                        .iter()
                        .map(StandingRow::from_table_row)
                        .collect(),
                    first_stage_complete: league.first_stage_played(),
                    first_stage_rows,
                }
            })
            .collect()
    }

    fn assemble_build_outputs(
        league_outputs: Vec<LeagueBuildOutput>,
        cup_output: Option<LeagueBuildOutput>,
        playoff_outputs: Vec<LeagueBuildOutput>,
    ) -> (Vec<Match>, CountryPendingState) {
        let mut pending = CountryPendingState {
            leagues: Vec::with_capacity(league_outputs.len()),
            cup: None,
            playoffs: Vec::with_capacity(playoff_outputs.len()),
            immediate_results: Vec::new(),
        };
        let mut all_matches = Vec::new();

        for output in league_outputs {
            all_matches.extend(output.matches);
            pending.leagues.push(output.pending);
            if let Some(result) = output.immediate {
                pending.immediate_results.push(result);
            }
        }
        if let Some(output) = cup_output {
            all_matches.extend(output.matches);
            pending.cup = output.pending;
            if let Some(result) = output.immediate {
                pending.immediate_results.push(result);
            }
        }
        for output in playoff_outputs {
            all_matches.extend(output.matches);
            pending.playoffs.push(output.pending);
            if let Some(result) = output.immediate {
                pending.immediate_results.push(result);
            }
        }

        (all_matches, pending)
    }

    /// Resume one tick after the engine has played the country's slice
    /// of the continent batch. Routes results to each league / cup,
    /// then runs the rest of the country tick (club simulation,
    /// transfer market, country-local result processing) exactly as
    /// the legacy `simulate` did.
    pub fn simulate_process(
        &mut self,
        ctx: GlobalContext<'_>,
        world: WorldSnapshot<'_>,
        pending: CountryPendingState,
        match_results: Vec<MatchResult>,
    ) -> CountryResult {
        let current_date = ctx.simulation.date.date();
        let results =
            CountryMatchdayResults::route(self, &ctx, current_date, pending, match_results);

        let clubs_results = CountryTicks::run(self, ctx, current_date, results.league_results());
        let mut results = results.with_clubs(clubs_results);

        let lookup = CountryClubFanout::reconcile(self);
        let (league_results, processed) = CountryLeagueFanout::run(
            self,
            world,
            results.take_league_results(),
            &lookup,
            results.deferred_global_ops_mut(),
        );
        results.set_league_results(league_results);
        results.record_processed(processed);

        let clubs_results = CountryClubFanout::run(
            self,
            world,
            results.take_clubs(),
            &lookup,
            results.deferred_global_ops_mut(),
        );
        let results = results.with_clubs(clubs_results);
        CountryClubFanout::restore(self, lookup);

        let transfer_ops = CountryTransferMarketTick::run(self, current_date, world);
        debug!("Country {} simulation complete", self.name);

        results.into_country_result(self.id, transfer_ops)
    }

    /// Single-call wrapper for the build → engine → process flow.
    /// Test-only: the production path goes through
    /// `Continent::simulate` (build-only) +
    /// `WorldMatchdayResult::process` (single global dispatch). This
    /// wrapper short-circuits both so a single-country test setup
    /// doesn't have to wire up a continent and the world matchday
    /// result.
    #[allow(dead_code)]
    pub(crate) fn simulate(
        &mut self,
        ctx: GlobalContext<'_>,
        world: WorldSnapshot<'_>,
    ) -> CountryResult {
        let (matches, pending) = self.simulate_build(&ctx, world);
        let match_results = if matches.is_empty() {
            Vec::new()
        } else {
            MatchRuntime::engine_pool().play(matches)
        };
        self.simulate_process(ctx, world, pending, match_results)
    }

    pub(super) fn simulate_clubs(&mut self, ctx: &GlobalContext<'_>) -> Vec<ClubResult> {
        let prologue = PerformanceProfiler::stage_scope("country_club_prologue", 3);
        // Build team_id → (position, league_size, total_matches, matches_played, tier, league_rep)
        let mut team_league_info: FxHashMap<u32, (u8, u8, u8, u8, u8, u16)> = FxHashMap::default();
        for league in &self.leagues.leagues {
            if league.friendly {
                continue;
            }
            let league_size = league.table.rows.len() as u8;
            let total_matches = if league_size > 1 {
                (league_size - 1) * 2
            } else {
                0
            };
            let tier = league.settings.tier.max(1);
            let league_rep = league.reputation;
            for (pos, row) in league.table.rows.iter().enumerate() {
                team_league_info.insert(
                    row.team_id,
                    (
                        (pos + 1) as u8,
                        league_size,
                        total_matches,
                        row.played,
                        tier,
                        league_rep,
                    ),
                );
            }
        }

        let country_reputation = self.reputation;
        drop(prologue);

        self.clubs
            .par_iter_mut()
            .map(|club| {
                // Stamp each team with the reputation of the league it
                // actually competes in (same per-tick refresh pattern as
                // fixture_window). B/reserve squads in real lower
                // divisions must develop at their own level, not the
                // main team's.
                for team in club.teams.teams.iter_mut() {
                    if let Some(info) = team_league_info.get(&team.id) {
                        team.league_reputation = info.5;
                    }
                }

                let league_info = club
                    .teams
                    .main()
                    .and_then(|t| team_league_info.get(&t.id))
                    .copied()
                    .unwrap_or((0, 0, 0, 0, 1, 0));

                let (main_blended_rep, main_world_rep) = club
                    .teams
                    .main()
                    .map(|t| {
                        let blended = t.reputation.market_value_score();
                        let world = t.reputation.world;
                        (blended, world)
                    })
                    .unwrap_or((0, 0));

                let name = club.name.clone();
                let club_ctx = ctx.with_club(club.id, &name);
                let club_ctx = {
                    let mut c = club_ctx;
                    if let Some(ref mut cc) = c.club {
                        *cc = cc
                            .clone()
                            .with_league_position(
                                league_info.0,
                                league_info.1,
                                league_info.2,
                                league_info.3,
                            )
                            .with_main_league_tier(league_info.4)
                            .with_reputations(
                                main_blended_rep,
                                main_world_rep,
                                league_info.5,
                                country_reputation,
                            );
                    }
                    c
                };
                club.simulate(club_ctx)
            })
            .collect()
    }

    /// Walk every competitive schedule and write each team's next four
    /// upcoming fixtures, with what each is worth to it today, and its last
    /// four recent ones into `Team::fixture_window`. Skips friendly leagues
    /// — those don't drive the MD-1/MD-2 training rhythm. The continent
    /// keeps its own run on the same window. Cheap: scales O(fixtures) per
    /// league once a tick.
    pub(super) fn refresh_team_fixture_windows(&mut self, today: NaiveDate) {
        let mut nearest: FxHashMap<u32, NearestFixtures> = FxHashMap::default();
        for league in self.leagues.leagues.iter().filter(|l| !l.friendly) {
            let stakes = league.table_stakes();
            let worth = |team_id: u32| {
                stakes
                    .get(&team_id)
                    .copied()
                    .unwrap_or(TableStakes::ROUTINE)
            };
            FixtureWindowRefresh::offer(&mut nearest, &league.schedule, today, |_, home, away| {
                (worth(home), worth(away))
            });
        }

        // Cup ties and play-offs count toward the week just like league
        // games; a bracket says what each tie is worth from its round and
        // the two sides.
        let strength: FxHashMap<u32, u16> = self
            .clubs
            .iter()
            .flat_map(|club| club.teams.teams.iter())
            .map(|team| (team.id, team.reputation.market_value_score()))
            .collect();
        let cup = self.domestic_cup.as_ref().map(|cup| {
            (
                &cup.league.schedule,
                DomesticCup::bracket_rounds(&self.clubs),
            )
        });
        let playoffs = self
            .playoffs
            .iter()
            .map(|playoff| (&playoff.league.schedule, playoff.bracket_rounds()));
        for (schedule, total_rounds) in cup.into_iter().chain(playoffs) {
            let worth = |round: u8, own: u32, other: u32| {
                let ratio = match (strength.get(&own), strength.get(&other)) {
                    (Some(&own), Some(&other)) => f32::from(other) / f32::from(own.max(1)),
                    _ => 1.0,
                };
                KnockoutStakes::of(KnockoutTieReading {
                    round,
                    total_rounds,
                    opponent_ratio: ratio,
                    aggregate_lead: None,
                })
            };
            FixtureWindowRefresh::offer(&mut nearest, schedule, today, |round, home, away| {
                (worth(round, home, away), worth(round, away, home))
            });
        }

        let mut development: FxHashMap<u32, NearestFixtures> = FxHashMap::default();
        for league in self.leagues.leagues.iter().filter(|l| l.friendly) {
            FixtureWindowRefresh::offer(&mut development, &league.schedule, today, |_, _, _| {
                (MatchdaySide::FRIENDLY_STAKES, MatchdaySide::FRIENDLY_STAKES)
            });
        }

        for club in &mut self.clubs {
            for team in &mut club.teams.teams {
                team.fixture_window.domestic =
                    nearest.remove(&team.id).unwrap_or_default().into_run();
                team.fixture_window.development =
                    development.remove(&team.id).unwrap_or_default().into_run();
                team.fixture_window.refreshed = Some(today);
            }
        }
    }
}

/// Folding one schedule into the teams' nearest fixtures.
struct FixtureWindowRefresh;

impl FixtureWindowRefresh {
    /// `worth` answers `(round, home, away)` with what the fixture is worth
    /// to the home side and to the away side.
    fn offer(
        nearest: &mut FxHashMap<u32, NearestFixtures>,
        schedule: &Schedule,
        today: NaiveDate,
        worth: impl Fn(u8, u32, u32) -> (f32, f32),
    ) {
        for tour in &schedule.tours {
            for item in &tour.items {
                let d = item.date.date();
                if d > today && item.result.is_none() {
                    let (home, away) = worth(tour.num, item.home_team_id, item.away_team_id);
                    nearest
                        .entry(item.home_team_id)
                        .or_default()
                        .offer_upcoming(d, home);
                    nearest
                        .entry(item.away_team_id)
                        .or_default()
                        .offer_upcoming(d, away);
                } else if d <= today && item.result.is_some() {
                    nearest
                        .entry(item.home_team_id)
                        .or_default()
                        .offer_recent(d);
                    nearest
                        .entry(item.away_team_id)
                        .or_default()
                        .offer_recent(d);
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture_calendar_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::context::SimulationContext;
    use crate::league::{
        DayMonthPeriod, LeagueMatch, LeagueSettings, LeagueTable, LeagueTableResult, ScheduleItem,
        ScheduleTour,
    };
    use crate::shared::Location;
    use crate::transfers::market::PlacementReachIndex;
    use crate::transfers::market::map::MarketMap;
    use crate::transfers::view::world::WorldPool;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PeopleNameGeneratorData,
        PlayerCollection, PlayerGenerator, PlayerPositionType, PlayerSkills, StaffCollection,
        TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDateTime, NaiveTime};
    use rustc_hash::FxHashSet;

    fn names() -> PeopleNameGeneratorData {
        PeopleNameGeneratorData {
            first_names: vec!["Test".to_string()],
            last_names: vec!["Player".to_string()],
            nicknames: Vec::new(),
        }
    }

    fn players(ids: std::ops::RangeInclusive<u32>) -> Vec<Player> {
        let now = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        ids.map(|id| {
            let position = if id % 11 == 0 {
                PlayerPositionType::Goalkeeper
            } else {
                PlayerPositionType::MidfielderCenter
            };
            let mut p = PlayerGenerator::generate(1, now, position, 15, &names());
            p.id = id;
            p.player_attributes.current_ability = 100;
            p.skills = PlayerSkills::flat_for_ability(100);
            p
        })
        .collect()
    }

    fn team(
        id: u32,
        club_id: u32,
        league_id: u32,
        team_type: TeamType,
        squad: Vec<Player>,
    ) -> Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("team-{id}"))
            .slug(format!("team-{id}"))
            .team_type(team_type)
            .players(PlayerCollection::new(squad))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 100))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn club(id: u32, teams: Vec<Team>) -> Club {
        Club::new(
            id,
            format!("club-{id}"),
            Location::new(1),
            ClubFinances::new(10_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        )
    }

    fn league(id: u32, tier: u8) -> League {
        League::new(
            id,
            format!("league-{id}"),
            format!("league-{id}"),
            1,
            5000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
                split_season: false,
            },
            false,
        )
    }

    #[test]
    fn a_first_team_and_its_second_team_never_share_a_player_on_opening_day() {
        // Thin first teams (nine men) against full second teams, both
        // divisions opening on Saturday 1 August 2026: the schedule is
        // drawn that morning and both sides play that afternoon, so the
        // first team's shortfall borrow would reach straight into its own
        // second team's dressing room.
        let clubs: Vec<Club> = (1..=6)
            .map(|n: u32| {
                let base = n * 1_000;
                club(
                    n * 100,
                    vec![
                        team(
                            n * 10,
                            n * 100,
                            10,
                            TeamType::Main,
                            players(base + 1..=base + 9),
                        ),
                        team(
                            n * 10 + 1,
                            n * 100,
                            20,
                            TeamType::Second,
                            players(base + 101..=base + 116),
                        ),
                        // An academy squad with no fixture of its own: both
                        // playing sides would reach for it.
                        team(
                            n * 10 + 2,
                            n * 100,
                            99,
                            TeamType::U20,
                            players(base + 201..=base + 216),
                        ),
                    ],
                )
            })
            .collect();
        let mut country = Country::builder()
            .id(1)
            .code("XX".to_string())
            .slug("xx".to_string())
            .name("Testland".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![league(10, 1), league(20, 2)]))
            .clubs(clubs)
            .build()
            .unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 8, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let ctx = GlobalContext::new(SimulationContext::new(date)).with_country(1);
        let country_info = FxHashMap::default();
        let market_map = MarketMap::default();
        let placement_reach = PlacementReachIndex::default();
        let world = WorldSnapshot {
            date,
            country_info: &country_info,
            indexes: None,
            world_pool: WorldPool::empty(),
            global_free_agents: &[],
            market_map: &market_map,
            placement_reach: &placement_reach,
        };

        let (matches, _) = country.simulate_build(&ctx, world);
        assert_eq!(matches.len(), 6, "both divisions play on opening day");

        let mut named: FxHashSet<u32> = FxHashSet::default();
        for m in &matches {
            for squad in [&m.home_squad, &m.away_squad] {
                for p in squad.main_squad.iter().chain(&squad.substitutes) {
                    assert!(
                        named.insert(p.id),
                        "player {} named in two squads on one day",
                        p.id
                    );
                }
            }
        }
    }

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    /// `count` clubs, first team only (club `n*100`, team `n*10`), all in
    /// one top division (league 10) with no schedule yet.
    pub(crate) fn country_with_first_teams(count: u32) -> Country {
        let clubs: Vec<Club> = (1..=count)
            .map(|n: u32| {
                club(
                    n * 100,
                    vec![team(
                        n * 10,
                        n * 100,
                        10,
                        TeamType::Main,
                        players(n * 1_000 + 1..=n * 1_000 + 18),
                    )],
                )
            })
            .collect();
        Country::builder()
            .id(1)
            .code("XX".to_string())
            .slug("xx".to_string())
            .name("Testland".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![league(10, 1)]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    /// Six first teams in one division, three Saturday rounds in September.
    pub(crate) fn september_country() -> Country {
        let mut country = country_with_first_teams(6);
        let tours = [(1, d(9, 12)), (2, d(9, 19)), (3, d(9, 26))]
            .into_iter()
            .map(|(num, day)| {
                let mut tour = ScheduleTour::new(num, 3);
                for (slot, (home, away)) in [(10, 20), (30, 40), (50, 60)].into_iter().enumerate() {
                    tour.items.push(ScheduleItem::new(
                        10,
                        "league-10".into(),
                        home,
                        away,
                        KickoffClock::at(day, CompetitionLevel::Senior, slot),
                        None,
                    ));
                }
                tour
            })
            .collect();
        country.leagues.leagues[0].schedule = Schedule { tours };
        country
    }

    #[test]
    fn a_cup_tie_the_day_after_a_midweek_league_round_moves_and_the_league_stays() {
        // Tuesday 22 September 2026 is a midweek league round; clubs 100 and
        // 300 meet in the cup the next day. No club here is in Europe.
        let mut country = country_with_first_teams(6);
        let league_day = d(9, 22);
        let mut league_tour = ScheduleTour::new(1, 3);
        for (slot, (home, away)) in [(10, 20), (30, 40), (50, 60)].into_iter().enumerate() {
            league_tour.items.push(ScheduleItem::new(
                10,
                "league-10".into(),
                home,
                away,
                KickoffClock::at(league_day, CompetitionLevel::Senior, slot),
                None,
            ));
        }
        country.leagues.leagues[0].schedule = Schedule {
            tours: vec![league_tour],
        };
        let mut cup = DomesticCup::new(league(20, 1));
        let mut cup_tour = ScheduleTour::new(1, 1);
        cup_tour.items.push(ScheduleItem::new(
            20,
            "cup-20".into(),
            10,
            30,
            KickoffClock::at(d(9, 23), CompetitionLevel::Senior, 0),
            None,
        ));
        cup.league.schedule = Schedule {
            tours: vec![cup_tour],
        };
        country.domestic_cup = Some(cup);

        country.rearrange_for_rest(&ContinentalCommitments::default(), d(9, 1));

        for item in country.leagues.leagues[0]
            .schedule
            .tours
            .iter()
            .flat_map(|t| &t.items)
        {
            assert_eq!(
                item.date.date(),
                league_day,
                "league fixture {} kept its day",
                item.id
            );
        }
        let tie = &country.domestic_cup.as_ref().unwrap().league.schedule.tours[0].items[0];
        assert_eq!(
            tie.date.date(),
            d(9, 25),
            "the nearest day with two clear days after Tuesday"
        );
        assert!(FixtureRest::is_rested(tie.date.date(), &[league_day]));
    }

    #[test]
    fn a_cup_round_drawn_the_day_after_the_last_one_waits_for_rest() {
        use crate::r#match::{Score, TeamScore};

        // Round two was played on Tuesday 25 August; round three is drawn
        // for the next day.
        let mut country = country_with_first_teams(6);
        let tie = |day: NaiveDate, played: bool| {
            let mut item = ScheduleItem::new(
                20,
                "cup-20".into(),
                10,
                30,
                KickoffClock::at(day, CompetitionLevel::Senior, 0),
                None,
            );
            if played {
                item.result = Some(Score {
                    home_team: TeamScore::new_with_score(10, 1),
                    away_team: TeamScore::new_with_score(30, 0),
                    details: Vec::new(),
                    home_shootout: 0,
                    away_shootout: 0,
                });
            }
            item
        };
        let mut cup = DomesticCup::new(league(20, 1));
        let mut round_two = ScheduleTour::new(2, 1);
        round_two.items.push(tie(d(8, 25), true));
        let mut round_three = ScheduleTour::new(3, 1);
        round_three.items.push(tie(d(8, 26), false));
        cup.league.schedule = Schedule {
            tours: vec![round_two, round_three],
        };
        country.domestic_cup = Some(cup);

        country.rearrange_for_rest(&ContinentalCommitments::default(), d(8, 26));

        let moved = &country.domestic_cup.as_ref().unwrap().league.schedule.tours[1].items[0];
        assert_eq!(
            moved.date.date(),
            d(8, 28),
            "the first day two clear days after the 25th"
        );
        let played = &country.domestic_cup.as_ref().unwrap().league.schedule.tours[0].items[0];
        assert_eq!(played.date.date(), d(8, 25), "a played tie never moves");
    }

    #[test]
    fn a_thursday_europa_league_club_plays_its_saturday_game_on_sunday() {
        use crate::continent::{CompetitionStage, ContinentalCompetitions, ContinentalMatch};
        use chrono::{Datelike, Timelike, Weekday};

        let mut country = september_country();
        let before: FxHashMap<String, NaiveDateTime> = country.leagues.leagues[0]
            .schedule
            .tours
            .iter()
            .flat_map(|t| &t.items)
            .map(|i| (i.id.clone(), i.date))
            .collect();

        // Club 100 (first team 10) plays in Europe on Thursday 17th.
        let mut competitions = ContinentalCompetitions::new();
        competitions.europa_league.matches.push(ContinentalMatch {
            home_team: 100,
            away_team: 9_999,
            date: d(9, 17),
            stage: CompetitionStage::GroupStage,
            match_id: String::new(),
            result: None,
        });
        let commitments = competitions.commitments(d(9, 1), d(9, 22));

        country.rearrange_for_rest(&commitments, d(9, 1));

        let items: Vec<&ScheduleItem> = country.leagues.leagues[0]
            .schedule
            .tours
            .iter()
            .flat_map(|t| &t.items)
            .collect();
        let moved = items
            .iter()
            .find(|i| {
                i.home_team_id == 10
                    && i.away_team_id == 20
                    && i.date.date() > d(9, 15)
                    && i.date.date() < d(9, 24)
            })
            .expect("the round-two fixture of the Europa League club");
        assert_eq!(
            moved.date.date(),
            d(9, 20),
            "Thursday tie, Sunday league game"
        );
        assert_eq!(moved.date.weekday(), Weekday::Sun);
        assert!(
            (12..=21).contains(&moved.date.hour()),
            "a Sunday kickoff: {}",
            moved.date
        );
        assert_eq!(
            before.get(&moved.id).map(|dt| dt.date()),
            Some(d(9, 19)),
            "same fixture, same id"
        );

        for item in items.iter().filter(|i| i.id != moved.id) {
            assert_eq!(
                Some(&item.date),
                before.get(&item.id),
                "{} was not the clashing fixture and must not move",
                item.id
            );
        }
    }

    fn run_process(
        country: &mut Country,
        date: NaiveDateTime,
        pending: CountryPendingState,
        match_results: Vec<MatchResult>,
    ) -> CountryResult {
        let ctx = GlobalContext::new(SimulationContext::new(date)).with_country(country.id);
        let country_info = FxHashMap::default();
        let market_map = MarketMap::default();
        let placement_reach = PlacementReachIndex::default();
        let world = WorldSnapshot {
            date,
            country_info: &country_info,
            indexes: None,
            world_pool: WorldPool::empty(),
            global_free_agents: &[],
            market_map: &market_map,
            placement_reach: &placement_reach,
        };
        country.simulate_process(ctx, world, pending, match_results)
    }

    fn played_match(id: &str, league_id: u32, slug: &str, home: u32, away: u32) -> MatchResult {
        use crate::r#match::Score;

        let score = Score::new(home, away);
        score.increment_home_goals();
        score.increment_away_goals();

        MatchResult {
            id: id.to_string(),
            league_id,
            league_slug: slug.to_string(),
            home_team_id: home,
            away_team_id: away,
            details: None,
            score,
            friendly: false,
        }
    }

    #[test]
    fn simulate_process_on_a_league_matchday_routes_and_stores_the_result() {
        let mut country = september_country();
        country.leagues.leagues[0].table = LeagueTable::new(&[10, 20, 30, 40, 50, 60]);
        let item = country.leagues.leagues[0].schedule.tours[0].items[0].clone();
        let date = KickoffClock::at(d(9, 12), CompetitionLevel::Senior, 0);
        let pending = CountryPendingState {
            leagues: vec![Some(LeaguePendingState {
                scheduled_matches: vec![LeagueMatch::from(item.clone())],
                table_result: LeagueTableResult {},
                new_season_started: false,
            })],
            cup: None,
            playoffs: Vec::new(),
            immediate_results: Vec::new(),
        };
        let result = run_process(
            &mut country,
            date,
            pending,
            vec![played_match(&item.id, 10, "league-10", 10, 20)],
        );

        assert_eq!(result.processed_match_results.len(), 1);
        let (league_id, matches) = &result.processed_match_results[0];
        assert_eq!(*league_id, 10);
        assert_eq!(matches.len(), 1);
        assert_eq!(country.leagues.leagues[0].table.rows[0].played, 1);
        assert_eq!(
            country.leagues.leagues[0].schedule.tours[0].items[0]
                .result
                .as_ref()
                .map(|score| (score.home_team.get(), score.away_team.get())),
            Some((1, 1))
        );
    }

    #[test]
    fn simulate_process_on_a_cup_day_pays_the_round_prize() {
        let mut country = country_with_first_teams(4);
        let mut cup = DomesticCup::new(league(20, 1));
        let date = KickoffClock::at(d(9, 12), CompetitionLevel::Senior, 0);
        let mut tour = ScheduleTour::new(1, 1);
        let item = ScheduleItem::new(20, "cup-20".into(), 10, 20, date, None);
        tour.items.push(item.clone());
        cup.league.schedule = Schedule { tours: vec![tour] };
        country.domestic_cup = Some(cup);
        let balance_before = country.clubs[0].finance.balance.balance;
        let pending = CountryPendingState {
            leagues: vec![None],
            cup: Some(LeaguePendingState {
                scheduled_matches: vec![LeagueMatch::from(item.clone())],
                table_result: LeagueTableResult {},
                new_season_started: false,
            }),
            playoffs: Vec::new(),
            immediate_results: Vec::new(),
        };
        let result = run_process(
            &mut country,
            date,
            pending,
            vec![played_match(&item.id, 20, "cup-20", 10, 20)],
        );

        assert_eq!(result.processed_match_results.len(), 1);
        assert_eq!(result.processed_match_results[0].0, 20);
        assert!(
            country.clubs[0].finance.balance.income_cup_prize > 0,
            "the tie winner banks the round prize"
        );
        assert!(country.clubs[0].finance.balance.balance > balance_before);
    }

    #[test]
    fn simulate_process_on_a_playoff_day_advances_the_series() {
        use crate::league::{PlayoffFormat, PlayoffSeries, PlayoffStage};

        let mut country = country_with_first_teams(4);
        let date = KickoffClock::at(d(9, 12), CompetitionLevel::Senior, 0);
        let mut playoff_league = league(30, 1);
        let mut tour = ScheduleTour::new(1, 1);
        let item = ScheduleItem::new(30, "playoff-30".into(), 10, 20, date, None);
        tour.items.push(item.clone());
        playoff_league.schedule = Schedule { tours: vec![tour] };
        let mut playoff = LeaguePlayoff::new(
            playoff_league,
            "Playoff".to_string(),
            vec![10],
            2,
            PlayoffFormat::SingleElimination,
            PlayoffStage::FullSeason,
        );
        playoff.series.push(PlayoffSeries {
            round: 1,
            bracket: 0,
            slot: 0,
            home_team_id: 10,
            away_team_id: 20,
            best_of: 1,
            home_wins: 0,
            away_wins: 0,
            neutral: false,
            games_scheduled: 1,
            last_game_date: Some(d(9, 12)),
        });
        country.playoffs = vec![playoff];
        let pending = CountryPendingState {
            leagues: vec![None],
            cup: None,
            playoffs: vec![Some(LeaguePendingState {
                scheduled_matches: vec![LeagueMatch::from(item.clone())],
                table_result: LeagueTableResult {},
                new_season_started: false,
            })],
            immediate_results: Vec::new(),
        };
        let result = run_process(
            &mut country,
            date,
            pending,
            vec![played_match(&item.id, 30, "playoff-30", 10, 20)],
        );

        assert_eq!(result.processed_match_results.len(), 1);
        assert_eq!(result.processed_match_results[0].0, 30);
        assert_eq!(country.playoffs[0].series[0].home_wins, 1);
    }

    #[test]
    fn simulate_process_on_a_no_fixture_day_keeps_the_calendar_untouched() {
        let mut country = september_country();
        let date = KickoffClock::at(d(9, 13), CompetitionLevel::Senior, 0);
        let before: Vec<(String, Option<(u8, u8)>)> = country.leagues.leagues[0]
            .schedule
            .tours
            .iter()
            .flat_map(|tour| &tour.items)
            .map(|item| {
                (
                    item.id.clone(),
                    item.result
                        .as_ref()
                        .map(|score| (score.home_team.get(), score.away_team.get())),
                )
            })
            .collect();
        let pending = CountryPendingState {
            leagues: vec![None],
            cup: None,
            playoffs: Vec::new(),
            immediate_results: Vec::new(),
        };
        let result = run_process(&mut country, date, pending, Vec::new());

        assert!(result.processed_match_results.is_empty());
        let after: Vec<(String, Option<(u8, u8)>)> = country.leagues.leagues[0]
            .schedule
            .tours
            .iter()
            .flat_map(|tour| &tour.items)
            .map(|item| {
                (
                    item.id.clone(),
                    item.result
                        .as_ref()
                        .map(|score| (score.home_team.get(), score.away_team.get())),
                )
            })
            .collect();
        assert_eq!(before, after);
        assert!(result.deferred_transfer_ops.is_some());
    }
}
