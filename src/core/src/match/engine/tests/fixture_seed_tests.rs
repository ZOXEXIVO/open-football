//! A pinned sim seed reaches the engine through the fixture: the same
//! fixture plays out the same whatever batch carries it, and an unpinned
//! world keeps drawing every match from entropy.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::{
    CompetitionKind, FixtureContext, MatchPlayEnginePool, MatchResultRaw, SquadFixture,
};
use crate::utils::random::engine::RandomEngine;
use chrono::NaiveDate;
use std::fmt::Write as _;

fn fixture(id: &str) -> FixtureContext {
    FixtureContext::new(
        id,
        NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
        CompetitionKind::League,
        false,
    )
}

fn first(index: usize) -> SquadFixture {
    (index, squad(1, 100), squad(2, 200), fixture("seed-first"))
}

fn second(index: usize) -> SquadFixture {
    (index, squad(3, 300), squad(4, 400), fixture("seed-second"))
}

/// Score plus every player's line, in player order.
fn outcome(raw: &MatchResultRaw) -> String {
    let mut out = format!("{:?}", raw.score);
    let mut stats: Vec<_> = raw.player_stats.iter().collect();
    stats.sort_by_key(|(id, _)| **id);
    for (id, line) in stats {
        let _ = write!(out, "|{id}:{line:?}");
    }
    out
}

fn played(results: &[(usize, MatchResultRaw)], index: usize) -> String {
    results
        .iter()
        .find(|(i, _)| *i == index)
        .map(|(_, raw)| outcome(raw))
        .expect("every fixture comes back")
}

#[test]
fn a_fixture_plays_the_same_whichever_order_its_batch_is_in() {
    let _seed = RandomEngine::hold_seed_for_test(0x5EED_F1C7);
    let pool = MatchPlayEnginePool::new(2);

    let forward = pool.play_squads_local(vec![first(0), second(1)]);
    let backward = pool.play_squads_local(vec![second(1), first(0)]);

    assert_eq!(played(&forward, 0), played(&backward, 0));
    assert_eq!(played(&forward, 1), played(&backward, 1));
}

#[test]
fn a_pinned_fixture_draws_its_own_stream() {
    let _seed = RandomEngine::hold_seed_for_test(0x5EED_F1C7);
    let seed = MatchEngineConfig::for_fixture(&fixture("seed-first"), false).seed;

    assert!(seed.is_some());
    assert_eq!(
        seed,
        MatchEngineConfig::for_fixture(&fixture("seed-first"), false).seed
    );
    assert_ne!(
        seed,
        MatchEngineConfig::for_fixture(&fixture("seed-second"), false).seed
    );
}

#[test]
fn an_unpinned_fixture_keeps_drawing_from_entropy() {
    let _unpinned = RandomEngine::hold_seed_for_test(0);
    assert_eq!(
        MatchEngineConfig::for_fixture(&fixture("seed-first"), false).seed,
        None
    );
}
