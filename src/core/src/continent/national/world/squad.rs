//! World-aware national-team squad building.
//!
//! Squad selection runs at world scope (see
//! [`crate::SimulatorData::process_world_national_team_callups`]); the
//! match-day builder here mirrors that scope so a player picked while
//! at a foreign club is reachable when the match actually fires.
//!
//! The emergency path triggers only when a fixture fires before the
//! regular world-level call-up has populated the squad — the
//! [`EMERGENCY_CALLUPS`] counter exposes that to operators so a drift
//! between the schedule and the call-up window can be detected.

use chrono::NaiveDate;
use log::warn;
use std::sync::atomic::{AtomicU64, Ordering};

use super::lookups::{country_lookup, country_lookup_mut};
use crate::continent::Continent;
use crate::r#match::MatchSquad;
use crate::{Club, NationalTeam};

static EMERGENCY_CALLUPS: AtomicU64 = AtomicU64::new(0);

/// Total emergency call-ups since process start. Bumps every time
/// [`build_world_match_squad`] has to repopulate an empty squad on the
/// fly; healthy runs see this stay flat.
pub fn emergency_callups_total() -> u64 {
    EMERGENCY_CALLUPS.load(Ordering::Relaxed)
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
/// window. Frequent emergency call-ups indicate a scheduling bug; the
/// [`EMERGENCY_CALLUPS`] counter exposes that to operators.
pub fn build_world_match_squad(
    continents: &mut [Continent],
    country_id: u32,
    date: NaiveDate,
) -> Option<MatchSquad> {
    let needs_emergency = country_lookup(continents, country_id).is_some_and(|c| {
        c.national_team.squad.is_empty() && c.national_team.generated_squad.is_empty()
    });

    if needs_emergency {
        emergency_world_callup(continents, country_id, date);
    }

    let all_clubs: Vec<&Club> = continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .flat_map(|c| c.clubs.iter())
        .collect();

    let country = country_lookup(continents, country_id)?;
    Some(
        country
            .national_team
            .build_match_squad_from_refs(&all_clubs),
    )
}

/// World-aware emergency call-up. Builds a candidate pool from every
/// country in every continent, applies it to the target country, and
/// fans the Int status out across the whole world (so a foreign-based
/// selectee at a Spanish club ends up flagged correctly even when
/// their nation sits on a different continent).
fn emergency_world_callup(continents: &mut [Continent], country_id: u32, date: NaiveDate) {
    EMERGENCY_CALLUPS.fetch_add(1, Ordering::Relaxed);

    let country_name = country_lookup(continents, country_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    warn!(
        "Emergency national-team call-up for {} on {} — squad empty before fixture; world-level break-start may have been missed",
        country_name, date
    );

    let country_ids: Vec<(u32, String)> = continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .map(|c| (c.id, c.name.clone()))
        .collect();

    let mut candidates_by_country = NationalTeam::collect_all_candidates_by_country(
        continents.iter().flat_map(|c| c.countries.iter()),
        date,
    );
    let candidates = candidates_by_country
        .remove(&country_id)
        .unwrap_or_default();

    if let Some(country) = country_lookup_mut(continents, country_id) {
        country.national_team.country_name = country.name.clone();
        country.national_team.reputation = country.reputation;
        let cid = country.id;
        country
            .national_team
            .call_up_squad(candidates, date, cid, &country_ids);
    }

    NationalTeam::apply_callup_statuses_across_world(continents, date);
}
