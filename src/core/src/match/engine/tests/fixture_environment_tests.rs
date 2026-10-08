//! Every match is played in the weather, on the pitch and before the crowd
//! and referee its fixture draws, and the same fixture always draws the
//! same.

#![cfg(test)]

use crate::r#match::engine::environment::{MatchEnvironment, Weather};
use crate::r#match::engine::referee::RefereeProfile;
use crate::r#match::{CompetitionKind, FixtureContext, MatchRules};
use chrono::{Duration, NaiveDate};
use std::collections::HashSet;

fn league(id: &str, date: NaiveDate) -> FixtureContext {
    FixtureContext::new(id, date, CompetitionKind::League, false)
}

#[test]
fn the_same_fixture_draws_the_same_match() {
    let date = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
    let fixture = league("eng_1_14_101_102", date);
    assert_eq!(
        MatchEnvironment::for_fixture(&fixture),
        MatchEnvironment::for_fixture(&fixture)
    );
    assert_eq!(
        RefereeProfile::draw(fixture.seed, RefereeProfile::default()),
        RefereeProfile::draw(fixture.seed, RefereeProfile::default())
    );
    assert_ne!(fixture.seed, league("eng_1_14_103_104", date).seed);
    assert_ne!(
        fixture.seed,
        league("eng_1_14_101_102", date + Duration::days(7)).seed
    );
}

#[test]
fn a_season_of_fixtures_sees_every_weather_and_every_pitch() {
    let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let mut weathers = HashSet::new();
    let mut pitches = HashSet::new();
    for day in (0..300).step_by(3) {
        let date = start + Duration::days(day);
        for game in 0..10 {
            let env = MatchEnvironment::for_fixture(&league(&format!("m{day}_{game}"), date));
            weathers.insert(env.weather);
            pitches.insert(env.pitch);
        }
    }
    assert_eq!(weathers.len(), Weather::ALL.len(), "{weathers:?}");
    assert_eq!(pitches.len(), 6, "{pitches:?}");
}

#[test]
fn the_seasons_turn_the_other_way_south_of_the_equator() {
    let january = NaiveDate::from_ymd_opt(2027, 1, 16).unwrap();
    let draws = |continent: u32| -> Vec<Weather> {
        (0..400)
            .map(|i| {
                let fixture = league(&format!("s{i}"), january).on_continent(continent);
                MatchEnvironment::for_fixture(&fixture).weather
            })
            .collect()
    };
    let north = draws(FixtureContext::EUROPE);
    let south = draws(FixtureContext::SOUTH_AMERICA);
    assert!(north.contains(&Weather::Snow) && !north.contains(&Weather::Hot));
    assert!(south.contains(&Weather::Hot) && !south.contains(&Weather::Snow));
}

#[test]
fn a_friendly_matters_less_and_draws_a_smaller_crowd() {
    let date = NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
    let competitive = MatchEnvironment::for_fixture(&league("x", date));
    let friendly = MatchEnvironment::for_fixture(&FixtureContext::new(
        "x",
        date,
        CompetitionKind::Friendly,
        false,
    ));
    assert!(friendly.match_importance < competitive.match_importance);
    assert!(friendly.crowd_intensity < competitive.crowd_intensity);
}

#[test]
fn every_competition_plays_to_its_own_rules() {
    assert_eq!(
        MatchRules::resolve_default(CompetitionKind::International),
        MatchRules::international()
    );
    assert_eq!(
        MatchRules::resolve_default(CompetitionKind::Friendly),
        MatchRules::friendly()
    );
    assert_eq!(
        MatchRules::resolve_default(CompetitionKind::Continental),
        MatchRules::modern()
    );
}

#[test]
fn referees_differ_from_fixture_to_fixture_around_the_baseline() {
    let baseline = RefereeProfile::default();
    let date = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
    let strictness: Vec<f32> = (0..500)
        .map(|i| RefereeProfile::draw(league(&format!("r{i}"), date).seed, baseline).strictness)
        .collect();
    let (low, high) = strictness
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
    let mean = strictness.iter().sum::<f32>() / strictness.len() as f32;
    assert!(
        high - low > 0.15,
        "every referee is the same man: {low}..{high}"
    );
    assert!((mean - baseline.strictness).abs() < 0.02, "mean {mean}");
    assert!(low >= 0.0 && high <= 1.0);
}
