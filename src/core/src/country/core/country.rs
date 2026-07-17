use crate::MatchRuntime;
use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult, PlayerCollectionResult};
use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::country::core::builder::CountryBuilder;
use crate::country::national::NationalTeam;
use crate::league::LeagueCollection;
use crate::league::LeaguePendingState;
use crate::league::result::{
    ClubProcessCtx, CountryLookupIndex, CountryProcessCtx, DeferredContractInteraction,
    DeferredGlobalOps, StagedClubOps, WorldSnapshot,
};
use crate::r#match::Match;
use crate::r#match::MatchResult;
use crate::transfers::market::TransferMarket;
use crate::transfers::pipeline::PipelineProcessor;
use crate::{Club, ClubResult, Player, PlayerResult};
use chrono::{Datelike, NaiveDate};
use log::debug;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
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

        let teams_ids: Vec<(u32, Option<u32>)> = self
            .clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .map(|c| (c.id, c.league_id))
            .collect();

        let mut all_matches: Vec<Match> = Vec::new();
        let mut pending_leagues: Vec<Option<LeaguePendingState>> =
            Vec::with_capacity(self.leagues.leagues.len());
        let mut immediate_results: Vec<LeagueResult> = Vec::new();

        for league in &mut self.leagues.leagues {
            let league_team_ids: Vec<u32> = teams_ids
                .iter()
                .filter(|(_, league_id)| *league_id == Some(league.id))
                .map(|(id, _)| *id)
                .collect();
            let league_ctx = ctx.with_league(
                league.id,
                league.slug.clone(),
                &league_team_ids,
                league.reputation,
            );
            let output = league.simulate_build(&self.clubs, &league_ctx);
            all_matches.extend(output.matches);
            pending_leagues.push(output.pending);
            if let Some(r) = output.immediate {
                immediate_results.push(r);
            }
        }

        // Domestic cup. Runs alongside the league programme and joins
        // the same continent dispatch batch. Disjoint field borrows: the
        // cup is taken mutably while `self.clubs` is read immutably.
        let mut cup_pending: Option<LeaguePendingState> = None;
        let cup_ctx_args = self
            .domestic_cup
            .as_ref()
            .map(|c| (c.league.id, c.league.slug.clone(), c.league.reputation));
        if let Some((cup_id, cup_slug, cup_rep)) = cup_ctx_args {
            let cup_ctx = ctx.with_league(cup_id, cup_slug, &[], cup_rep);
            let cup = self.domestic_cup.as_mut().expect("cup checked above");
            let output = cup.simulate_build(&self.clubs, &cup_ctx);
            all_matches.extend(output.matches);
            if let Some(p) = output.pending {
                cup_pending = Some(p);
            }
            if let Some(r) = output.immediate {
                immediate_results.push(r);
            }
        }

        (
            all_matches,
            CountryPendingState {
                leagues: pending_leagues,
                cup: cup_pending,
                immediate_results,
            },
        )
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
        let country_name = self.name.clone();
        let current_date = ctx.simulation.date.date();

        // Group results by league_id so each league/cup gets its own
        // ordered slice.
        let mut by_league: HashMap<u32, Vec<MatchResult>> = HashMap::new();
        for mr in match_results {
            by_league.entry(mr.league_id).or_default().push(mr);
        }

        let mut league_results: Vec<LeagueResult> = pending.immediate_results;

        // Per-league post-match work. Pending vector is parallel to
        // `self.leagues.leagues` (same length, same order).
        let mut pending_iter = pending.leagues.into_iter();
        for league in &mut self.leagues.leagues {
            let slot = pending_iter.next().flatten();
            if let Some(p) = slot {
                let r = by_league.remove(&league.id).unwrap_or_default();
                let league_ctx =
                    ctx.with_league(league.id, league.slug.clone(), &[], league.reputation);
                let lr = league.simulate_process(r, p, &self.clubs, &league_ctx, current_date);
                league_results.push(lr);
            }
        }

        // Cup post-match work.
        if let Some(p) = pending.cup {
            let cup_ctx_args = self
                .domestic_cup
                .as_ref()
                .map(|c| (c.league.id, c.league.slug.clone(), c.league.reputation));
            if let Some((cup_id, cup_slug, cup_rep)) = cup_ctx_args {
                let cup_ctx = ctx.with_league(cup_id, cup_slug, &[], cup_rep);
                let cup = self.domestic_cup.as_mut().expect("cup checked above");
                let r = by_league.remove(&cup_id).unwrap_or_default();
                let cup_result = cup.simulate_process(r, p, &self.clubs, &cup_ctx, current_date);
                // Round prize money: every tie winner banks its round's
                // fee from the federation, scaled by the country's
                // broadcast market. Computed from the freshly stamped
                // bracket, then booked with the clubs list mutable again.
                let payouts = self
                    .domestic_cup
                    .as_ref()
                    .map(|c| {
                        c.round_prize_payouts(
                            cup_result.match_results.as_deref().unwrap_or(&[]),
                            &self.clubs,
                            self.economic_factors.tv_revenue_multiplier,
                        )
                    })
                    .unwrap_or_default();
                for (club_id, amount) in payouts {
                    if let Some(club) = self.clubs.iter_mut().find(|c| c.id == club_id) {
                        club.finance.balance.push_income_cup_prize(amount);
                    }
                }
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

        // Phase 1c (PARALLEL PER CLUB): drive each ClubResult's
        // sub-processors (finance, board, teams' players/staffs/training/
        // behaviour, academy) with a club-scoped view, parallel across
        // the country's clubs. The audited fan-out only touches its own
        // club's state; the rare escapes go through queues: global
        // cross-cuts (sacked staff, manager appointments) on a per-worker
        // `DeferredGlobalOps`, country-level writes (market listings,
        // interest clears) and loan-parent contract calls on a per-worker
        // `StagedClubOps` — all merged and applied serially in club
        // order below, so the drained sequences match the old serial
        // loop.
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
        for cr in &clubs_results {
            // Placeholder ClubResult to satisfy the existing
            // CountryResult.clubs surface — Phase C only needs the type.
            clubs_results_after.push(ClubResult::new(
                cr.club_id,
                ClubFinanceResult::new(),
                Vec::new(),
                BoardResult::new(),
                ClubAcademyResult::new(PlayerCollectionResult::new(Vec::new())),
            ));
        }
        let staged_parts: Vec<(DeferredGlobalOps, StagedClubOps)> = {
            // Disjoint field borrows: clubs mutable per worker, the
            // country-level context read-only and shared.
            let country_id = self.id;
            let market_strength = self.economic_factors.sponsorship_market_strength;
            let leagues_ref = &self.leagues;
            let cup_league_ref = self.domestic_cup.as_ref().map(|c| &c.league);
            self.clubs
                .par_iter_mut()
                .zip(clubs_results.into_par_iter())
                .enumerate()
                .map(|(club_idx, (club, cr))| {
                    // `simulate_clubs` collected results with a
                    // par_iter().map().collect() over this same vec, so
                    // order is guaranteed; nothing between there and here
                    // adds or removes clubs.
                    assert_eq!(
                        club.id, cr.club_id,
                        "Phase 1c: clubs/results order diverged"
                    );
                    let mut worker_deferred = DeferredGlobalOps::new();
                    let mut worker_staged = StagedClubOps::default();
                    let mut ctx_local = ClubProcessCtx {
                        club,
                        club_idx: club_idx as u16,
                        country_id,
                        date: world.date,
                        country_info_ref: world.country_info,
                        indexes_ref: world.indexes,
                        leagues_ref,
                        cup_league_ref,
                        sponsorship_market_strength: market_strength,
                        lookup: &lookup,
                        deferred: &mut worker_deferred,
                        staged: &mut worker_staged,
                    };
                    cr.process(&mut ctx_local, &mut SimulationResult::new());
                    (worker_deferred, worker_staged)
                })
                .collect()
        };
        // Serial post-pass, in club order. Listings and interest clears
        // land on the country market / plans exactly as the serial loop
        // wrote them mid-iteration; the relative order across clubs is
        // preserved, and nothing between a club's process and this drain
        // reads the affected state.
        let mut staged_contract_interactions: Vec<DeferredContractInteraction> = Vec::new();
        for (worker_deferred, worker_staged) in staged_parts {
            deferred.merge(worker_deferred);
            for listing in worker_staged.market_listings {
                self.transfer_market.add_listing(listing);
            }
            for player_id in worker_staged.interest_clears {
                PipelineProcessor::clear_player_interest(self, player_id);
            }
            staged_contract_interactions.extend(worker_staged.contract_interactions);
        }
        // Loan-parent contract interactions replay with full country
        // access — identical inputs to what the serial loop handed the
        // shared interaction fn, only the point in the tick moved.
        for interaction in staged_contract_interactions {
            let mut player_result = PlayerResult::new(interaction.player_id);
            player_result.contract.no_contract = interaction.no_contract;
            player_result.contract.want_improve_contract = interaction.want_improve_contract;
            player_result.contract.want_extend_contract = interaction.want_extend_contract;
            let mut ctx_local = CountryProcessCtx {
                country: self,
                date: world.date,
                country_info_ref: world.country_info,
                indexes_ref: world.indexes,
                deferred: &mut deferred,
                lookup: Some(&lookup),
            };
            ClubResult::process_player_contract_interaction(
                &player_result,
                &mut ctx_local,
                interaction.employing_club_id,
            );
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
