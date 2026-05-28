use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult, PlayerCollectionResult};
use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::country::core::builder::CountryBuilder;
use crate::country::national::NationalTeam;
use crate::league::LeagueCollection;
use crate::league::result::{
    CountryLookupIndex, CountryProcessCtx, DeferredGlobalOps, WorldSnapshot,
};
use crate::r#match::MatchResult;
use crate::transfers::market::TransferMarket;
use crate::{Club, ClubResult, Player};
use chrono::{Datelike, NaiveDate};
use log::debug;
use rayon::iter::ParallelIterator;
use rayon::prelude::IntoParallelRefMutIterator;

use crate::SimulationResult;
use crate::Team;
use crate::country::{
    CountryEconomicFactors, CountryGeneratorData, CountryRegulations, CountrySettings,
    InternationalCompetition, MediaCoverage,
};
use crate::league::DomesticCup;
use crate::league::League;
use crate::league::LeagueResult;
use crate::league::LeagueTableResult;
use std::collections::HashMap;

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
    pub regulations: CountryRegulations,

    pub retired_players: Vec<Player>,

    /// `start_year` of the most recent season for which the per-player
    /// statistics snapshot has fired. `None` before the first
    /// snapshot runs. Used as a watermark so the snapshot can catch
    /// up on missed seasons (any league failing the
    /// `new_season_started` gate would otherwise leave a gap in
    /// every player's career history table). Updated by
    /// `snapshot_player_season_statistics` once for each season it
    /// processes.
    pub last_snapshotted_season_year: Option<u16>,
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

    pub(crate) fn simulate(
        &mut self,
        ctx: GlobalContext<'_>,
        world: WorldSnapshot<'_>,
    ) -> CountryResult {
        let country_name = self.name.clone();

        debug!(
            "Simulating country: {} (Reputation: {})",
            country_name, self.reputation
        );

        // Phase 1: League Competitions
        let mut league_results = self.leagues.simulate(&self.clubs, &ctx);

        // Phase 1a: Domestic cup. Runs alongside the league programme and
        // appends its day's results to `league_results` so the same Phase
        // 1b `process_local` fan-out records cup stats, morale, discipline
        // and reputation — routed into the cup buckets because the inner
        // league carries `is_cup = true`. Disjoint field borrows: the cup
        // is read mutably while `clubs` is read immutably.
        {
            let clubs = &self.clubs;
            if let Some(cup) = self.domestic_cup.as_mut() {
                let cup_ctx = ctx.with_league(
                    cup.league.id,
                    cup.league.slug.clone(),
                    &[],
                    cup.league.reputation,
                );
                let cup_result = cup.simulate(clubs, &cup_ctx);
                league_results.push(cup_result);
            }
        }

        // Bridge between league and club passes: refresh each team's
        // fixture window from the (now-current) league schedule so
        // training in Phase 2 can react to real calendar distance to
        // the next match instead of guessing a Saturday fixture.
        self.refresh_team_fixture_windows(ctx.simulation.date.date());

        // Phase 2: Club Operations (with economic factors)
        // National team call-ups are handled at the continent level (cross-country visibility)
        let ctx = {
            let mut c = ctx;
            if let Some(ref mut country_ctx) = c.country {
                country_ctx.tv_revenue_multiplier = self.economic_factors.tv_revenue_multiplier;
                country_ctx.sponsorship_market_strength =
                    self.economic_factors.sponsorship_market_strength;
                country_ctx.stadium_attendance_factor =
                    self.economic_factors.stadium_attendance_factor;
                country_ctx.price_level = self.settings.pricing.price_level;
                country_ctx.reputation = self.reputation;
            }
            c
        };
        let mut clubs_results = self.simulate_clubs(&ctx);

        // Per-country local result processing — used to run serially in
        // Phase C after the parallel continent pass joined. Moved into
        // `Country::simulate` so it runs inside the existing
        // `countries.par_iter_mut()` (see `continent.rs`), parallel
        // across every country in the world. Only the truly
        // country-local pieces are here; cross-country work (transfer
        // market, loan returns, club result fan-out into the global
        // match store) stays in `CountryResult::process` until that
        // work is sharded too.
        let current_date = ctx.simulation.date.date();
        CountryResult::simulate_media_coverage(self, &league_results);
        // End-of-period must run BEFORE downstream Phase C work that
        // reads player rosters — it retires players and triggers
        // season awards. Order-equivalent to its old position in Phase
        // C (which was before league_result.process). It uses
        // club_results so it runs after `simulate_clubs`.
        CountryResult::process_end_of_period(self, current_date, &clubs_results);
        CountryResult::update_country_reputation(self);
        CountryResult::simulate_international_competitions(self, current_date);
        CountryResult::update_economic_factors(self, current_date);
        if self.season_dates().is_off_season(current_date) {
            CountryResult::simulate_preseason_activities(self, current_date);
        }

        // Phase 1b (NEW PARALLEL PATH): drive each LeagueResult's per-match
        // stat / morale / discipline fan-out through `process_local`
        // here, inside the parallel `countries.par_iter_mut()`. The
        // existing `LeagueResult::process` (the global path) stays in
        // place for `process_cup_match` (continental cups cross country
        // boundaries). Match results consumed by `process_local` are
        // captured into `processed_match_results` and folded back into
        // the LeagueResult so the simulator's serial Phase C drains
        // them into the world `SimulationResult`.
        let mut deferred = DeferredGlobalOps::new();
        let mut processed: Vec<(u32, Vec<MatchResult>)> = Vec::new();
        let mut league_results_after = Vec::with_capacity(league_results.len());
        // Build the per-country positional index once and share it
        // across both result-processing loops. The fan-out
        // (`TeamBehaviourResult::process` etc.) calls
        // `data.player(id)` / `player_mut(id)` thousands of times per
        // tick — without this index each call linear-scans every team
        // and player in the country, dominating wall-time on big
        // countries. Drop before `simulate_transfer_market_local`
        // since the transfer pipeline mutates rosters.
        let lookup = CountryLookupIndex::build(self);
        for lr in league_results {
            let league_id = lr.league_id;
            if lr.match_results.is_some() {
                let mut ctx_local = CountryProcessCtx {
                    country: self,
                    date: world.date,
                    country_info_ref: world.country_info,
                    indexes_ref: world.indexes,
                    deferred: &mut deferred,
                    lookup: Some(&lookup),
                };
                let mut out: Vec<MatchResult> = Vec::new();
                // Take match_results out so process_local can consume
                // them, then we restore the shell of the LeagueResult.
                let new_season_started = lr.new_season_started;
                lr.process_local(&mut ctx_local, &mut out);
                processed.push((league_id, out));
                // Rebuild a stripped LeagueResult — `match_results`
                // already fanned out, so the serial Phase C only needs
                // to push them into `SimulationResult.match_results`.
                // `LeagueTableResult` is a unit-like marker so there's
                // no state to copy from the consumed result.
                let mut placeholder = LeagueResult::new(league_id, LeagueTableResult {});
                placeholder.new_season_started = new_season_started;
                league_results_after.push(placeholder);
            } else {
                league_results_after.push(lr);
            }
        }

        // Phase 1b.1: domestic cup winner fan-out. Runs strictly AFTER
        // every LeagueResult has been through `process_local` for the
        // day — that's what writes the final-day cup appearance rows on
        // each player's `cup_statistics_by_competition`, and eligibility
        // hinges on those. Idempotent across ticks via the cup's own
        // `award_emitted_*` markers, so a no-op on every non-final day.
        CountryResult::process_domestic_cup_winner_awards(self, current_date);

        // Phase 1c (NEW PARALLEL PATH): drive each ClubResult's
        // sub-processors (finance, board, teams' players/staffs/training/
        // behaviour, academy) here, inside the parallel
        // `countries.par_iter_mut()`. The contracts / discipline /
        // training / morale fan-out is now country-local — global
        // cross-cuts (sacked staff joining the global free-agent pool,
        // pending manager appointments) are queued on
        // `deferred_global_ops` and drained serially in Phase C.
        let mut clubs_results_after: Vec<ClubResult> = Vec::with_capacity(clubs_results.len());
        // Collect academy transfers up front while we still own the
        // ClubResult slice — they're appended to the country's transfer
        // history below in a single pass after the parallel work.
        let mut academy_transfers = Vec::new();
        for cr in &clubs_results {
            if !cr.academy_transfers.is_empty() {
                academy_transfers.extend(cr.academy_transfers.iter().cloned());
            }
        }
        // Aged-out academy players need to flow into the global free
        // agent pool. Each club's released cohort is moved (not cloned)
        // into `deferred.free_agent_players`; Phase C extends them onto
        // `data.free_agents`.
        for cr in &mut clubs_results {
            if !cr.academy_released_players.is_empty() {
                deferred
                    .free_agent_players
                    .extend(std::mem::take(&mut cr.academy_released_players));
            }
        }
        for cr in clubs_results {
            let club_id = cr.club_id;
            let mut ctx_local = CountryProcessCtx {
                country: self,
                date: world.date,
                country_info_ref: world.country_info,
                indexes_ref: world.indexes,
                deferred: &mut deferred,
                lookup: Some(&lookup),
            };
            // ClubResult::process consumes self; we don't need the
            // returned shell after applying. Reconstruct a stripped
            // placeholder for Phase C's iteration cost-of-bookkeeping.
            // (Phase C no longer calls ClubResult::process — the shell
            // is just for typing.)
            cr.process(&mut ctx_local, &mut SimulationResult::new());
            // Keep a placeholder ClubResult to satisfy the existing
            // CountryResult.clubs surface. The data Phase C cares about
            // (academy_transfers, pending_ai_requests) were already
            // extracted upstream — we just need the type.
            clubs_results_after.push(ClubResult::new(
                club_id,
                ClubFinanceResult::new(),
                Vec::new(),
                BoardResult::new(),
                ClubAcademyResult::new(PlayerCollectionResult::new(Vec::new())),
            ));
        }
        // Push academy transfers to country transfer history here — no
        // longer needs Phase C since we own &mut self.
        if !academy_transfers.is_empty() {
            for transfer in academy_transfers {
                self.transfer_market.transfer_history.push(transfer);
            }
        }

        // Drop the lookup index before the transfer market runs — the
        // pipeline mutates rosters (signings, free agents) and the
        // index would go stale.
        drop(lookup);

        // Phase 1d (NEW PARALLEL PATH): run the country-local transfer
        // market pipeline here. The heavy work — scouting, negotiations,
        // squad evaluation, recruitment meetings, shadow reports —
        // takes &mut Country already, so it lifts cleanly into Phase A.
        // Cross-country writes (sweeps, data.free_agents mutation,
        // transfer execution, foreign negotiations) go through the
        // returned DeferredTransferOps, drained by Phase C.
        let transfer_ops = CountryResult::simulate_transfer_market_local(
            self,
            current_date,
            world.world_pool,
            world.global_free_agents,
        );

        // Stash the processed matches and any deferred global ops on
        // CountryResult so the serial Phase C can apply them.
        let mut country_result =
            CountryResult::new(self.id, league_results_after, clubs_results_after);
        country_result.processed_match_results = processed;
        country_result.deferred_global_ops = deferred;
        country_result.deferred_transfer_ops = Some(transfer_ops);

        debug!("Country {} simulation complete", country_name);

        country_result
    }

    fn simulate_clubs(&mut self, ctx: &GlobalContext<'_>) -> Vec<ClubResult> {
        // Build team_id → (position, league_size, total_matches, matches_played, tier, league_rep)
        let mut team_league_info: HashMap<u32, (u8, u8, u8, u8, u8, u16)> = HashMap::new();
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

        self.clubs
            .par_iter_mut()
            .map(|club| {
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

    /// Walk every league's schedule and write each team's next four
    /// upcoming + last four recent competitive fixture dates into
    /// `Team::fixture_window`. Skips friendly leagues — those don't
    /// drive the MD-1/MD-2 training rhythm. Cheap: scales O(fixtures)
    /// per league once a tick.
    fn refresh_team_fixture_windows(&mut self, today: NaiveDate) {
        use std::collections::HashMap;
        let mut upcoming_map: HashMap<u32, Vec<NaiveDate>> = HashMap::new();
        let mut recent_map: HashMap<u32, Vec<NaiveDate>> = HashMap::new();
        // Cup ties count toward fixture congestion just like league games,
        // so fold the cup's bracket into the same window maps.
        let cup_schedule = self.domestic_cup.as_ref().map(|c| &c.league.schedule);
        let schedules = self
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| &l.schedule)
            .chain(cup_schedule);
        for schedule in schedules {
            for tour in &schedule.tours {
                for item in &tour.items {
                    let d = item.date.date();
                    if d > today && item.result.is_none() {
                        upcoming_map.entry(item.home_team_id).or_default().push(d);
                        upcoming_map.entry(item.away_team_id).or_default().push(d);
                    } else if d <= today && item.result.is_some() {
                        recent_map.entry(item.home_team_id).or_default().push(d);
                        recent_map.entry(item.away_team_id).or_default().push(d);
                    }
                }
            }
        }
        for club in &mut self.clubs {
            for team in &mut club.teams.teams {
                let mut up = upcoming_map.remove(&team.id).unwrap_or_default();
                up.sort_unstable();
                up.truncate(4);
                let mut rec = recent_map.remove(&team.id).unwrap_or_default();
                rec.sort_unstable_by(|a, b| b.cmp(a));
                rec.truncate(4);
                team.fixture_window.refreshed = Some(today);
                team.fixture_window.upcoming = up;
                team.fixture_window.recent = rec;
            }
        }
    }
}
