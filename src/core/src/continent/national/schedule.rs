use chrono::NaiveDate;

use super::competition::GroupFixture;
use super::config::ScheduleConfig;

/// Generate proper round-robin fixtures where each team plays exactly once per matchday.
/// Uses the "circle method" (rotate all teams except one fixed position).
///
/// For N teams (even): N-1 matchdays for single round, 2*(N-1) for home & away.
/// For N teams (odd): N matchdays per round (one team gets a bye each matchday).
///
/// Returns (matchday, home_idx, away_idx) tuples with matchday 1-based.
pub fn generate_round_robin_fixtures(
    team_count: usize,
    matchdays_available: usize,
) -> Vec<(u8, usize, usize)> {
    if team_count < 2 {
        return Vec::new();
    }

    // For circle method, we need an even number of slots
    let n = if team_count % 2 == 0 {
        team_count
    } else {
        team_count + 1
    };
    let rounds_single = n - 1; // matchdays for one leg

    let mut fixtures = Vec::new();

    // Build a rotation array: indices 0..n, where index n-1 is "bye" if team_count is odd
    let mut rotation: Vec<usize> = (0..n).collect();

    // First leg
    for round in 0..rounds_single {
        for i in 0..n / 2 {
            let home = rotation[i];
            let away = rotation[n - 1 - i];

            // Skip if either is the "bye" slot
            if home >= team_count || away >= team_count {
                continue;
            }

            // Alternate home/away by round to be fair
            let md = (round + 1) as u8;
            if round % 2 == 0 {
                fixtures.push((md, home, away));
            } else {
                fixtures.push((md, away, home));
            }
        }

        // Rotate: fix position 0, rotate rest clockwise
        let last = rotation[n - 1];
        for i in (2..n).rev() {
            rotation[i] = rotation[i - 1];
        }
        rotation[1] = last;
    }

    // Second leg (reverse home/away, offset matchdays)
    let first_leg_count = fixtures.len();
    for i in 0..first_leg_count {
        let (md, home, away) = fixtures[i];
        let md2 = md + rounds_single as u8;
        fixtures.push((md2, away, home));
    }

    // Now remap matchdays to fit available schedule slots.
    // Total matchdays generated: 2 * rounds_single (e.g. 8 for 5 teams).
    // Available: matchdays_available (e.g. 8 from config).
    let total_matchdays = 2 * rounds_single;

    if total_matchdays > matchdays_available {
        // Compress: map generated matchdays into available slots
        for fixture in &mut fixtures {
            let zero_based = (fixture.0 - 1) as usize;
            let mapped = (zero_based * matchdays_available / total_matchdays) + 1;
            fixture.0 = mapped.min(matchdays_available) as u8;
        }
    }

    fixtures
}

/// Generate group fixtures with actual dates for qualifying, driven by config.
/// Each team plays at most once per matchday (date).
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
