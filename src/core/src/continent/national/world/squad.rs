//! World-aware national-team squad building.
//!
//! Squad selection runs at world scope (see
//! [`crate::SimulatorData::process_world_national_team_callups`]); the
//! match-day builder here mirrors that scope so a player picked while
//! at a foreign club is reachable when the match actually fires.
//!
//! The emergency path triggers only when a fixture fires before the
//! regular world-level call-up has populated the squad —
//! [`EmergencyCallupMetrics`] exposes that to operators so a drift
//! between the schedule and the call-up window can be detected.

use chrono::NaiveDate;
use log::warn;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

use super::lookups::{country_lookup, country_lookup_mut};
use crate::continent::Continent;
use crate::r#match::MatchSquad;
use crate::{Club, Country, NationalSelectionPolicy, NationalTeam, NationalTeamLevel};
use std::collections::HashSet;

static EMERGENCY_CALLUPS: AtomicU64 = AtomicU64::new(0);

/// Process-global counter of world-squad emergency call-ups. Bumps every
/// time [`NationalSquadBuilder::build`] has to repopulate an empty squad on
/// the fly; healthy runs see [`EmergencyCallupMetrics::total`] stay flat.
pub struct EmergencyCallupMetrics;

impl EmergencyCallupMetrics {
    /// Total emergency call-ups since process start.
    pub fn total() -> u64 {
        EMERGENCY_CALLUPS.load(Ordering::Relaxed)
    }

    /// Record one emergency call-up. Invoked from the emergency path.
    pub fn record() {
        EMERGENCY_CALLUPS.fetch_add(1, Ordering::Relaxed);
    }
}

/// World-aware national-team squad construction. Searches every club in
/// every continent so foreign-based selected players (e.g. a Brazilian at
/// a Spanish club) are reachable from their nation's squad regardless of
/// which continent the match is being played for.
///
/// The single-shot [`build`](Self::build) / [`build_for_level`](Self::build_for_level)
/// entry points rebuild the world-clubs pool themselves. When building a
/// whole matchday of squads at once, prefer resolving emergencies up front
/// with [`needs_emergency`](Self::needs_emergency) /
/// [`emergency_callup`](Self::emergency_callup), collecting the pool once
/// with [`collect_world_clubs`](Self::collect_world_clubs), and fanning
/// [`build_from_clubs`](Self::build_from_clubs) across the fixtures.
pub struct NationalSquadBuilder;

impl NationalSquadBuilder {
    /// Senior world squad builder. Triggers the emergency call-up path if
    /// the country has neither a real nor a synthetic squad — which only
    /// happens before the first `process_world_national_team_callups` of
    /// the run, or if a fixture has somehow been scheduled outside the
    /// regular break/tournament window. Frequent emergency call-ups
    /// indicate a scheduling bug; [`EmergencyCallupMetrics`] exposes that.
    pub fn build(
        continents: &mut [Continent],
        country_id: u32,
        date: NaiveDate,
    ) -> Option<MatchSquad> {
        Self::build_for_level(continents, country_id, date, NationalTeamLevel::Senior)
    }

    /// Level-aware world squad builder. Senior fixtures pull from
    /// `country.national_team`, U21 fixtures from `country.u21_national_team`.
    /// The emergency call-up path respects the level so an empty U21 squad
    /// is repopulated with the U21 selection policy, not the senior one.
    pub fn build_for_level(
        continents: &mut [Continent],
        country_id: u32,
        date: NaiveDate,
        level: NationalTeamLevel,
    ) -> Option<MatchSquad> {
        if Self::needs_emergency(continents, country_id, level) {
            Self::emergency_callup(continents, country_id, date, level);
        }

        let all_clubs = Self::collect_world_clubs(continents);
        Self::build_from_clubs(continents, &all_clubs, country_id, date, level)
    }

    /// Build match squads for a whole matchday of fixtures at once.
    ///
    /// Resolves any emergency call-ups serially up front — that path
    /// mutates world state (repopulates an empty squad, fans Int flags
    /// across the world) so it cannot run under the parallel build — then
    /// builds every squad in parallel against ONE shared world-clubs
    /// snapshot instead of re-walking every club in the world per squad.
    ///
    /// Each fixture is `(home_country_id, away_country_id, level,
    /// is_knockout)`. The return carries the fixture's original index (so
    /// callers can map engine results back) and its `is_knockout` flag.
    /// Output order matches input order; a fixture whose home OR away squad
    /// can't be built is dropped. The result is identical to building each
    /// squad one-by-one with [`build`](Self::build) — no shared mutation
    /// happens during the parallel pass, so fixture order can't matter.
    pub fn build_fixture_squads(
        continents: &mut [Continent],
        fixtures: &[(u32, u32, NationalTeamLevel, bool)],
        date: NaiveDate,
    ) -> Vec<(usize, MatchSquad, MatchSquad, bool)> {
        // Phase 1 (serial, rare): emergencies mutate `continents`. Home
        // then away, per fixture in order, firing only while the squad is
        // still empty — the same trigger order as the old per-fixture path.
        for &(home, away, level, _) in fixtures {
            for country_id in [home, away] {
                if Self::needs_emergency(continents, country_id, level) {
                    Self::emergency_callup(continents, country_id, date, level);
                }
            }
        }

        // Phase 2 (parallel, read-only): one shared world-clubs snapshot,
        // then build both squads per fixture across the rayon pool.
        let all_clubs = Self::collect_world_clubs(continents);
        fixtures
            .par_iter()
            .enumerate()
            .filter_map(|(idx, &(home, away, level, is_knockout))| {
                let home_squad = Self::build_from_clubs(continents, &all_clubs, home, date, level)?;
                let away_squad = Self::build_from_clubs(continents, &all_clubs, away, date, level)?;
                Some((idx, home_squad, away_squad, is_knockout))
            })
            .collect()
    }

