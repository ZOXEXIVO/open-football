//! Continental qualifier orchestrator.
//!
//! Replaces the old per-continent simulation pass. Walks every
//! continent's `national_team_competitions`, plays today's fixtures
//! through the engine pool, and fans the post-match writes (caps,
//! goals, reputation, Elo, schedule, MatchResult) out across the
//! entire world via the helpers in [`super::stats`].
//!
//! Lifted out of the parallel continent phase because squad
//! construction needs read access to clubs in every continent —
//! something a continent-local pass cannot provide.

use chrono::NaiveDate;
use log::info;
use std::collections::HashMap;

use super::lookups::{world_country_name, world_country_reputation};
use super::squad::build_world_match_squad;
use super::stats::{
    apply_world_elo, apply_world_international_stats, record_world_country_schedule,
};
use crate::continent::Continent;
use crate::continent::national::NationalCompetitionFixture;
use crate::r#match::{MatchResult, MatchSquad};

/// Pair a continent index with one of its national-competition
/// fixtures so the orchestrator can fan match results back to the
/// right continent's competition tracker after engine play.
struct StampedFixture {
    continent_idx: usize,
    fixture: NationalCompetitionFixture,
}

/// World-aware national-competition simulation.
///
/// * fixture collection walks every continent
/// * squad building uses world-wide club visibility
/// * stats / Elo / schedule writes fan out to every continent
/// * MatchResults are stashed via a single helper that uses the
///   `"international"` league slug so the match-detail page can find
///   them
///
/// The same [`apply_world_international_stats`], [`apply_world_elo`]
/// and [`record_world_country_schedule`] helpers are reused by
/// [`super::tournament::apply_global_tournament_result`] so World Cup
/// matches see exactly the same downstream side effects as
/// continental qualifiers.
pub fn simulate_world_national_competitions(
    continents: &mut [Continent],
    date: NaiveDate,
) -> Vec<MatchResult> {
    advance_competition_cycles(continents, date);

    let stamped = collect_todays_fixtures(continents, date);

    if stamped.is_empty() {
        run_phase_transitions(continents);
        return Vec::new();
    }

    let prepared = build_squads(continents, &stamped, date);
    let engine_results = crate::match_engine_pool().play_squads_with_knockout(prepared);

    let mut collected: Vec<MatchResult> = Vec::with_capacity(engine_results.len());
    for (stamp_idx, raw) in engine_results {
        if let Some(match_result) = apply_match_outcome(continents, &stamped[stamp_idx], raw, date)
        {
            collected.push(match_result);
        }
    }

    run_phase_transitions(continents);
    collected
}

/// Per-continent: refresh competition cycles. Sorts countries by
/// reputation descending — feeds the qualifying-group draw which uses
/// pots ordered by national strength.
fn advance_competition_cycles(continents: &mut [Continent], date: NaiveDate) {
    for continent in continents.iter_mut() {
        let continent_id = continent.id;
        let mut country_ids_by_rep: Vec<(u32, u16)> = continent
            .countries
            .iter()
            .map(|c| (c.id, c.reputation))
            .collect();
        country_ids_by_rep.sort_by(|a, b| b.1.cmp(&a.1));
        let sorted_ids: Vec<u32> = country_ids_by_rep.iter().map(|(id, _)| *id).collect();
        continent
            .national_team_competitions
            .check_new_cycles(date, &sorted_ids, continent_id);
    }
}

/// Snapshot today's fixtures across every continent into a flat list,
/// stamped with the originating continent index so results can be
/// fanned back correctly.
fn collect_todays_fixtures(continents: &[Continent], date: NaiveDate) -> Vec<StampedFixture> {
    let mut stamped: Vec<StampedFixture> = Vec::new();
    for (idx, continent) in continents.iter().enumerate() {
        for fixture in continent
            .national_team_competitions
            .get_todays_matches(date)
        {
            stamped.push(StampedFixture {
                continent_idx: idx,
                fixture,
            });
        }
    }
    stamped
}

