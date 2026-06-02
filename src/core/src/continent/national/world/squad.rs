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
use std::sync::atomic::{AtomicU64, Ordering};

use super::lookups::{country_lookup, country_lookup_mut};
use crate::continent::Continent;
use crate::r#match::MatchSquad;
use crate::{Club, Country, NationalSelectionPolicy, NationalTeam, NationalTeamLevel};
use std::collections::HashSet;

static EMERGENCY_CALLUPS: AtomicU64 = AtomicU64::new(0);

/// Process-global counter of world-squad emergency call-ups. Bumps every
/// time [`build_world_match_squad`] has to repopulate an empty squad on the
/// fly; healthy runs see [`EmergencyCallupMetrics::total`] stay flat.
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

/// World-aware squad builder. Searches every club in every continent
/// so foreign-based selected players (e.g. a Brazilian at a Spanish
/// club) are reachable from their nation's squad regardless of which
/// continent the match is being played for.
///
/// Triggers the emergency call-up path if the country has neither a
/// real nor a synthetic squad — which only happens before the first
/// `process_world_national_team_callups` of the run, or if a fixture
/// has somehow been scheduled outside the regular break/tournament
/// window. Frequent emergency call-ups indicate a scheduling bug;
/// [`EmergencyCallupMetrics`] exposes that to operators.
pub fn build_world_match_squad(
    continents: &mut [Continent],
    country_id: u32,
    date: NaiveDate,
) -> Option<MatchSquad> {
    build_world_match_squad_for_level(continents, country_id, date, NationalTeamLevel::Senior)
}

/// Level-aware world squad builder. Senior fixtures pull from
/// `country.national_team`, U21 fixtures from `country.u21_national_team`.
/// The emergency call-up path respects the level so an empty U21 squad is
/// repopulated with the U21 selection policy, not the senior one.
pub fn build_world_match_squad_for_level(
    continents: &mut [Continent],
    country_id: u32,
    date: NaiveDate,
    level: NationalTeamLevel,
) -> Option<MatchSquad> {
    let needs_emergency = country_lookup(continents, country_id).is_some_and(|c| {
        let team = team_for_level(c, level);
        team.squad.is_empty() && team.generated_squad.is_empty()
    });

    if needs_emergency {
        emergency_world_callup(continents, country_id, date, level);
    }

    let all_clubs: Vec<&Club> = continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .flat_map(|c| c.clubs.iter())
        .collect();

    let country = country_lookup(continents, country_id)?;
    Some(team_for_level(country, level).build_match_squad_from_refs(&all_clubs, date))
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
fn emergency_world_callup(
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

    let mut candidates_by_country = NationalTeam::collect_all_candidates_by_country_with_policy(
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