    /// Does `country_id`'s national team at `level` have no squad yet? True
    /// means an emergency call-up must run (which mutates state) before the
    /// squad can be built. Split out so a matchday batch can resolve every
    /// emergency serially, up front, and then build all squads in parallel
    /// with read-only world access.
    fn needs_emergency(
        continents: &[Continent],
        country_id: u32,
        level: NationalTeamLevel,
    ) -> bool {
        country_lookup(continents, country_id).is_some_and(|c| {
            let team = Self::team_for_level(c, level);
            team.squad.is_empty() && team.generated_squad.is_empty()
        })
    }

    /// Flatten every club in the world into one `&Club` candidate pool for
    /// world-aware national selection. Building this walks every club in
    /// every country in every continent, so callers that build many squads
    /// in a batch (the national-competition matchday) should build it ONCE
    /// and share it via [`build_from_clubs`](Self::build_from_clubs) rather
    /// than re-walking the whole world per squad.
    fn collect_world_clubs(continents: &[Continent]) -> Vec<&Club> {
        continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .flat_map(|c| c.clubs.iter())
            .collect()
    }

    /// Read-only squad build from a pre-collected world-clubs pool. The
    /// caller is responsible for having already resolved any emergency
    /// call-up (that path mutates squads); this one only reads, so it is
    /// safe to fan out across fixtures on the rayon pool.
    fn build_from_clubs(
        continents: &[Continent],
        all_clubs: &[&Club],
        country_id: u32,
        date: NaiveDate,
        level: NationalTeamLevel,
    ) -> Option<MatchSquad> {
        let country = country_lookup(continents, country_id)?;
        Some(Self::team_for_level(country, level).build_match_squad_from_refs(all_clubs, date))
    }

    /// Pick the national team at `level` from a country.
    fn team_for_level(country: &Country, level: NationalTeamLevel) -> &NationalTeam {
        match level {
            NationalTeamLevel::Senior => &country.national_team,
            NationalTeamLevel::Under21 => &country.u21_national_team,
        }
    }

    /// World-aware emergency call-up. Builds a candidate pool from every
    /// country in every continent, applies it to the target country, and
    /// fans the Int status out across the whole world (so a foreign-based
    /// selectee at a Spanish club ends up flagged correctly even when
    /// their nation sits on a different continent).
    fn emergency_callup(
        continents: &mut [Continent],
        country_id: u32,
        date: NaiveDate,
        level: NationalTeamLevel,
    ) {
        EmergencyCallupMetrics::record();

        let country_name = country_lookup(continents, country_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        warn!(
            "Emergency national-team call-up ({:?}) for {} on {} — squad empty before fixture; world-level break-start may have been missed",
            level, country_name, date
        );

        let country_ids: Vec<(u32, String)> = continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .map(|c| (c.id, c.name.clone()))
            .collect();

        let policy = match level {
            NationalTeamLevel::Senior => NationalSelectionPolicy::senior(),
            NationalTeamLevel::Under21 => NationalSelectionPolicy::under21(),
        };

        let mut candidates_by_country =
            NationalTeam::collect_all_candidates_by_country_with_policy(
                continents.iter().flat_map(|c| c.countries.iter()),
                date,
                &policy,
            );
        let mut candidates = candidates_by_country
            .remove(&country_id)
            .unwrap_or_default();

        // For U21, keep the youth pool disjoint from the current senior squad.
        if level == NationalTeamLevel::Under21 {
            let senior_selected: HashSet<u32> = continents
                .iter()
                .flat_map(|c| c.countries.iter())
                .flat_map(|c| c.national_team.squad.iter().map(|sp| sp.player_id))
                .collect();
            candidates.retain(|c| !senior_selected.contains(&c.player_id));
        }

        if let Some(country) = country_lookup_mut(continents, country_id) {
            let name = country.name.clone();
            let rep = country.reputation;
            let cid = country.id;
            let team = match level {
                NationalTeamLevel::Senior => &mut country.national_team,
                NationalTeamLevel::Under21 => &mut country.u21_national_team,
            };
            team.country_name = name;
            team.reputation = rep;
            team.call_up_squad_with_policy(candidates, date, cid, &country_ids, &policy);
        }

        match level {
            NationalTeamLevel::Senior => {
                NationalTeam::apply_callup_statuses_across_world(continents, date)
            }
            NationalTeamLevel::Under21 => {
                NationalTeam::apply_u21_callup_statuses_across_world(continents, date)
            }
        }
    }
}
