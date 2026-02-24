use chrono::NaiveDate;

use super::competition::GroupFixture;
use super::config::ScheduleConfig;

/// Generate round-robin fixtures for a group (home and away).
/// Returns a list of (matchday, home_idx, away_idx) tuples.
pub fn generate_round_robin_fixtures(team_count: usize, matchdays_available: usize) -> Vec<(u8, usize, usize)> {
    let mut fixtures = Vec::new();
    let mut matchday: u8 = 1;

    // Home leg
    for i in 0..team_count {
        for j in (i + 1)..team_count {
            fixtures.push((matchday, i, j));
            matchday += 1;
            if matchday > 8 {
                matchday = 1;
            }
        }
    }

    // Away leg (reverse home/away)
    for i in 0..team_count {
        for j in (i + 1)..team_count {
            fixtures.push((matchday, j, i));
            matchday += 1;
            if matchday > 8 {
                matchday = 1;
            }
        }
    }

    // Re-assign matchdays properly using round-robin scheduling
    let total = fixtures.len();
    let per_matchday = (total + matchdays_available - 1) / matchdays_available;

    for (idx, fixture) in fixtures.iter_mut().enumerate() {
        fixture.0 = ((idx / per_matchday) + 1).min(matchdays_available) as u8;
    }

    fixtures
}

/// Generate group fixtures with actual dates for qualifying, driven by config
pub fn generate_group_qualifying_fixtures_from_config(
    team_country_ids: &[u32],
    start_year: i32,
    schedule: &ScheduleConfig,
) -> Vec<GroupFixture> {
    let dates = schedule.generate_qualifying_dates(start_year);
    let matchdays_available = dates.len();
    let round_robin = generate_round_robin_fixtures(team_country_ids.len(), matchdays_available);

    let mut fixtures = Vec::new();

    for (matchday, home_idx, away_idx) in round_robin {
        let date = dates
            .iter()
            .find(|(md, _)| *md == matchday)
            .map(|(_, d)| *d)
            .unwrap_or_else(|| {
                dates
                    .first()
                    .map(|(_, d)| *d)
                    .unwrap_or(NaiveDate::from_ymd_opt(start_year, 9, 6).unwrap())
            });

        fixtures.push(GroupFixture {
            matchday,
            date,
            home_country_id: team_country_ids[home_idx],
            away_country_id: team_country_ids[away_idx],
            result: None,
        });
    }

    fixtures
}
