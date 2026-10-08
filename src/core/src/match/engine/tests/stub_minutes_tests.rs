//! The `match-stub` result under `OF_STUB_MINUTES`: every starter credited
//! a full match, the named substitutes left unused.

use super::goal_celebration_tests::squad;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::FootballEngine;

#[test]
fn recording_minutes_credits_every_starter_a_full_match() {
    let mut home = squad(1, 100);
    let mut substitute = home.main_squad[0].clone();
    substitute.id = 199;
    home.substitutes.push(substitute);
    let result = FootballEngine::<840, 545>::stub_result(
        home,
        squad(2, 200),
        &MatchEngineConfig::seeded(1),
        true,
    );
    let full = result
        .player_stats
        .values()
        .filter(|s| s.minutes_played == 90 && s.match_rating == 6.6)
        .count();
    assert_eq!(full, 22);
    assert_eq!(result.player_stats.len(), 22);
    assert!(!result.player_stats.contains_key(&199));
    assert!(result.left_team_players.substitutes_used.is_empty());
}

#[test]
fn the_bare_stub_records_nobody() {
    let result = FootballEngine::<840, 545>::stub_result(
        squad(1, 100),
        squad(2, 200),
        &MatchEngineConfig::seeded(1),
        false,
    );
    assert!(result.player_stats.is_empty());
}
