use chrono::NaiveDate;

use super::competition::GroupFixture;

/// International break match dates within a qualifying year.
/// (month, day) pairs for scheduling qualifying matchdays.
const QUALIFYING_MATCH_DATES: [(u32, u32); 8] = [
    (9, 6),   // Sep matchday 1
    (9, 9),   // Sep matchday 2
    (10, 11), // Oct matchday 3
    (10, 14), // Oct matchday 4
    (11, 15), // Nov matchday 5
    (11, 18), // Nov matchday 6
    (3, 22),  // Mar matchday 7 (following year)
    (3, 25),  // Mar matchday 8 (following year)
];

/// Tournament group stage dates (month, day)
const TOURNAMENT_GROUP_DATES: [(u32, u32); 9] = [
    (6, 14), (6, 15), (6, 16), // Group MD1
    (6, 18), (6, 19), (6, 20), // Group MD2
    (6, 22), (6, 23), (6, 24), // Group MD3
];

/// Tournament knockout dates (month, day)
const TOURNAMENT_KNOCKOUT_DATES: [(u32, u32); 7] = [
    (6, 28), (6, 29), // R16
    (7, 2), (7, 3),   // QF
    (7, 6), (7, 7),   // SF
    (7, 10),           // Final
];

/// Generate qualifying fixture dates for a given starting year.
/// Matchdays 1-6 are in the starting year (Sep-Nov),
/// matchdays 7-8 are in the following year (Mar).
pub fn generate_qualifying_dates(start_year: i32) -> Vec<(u8, NaiveDate)> {
    let mut dates = Vec::new();

    for (matchday_idx, &(month, day)) in QUALIFYING_MATCH_DATES.iter().enumerate() {
        let year = if month >= 9 { start_year } else { start_year + 1 };
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            dates.push(((matchday_idx + 1) as u8, date));
        }
    }

    dates
}

/// Generate round-robin fixtures for a group (home and away).
/// Returns a list of (matchday, home_idx, away_idx) tuples.
pub fn generate_round_robin_fixtures(team_count: usize) -> Vec<(u8, usize, usize)> {
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
    let matchdays_available = QUALIFYING_MATCH_DATES.len();
    let per_matchday = (total + matchdays_available - 1) / matchdays_available;

    for (idx, fixture) in fixtures.iter_mut().enumerate() {
        fixture.0 = ((idx / per_matchday) + 1).min(matchdays_available) as u8;
    }

    fixtures
}

/// Generate group fixtures with actual dates for qualifying
pub fn generate_group_qualifying_fixtures(
    team_country_ids: &[u32],
    start_year: i32,
) -> Vec<GroupFixture> {
    let dates = generate_qualifying_dates(start_year);
    let round_robin = generate_round_robin_fixtures(team_country_ids.len());

    let mut fixtures = Vec::new();

    for (matchday, home_idx, away_idx) in round_robin {
        // Find the date for this matchday
        let date = dates
            .iter()
            .find(|(md, _)| *md == matchday)
            .map(|(_, d)| *d)
            .unwrap_or_else(|| {
                // Fallback: use first available date
                dates.first().map(|(_, d)| *d).unwrap_or(
                    NaiveDate::from_ymd_opt(start_year, 9, 6).unwrap()
                )
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

/// Generate tournament group stage dates for a given year
pub fn generate_tournament_group_dates(year: i32) -> Vec<NaiveDate> {
    TOURNAMENT_GROUP_DATES
        .iter()
        .filter_map(|&(month, day)| NaiveDate::from_ymd_opt(year, month, day))
        .collect()
}

/// Generate tournament knockout dates for a given year
pub fn generate_tournament_knockout_dates(year: i32) -> Vec<NaiveDate> {
    TOURNAMENT_KNOCKOUT_DATES
        .iter()
        .filter_map(|&(month, day)| NaiveDate::from_ymd_opt(year, month, day))
        .collect()
}
