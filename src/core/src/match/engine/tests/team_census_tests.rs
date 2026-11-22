//! **The team census reads a scripted match back.** Game states weighted by
//! the time spent in them, and possessions counted from how they began and
//! ended.

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::result::team_census::{LedgerState, PossessionEnd};
use crate::r#match::engine::result::{MatchTally, PlayingTime};
use crate::r#match::engine::teamplay::census::{TeamCensus, TeamCensusInputs};
use crate::r#match::{DeadTime, MatchField, MatchState};

const HOME: u32 = 1;
const AWAY: u32 = 2;
const TICKS_PER_MINUTE: u64 = 6_000;

fn field() -> MatchField {
    MatchField::new(840, 545, squad(HOME, 100), squad(AWAY, 200))
}

fn sample(
    census: &mut TeamCensus,
    field: &MatchField,
    tick: u64,
    holder: u32,
    score: (u8, u8),
    playing_time: PlayingTime,
) {
    census.sample(&TeamCensusInputs {
        field,
        holder: Some(holder),
        home_team_id: HOME,
        away_team_id: AWAY,
        home_goals: score.0,
        away_goals: score.1,
        tick,
        period: MatchState::FirstHalf,
        playing_time,
    });
}

fn shoot(field: &mut MatchField, team_id: u32) {
    let shooter = field
        .players
        .iter_mut()
        .find(|p| p.team_id == team_id)
        .expect("a player");
    shooter.memory.shots_taken += 1;
    shooter.memory.xg_total += 0.25;
}

#[test]
fn the_ledger_weights_each_state_by_its_time() {
    let field = field();
    let mut census = TeamCensus::default();
    // Thirty minutes ahead, fifty level, ten behind; the home side holds the
    // ball 40%, 50% and 70% of each spell.
    let spells = [(30, (1, 0), 0.4), (50, (1, 1), 0.5), (10, (1, 2), 0.7)];
    let mut tick = 0;
    for (minutes, score, home_share) in spells {
        let ticks = minutes * TICKS_PER_MINUTE;
        for i in 0..ticks {
            let holder = if (i as f64) < ticks as f64 * home_share {
                HOME
            } else {
                AWAY
            };
            sample(&mut census, &field, tick, holder, score, PlayingTime::Live);
            tick += 1;
        }
    }
    let mut tally = MatchTally::default();
    census.finish(&mut tally);
    let home = &tally.team_census.ledger[0];
    let away = &tally.team_census.ledger[1];

    let expected = [
        (LedgerState::LeadingByOne, 30, 0.4),
        (LedgerState::Level, 50, 0.5),
        (LedgerState::TrailingByOne, 10, 0.7),
    ];
    for (state, minutes, share) in expected {
        let ledger = &home[state.index()];
        assert_eq!(ledger.ticks, minutes * TICKS_PER_MINUTE, "{state:?}");
        let held = ledger.possession_ticks as f64 / ledger.held_ticks as f64;
        assert!((held - share).abs() < 1e-3, "{state:?}: {held}");
    }
    assert_eq!(home[LedgerState::LeadingByTwo.index()].ticks, 0);
    assert_eq!(
        away[LedgerState::TrailingByOne.index()].ticks,
        30 * TICKS_PER_MINUTE
    );
    assert_eq!(
        away[LedgerState::LeadingByOne.index()].ticks,
        10 * TICKS_PER_MINUTE
    );
    let away_trailing = &away[LedgerState::TrailingByOne.index()];
    let away_share = away_trailing.possession_ticks as f64 / away_trailing.held_ticks as f64;
    assert!((away_share - 0.6).abs() < 1e-3, "{away_share}");
}

#[test]
fn possessions_are_counted_from_how_they_began_and_ended() {
    let mut field = field();
    let mut census = TeamCensus::default();
    let restart = PlayingTime::Dead(DeadTime::Restart);
    // The home side kicks off and keeps it for ten seconds; the away side
    // wins it in play, shoots five seconds later and keeps going until the
    // ball goes dead; the home side restarts and has it to the whistle.
    for tick in 0..1_000 {
        let time = if tick < 50 {
            restart
        } else {
            PlayingTime::Live
        };
        sample(&mut census, &field, tick, HOME, (0, 0), time);
    }
    for tick in 1_000..2_000 {
        if tick == 1_500 {
            shoot(&mut field, AWAY);
        }
        sample(&mut census, &field, tick, AWAY, (0, 0), PlayingTime::Live);
    }
    for tick in 2_000..3_000 {
        let time = if tick < 2_100 {
            restart
        } else {
            PlayingTime::Live
        };
        sample(&mut census, &field, tick, HOME, (0, 0), time);
    }
    let mut tally = MatchTally::default();
    census.finish(&mut tally);
    let team = &tally.team_census;

    let home = &team.possessions[0];
    assert_eq!(home.possessions, 2);
    assert_eq!(home.ended[PossessionEnd::LostInPlay.index()], 1);
    assert_eq!(home.ended[PossessionEnd::PeriodEnd.index()], 1);
    let away = &team.possessions[1];
    assert_eq!(away.possessions, 1);
    assert_eq!(away.ended[PossessionEnd::Shot.index()], 1);
    assert_eq!(away.ticks, 1_000);

    assert_eq!(team.style[1].regains, 1);
    assert_eq!(team.style[1].counter_attacks, 1);
    assert_eq!(team.style[0].regains, 0, "both home spells began dead");
}