/// Build home/away MatchSquads for every fixture using world-wide
/// club visibility. Fixtures whose squads can't be built (missing
/// country) are silently skipped — the orchestrator still progresses.
fn build_squads(
    continents: &mut [Continent],
    stamped: &[StampedFixture],
    date: NaiveDate,
) -> Vec<(usize, MatchSquad, MatchSquad, bool)> {
    stamped
        .iter()
        .enumerate()
        .filter_map(|(stamp_idx, stamp)| {
            let home = build_world_match_squad(continents, stamp.fixture.home_country_id, date)?;
            let away = build_world_match_squad(continents, stamp.fixture.away_country_id, date)?;
            Some((stamp_idx, home, away, stamp.fixture.phase.is_knockout()))
        })
        .collect()
}

/// Drain phase transitions for each continent. Runs after fixture
/// processing so a knockout completed today is correctly advanced.
fn run_phase_transitions(continents: &mut [Continent]) {
    for continent in continents.iter_mut() {
        let continent_id = continent.id;
        continent
            .national_team_competitions
            .check_phase_transitions(continent_id);
    }
}

/// Apply a single match's outcome: record into the source continent's
/// competition state, fan stats/Elo/schedule out across the world,
/// and produce the MatchResult for the global match store.
fn apply_match_outcome(
    continents: &mut [Continent],
    stamp: &StampedFixture,
    raw: crate::r#match::MatchResultRaw,
    date: NaiveDate,
) -> Option<MatchResult> {
    let fixture = stamp.fixture.clone();
    let continent_idx = stamp.continent_idx;

    let score = raw.score.as_ref().expect("match should have score").clone();
    let home_score = score.home_team.get();
    let away_score = score.away_team.get();
    let home_country_id = fixture.home_country_id;
    let away_country_id = fixture.away_country_id;

    let match_id = format!(
        "int-{}-{}-{}",
        date.format("%Y%m%d"),
        home_country_id,
        away_country_id
    );

    // Knockout draws read the winner straight from the engine-played
    // shootout. Reputation comparison was wrong: the lower-rep side
    // can win on penalties, and the engine actually models the kicks.
    let penalty_winner = if fixture.phase.is_knockout() && home_score == away_score {
        if score.had_shootout() {
            Some(if score.home_shootout > score.away_shootout {
                home_country_id
            } else if score.away_shootout > score.home_shootout {
                away_country_id
            } else {
                // Shootout tied — defensive fallback.
                home_country_id
            })
        } else {
            // No shootout was run (engine didn't recognise this as a
            // knockout, or fixture data was inconsistent). Last-resort
            // reputation-weighted resolution to keep the tournament
            // moving.
            let home_rep = world_country_reputation(continents, home_country_id);
            let away_rep = world_country_reputation(continents, away_country_id);
            Some(if home_rep >= away_rep {
                home_country_id
            } else {
                away_country_id
            })
        }
    } else {
        None
    };

    let (label, comp_full_name) = continents
        .get(continent_idx)
        .and_then(|c| {
            c.national_team_competitions
                .competitions
                .get(fixture.competition_idx)
        })
        .map(|c| (c.short_name().to_string(), c.config.name.clone()))
        .unwrap_or_else(|| ("INT".to_string(), "International".to_string()));

    if let Some(continent) = continents.get_mut(continent_idx) {
        continent.national_team_competitions.record_result(
            &fixture,
            home_score,
            away_score,
            penalty_winner,
        );
    }

    let player_goals: HashMap<u32, u16> = raw
        .player_stats
        .iter()
        .filter(|(_, stats)| stats.goals > 0)
        .map(|(&id, stats)| (id, stats.goals))
        .collect();
    let appearance_ids: std::collections::HashSet<u32> =
        raw.player_stats.keys().copied().collect();

    apply_world_international_stats(
        continents,
        home_country_id,
        away_country_id,
        &player_goals,
        &appearance_ids,
    );
    apply_world_elo(
        continents,
        home_country_id,
        away_country_id,
        home_score,
        away_score,
    );

    let home_name = world_country_name(continents, home_country_id);
    let away_name = world_country_name(continents, away_country_id);

    record_world_country_schedule(
        continents,
        date,
        home_country_id,
        away_country_id,
        &home_name,
        &away_name,
        home_score,
        away_score,
        &comp_full_name,
        &match_id,
    );

    info!(
        "International match ({}): {} {} - {} {}",
        label, home_name, home_score, away_score, away_name
    );

    Some(MatchResult {
        id: match_id,
        league_id: 0,
        league_slug: "international".to_string(),
        home_team_id: home_country_id,
        away_team_id: away_country_id,
        score,
        details: Some(raw),
        friendly: false,
    })
}
