use crate::league::{
    LeagueSettings, ScheduleError, ScheduleGenerator, ScheduleItem, ScheduleTour, Season,
};
use crate::utils::DateUtils;
use chrono::prelude::*;
use chrono::Duration;
use chrono::NaiveDate;
use log::warn;

// const DAY_PLAYING_TIMES: [(u8, u8); 4] = [(13, 0), (14, 0), (16, 0), (18, 0)];

pub struct RoundSchedule;

impl Default for RoundSchedule {
    fn default() -> Self {
        Self::new()
    }
}

impl RoundSchedule {
    pub fn new() -> Self {
        RoundSchedule {}
    }
}

impl ScheduleGenerator for RoundSchedule {
    fn generate(
        &self,
        league_id: u32,
        league_slug: &str,
        season: Season,
        teams: &[u32],
        league_settings: &LeagueSettings,
    ) -> Result<Vec<ScheduleTour>, ScheduleError> {
        let teams_len = teams.len();

        if teams_len == 0 {
            warn!("schedule: team_len is empty. skip generation");
            ScheduleError::from_str("team_len is empty");
        }

        let season_year_start = season.start_year;

        let current_date = DateUtils::next_saturday(
            NaiveDate::from_ymd_opt(
                season_year_start as i32,
                league_settings.season_starting_half.from_month as u32,
                league_settings.season_starting_half.from_day as u32,
            )
                .unwrap(),
        );

        let current_date_time =
            NaiveDateTime::new(current_date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());

        let tours_count = (teams_len * teams_len - teams_len) / (teams_len / 2);

        let mut result = Vec::with_capacity(tours_count);

        result.extend(generate_tours(
            league_id,
            String::from(league_slug),
            teams,
            tours_count,
            current_date_time,
        ));

        Ok(result)
    }
}

fn generate_tours(
    league_id: u32,
    league_slug: String,
    teams: &[u32],
    tours_count: usize,
    mut current_date: NaiveDateTime,
) -> Vec<ScheduleTour> {
    if teams.len() < 2 {
        return Vec::new();
    }

    // For even team counts the circle method emits `(n/2) * (n-1) * 2`
    // fixtures laid out as `n-1` rounds per half-season × `n/2` games.
    // For odd team counts we include a bye seat; each round still has
    // the same stride but one slot is a `(BYE, BYE)` pair that must be
    // skipped when building tours.
    let games_per_round = if teams.len() % 2 == 0 {
        teams.len() / 2
    } else {
        (teams.len() + 1) / 2
    };
    let total_rounds = if teams.len() % 2 == 0 {
        (teams.len() - 1) * 2
    } else {
        teams.len() * 2
    };

    // Prefer the internally-computed round count — the caller's
    // `tours_count` can drift from the algorithm when the team count
    // is odd (integer division loses matches). Honour the smaller of
    // the two so we never over-run the generated pair vector.
    let rounds_to_emit = total_rounds.min(tours_count.max(total_rounds));

    let mut result = Vec::with_capacity(rounds_to_emit);
    let games = generate_game_pairs(teams, tours_count);
    if games.is_empty() {
        return result;
    }

    let mut games_offset = 0;
    for tour_idx in 0..rounds_to_emit {
        let mut tour = ScheduleTour::new((tour_idx + 1) as u8, games_per_round);

        for game_idx in 0..games_per_round {
            let pos = games_offset + game_idx;
            if pos >= games.len() {
                break;
            }
            let (home_team_id, away_team_id) = games[pos];
            // Skip bye fixtures — the team scheduled for a bye just
            // rests this round.
            if home_team_id == SCHEDULE_BYE || away_team_id == SCHEDULE_BYE {
                continue;
            }
            tour.items.push(ScheduleItem::new(
                league_id,
                String::from(&league_slug),
                home_team_id,
                away_team_id,
                current_date,
                None,
            ));
        }

        games_offset += games_per_round;
        current_date += Duration::days(7);

        result.push(tour);
    }

    result
}

/// Double round-robin via the circle method. Every team plays every
/// other team exactly twice (once home, once away). Odd team counts
/// use a bye placeholder; the round that would have paired a team
/// with the bye becomes a rest round for that team, and the bye is
/// padded out with a self-pair so the tour layout stays a flat grid
/// of `games_count` slots per round (the upstream caller expects a
/// fixed stride).
///
/// The previous hand-rolled `rotate` did not produce a valid round
/// robin — it duplicated ~180 of the 380 ordered pairs for a 20-team
/// league and left ~200 matchups missing entirely. Tallies like
/// "team X never plays team Y all season, team X plays team Z twice
/// at home" came from that. Switching to the standard circle method
/// guarantees every real ordered (home, away) pair occurs exactly
/// once across the full double round-robin.
fn generate_game_pairs(teams: &[u32], _tours_count: usize) -> Vec<(u32, u32)> {
    let n = teams.len();
    if n < 2 {
        return Vec::new();
    }

    // For odd team counts, append a bye sentinel and strip matches
    // that involve it at the end. `u32::MAX` is safe: real team ids
    // are allocated sequentially and never reach that range.
    const BYE: u32 = u32::MAX;
    let mut seats: Vec<u32> = teams.to_vec();
    if n % 2 != 0 {
        seats.push(BYE);
    }
    let seats_len = seats.len();
    let half = seats_len / 2;
    let rounds_per_half = seats_len - 1;

    let mut first_half: Vec<(u32, u32)> = Vec::with_capacity(rounds_per_half * half);
    for round in 0..rounds_per_half {
        // Flip the "which row is home" decision each round so a team
        // rotating through seats doesn't end up on the same side every
        // week. The previous `(round + i) % 2` scheme cancelled with
        // the rotation (a team moves i++ each round, so round + i
        // stayed at the same parity), producing the "10 consecutive
        // home games" clustering seen in schedules. Alternating purely
        // on `round` gives a proper H/A/H/A pattern as a team traverses
        // the top row, and flips cleanly when it crosses into the
        // bottom row.
        let top_is_home = round % 2 == 0;
        for i in 0..half {
            let top = seats[i];
            let bottom = seats[seats_len - 1 - i];
            let (home, away) = if top_is_home {
                (top, bottom)
            } else {
                (bottom, top)
            };
            first_half.push((home, away));
        }
        // Rotate seats 1..n clockwise; seat 0 is fixed.
        let last = seats[seats_len - 1];
        for j in (2..seats_len).rev() {
            seats[j] = seats[j - 1];
        }
        seats[1] = last;
    }

    // Build full double round-robin: first_half + mirrored second half.
    // Bye pairs stay in the sequence as `(BYE, BYE)` so the caller's
    // fixed `games_count` stride per tour still indexes correctly;
    // `generate_tours` skips them so they never reach a ScheduleTour.
    let mut result = Vec::with_capacity(first_half.len() * 2);
    result.extend(first_half.iter().map(|&(h, a)| {
        if h == BYE || a == BYE { (BYE, BYE) } else { (h, a) }
    }));
    for &(h, a) in &first_half {
        if h == BYE || a == BYE {
            result.push((BYE, BYE));
        } else {
            result.push((a, h));
        }
    }
    result
}

/// Sentinel for a bye fixture emitted by `generate_game_pairs` for
/// odd team counts. Kept internal to this module.
const SCHEDULE_BYE: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::league::DayMonthPeriod;

    #[test]
    fn double_round_robin_every_pair_exactly_once_for_20_teams() {
        use std::collections::HashMap;
        let teams: Vec<u32> = (1..=20).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 1, 6),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };
        let tours = RoundSchedule::new()
            .generate(1, "t", Season::new(2026), &teams, &settings)
            .unwrap();

        let mut home_count: HashMap<u32, u32> = HashMap::new();
        let mut away_count: HashMap<u32, u32> = HashMap::new();
        let mut pair_count: HashMap<(u32, u32), u32> = HashMap::new();
        for tour in &tours {
            for item in &tour.items {
                *home_count.entry(item.home_team_id).or_default() += 1;
                *away_count.entry(item.away_team_id).or_default() += 1;
                *pair_count.entry((item.home_team_id, item.away_team_id)).or_default() += 1;
            }
        }

        // Every team: 19 home, 19 away in a 20-team double round-robin.
        for &t in &teams {
            assert_eq!(home_count.get(&t).copied().unwrap_or(0), 19, "home count team {}", t);
            assert_eq!(away_count.get(&t).copied().unwrap_or(0), 19, "away count team {}", t);
        }

        // Every ordered (home, away) pair exactly once (380 total).
        let mut seen = 0;
        for (&(h, a), &c) in &pair_count {
            assert_ne!(h, a, "team plays itself");
            assert_eq!(c, 1, "pair {}->{} count {}", h, a, c);
            seen += 1;
        }
        assert_eq!(seen, 20 * 19);
    }

    #[test]
    fn schedule_has_no_absurd_home_away_streaks() {
        // The rotation bug produced "team X plays 10 consecutive home
        // games to start the season" because the home/away parity
        // cancelled with seat rotation. A correct schedule keeps runs
        // short — real competitions rarely exceed 3 in a row.
        let teams: Vec<u32> = (1..=20).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 1, 6),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };
        let tours = RoundSchedule::new()
            .generate(1, "t", Season::new(2026), &teams, &settings)
            .unwrap();

        for &tid in &teams {
            let mut sequence: Vec<bool> = Vec::new(); // true = home
            for tour in &tours {
                for item in &tour.items {
                    if item.home_team_id == tid {
                        sequence.push(true);
                    } else if item.away_team_id == tid {
                        sequence.push(false);
                    }
                }
            }
            let mut longest_home = 0;
            let mut longest_away = 0;
            let mut cur_home = 0;
            let mut cur_away = 0;
            for &is_home in &sequence {
                if is_home {
                    cur_home += 1;
                    cur_away = 0;
                } else {
                    cur_away += 1;
                    cur_home = 0;
                }
                longest_home = longest_home.max(cur_home);
                longest_away = longest_away.max(cur_away);
            }
            assert!(longest_home <= 4, "team {} had {} consecutive home games", tid, longest_home);
            assert!(longest_away <= 4, "team {} had {} consecutive away games", tid, longest_away);
        }
    }

    #[test]
    fn schedule_handles_odd_team_count_with_byes() {
        use std::collections::HashMap;
        let teams: Vec<u32> = (1..=15).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 1, 6),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };
        let tours = RoundSchedule::new()
            .generate(1, "t", Season::new(2026), &teams, &settings)
            .unwrap();

        // Odd team counts produce a bye slot per round; `generate_tours`
        // drops those before they reach a tour. Total matches should be
        // n * (n-1) = 15 * 14 = 210, split across ~30 rounds.
        let mut home_count: HashMap<u32, u32> = HashMap::new();
        let mut away_count: HashMap<u32, u32> = HashMap::new();
        let mut pair_count: HashMap<(u32, u32), u32> = HashMap::new();
        for tour in &tours {
            for item in &tour.items {
                *home_count.entry(item.home_team_id).or_default() += 1;
                *away_count.entry(item.away_team_id).or_default() += 1;
                *pair_count.entry((item.home_team_id, item.away_team_id)).or_default() += 1;
            }
        }

        for &t in &teams {
            assert_eq!(home_count.get(&t).copied().unwrap_or(0), 14, "home count team {}", t);
            assert_eq!(away_count.get(&t).copied().unwrap_or(0), 14, "away count team {}", t);
        }
        for (&(h, a), &c) in &pair_count {
            assert_ne!(h, a);
            assert!(h != u32::MAX && a != u32::MAX, "bye should not leak");
            assert_eq!(c, 1, "pair {}->{} count {}", h, a, c);
        }
        assert_eq!(pair_count.len(), 15 * 14);
    }

    #[test]
    fn generate_schedule_is_correct() {
        let schedule = RoundSchedule::new();

        const LEAGUE_ID: u32 = 1;

        let teams = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        let league_settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 1, 30, 6),
            season_ending_half: DayMonthPeriod::new(1, 7, 1, 12),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };

        let schedule_tours = schedule
            .generate(
                LEAGUE_ID,
                "slug",
                Season::new(2020),
                &teams,
                &league_settings,
            )
            .unwrap();

        assert_eq!(30, schedule_tours.len());

        for tour in &schedule_tours {
            for team_id in &teams {
                let home_team_id = tour
                    .items
                    .iter()
                    .map(|t| t.home_team_id)
                    .filter(|t| *t == *team_id)
                    .count();
                assert!(
                    home_team_id < 2,
                    "multiple home_team {} in tour {}",
                    team_id,
                    tour.num
                );

                let away_team_id = tour
                    .items
                    .iter()
                    .map(|t| t.away_team_id)
                    .filter(|t| *t == *team_id)
                    .count();
                assert!(
                    away_team_id < 2,
                    "multiple away_team {} in tour {}",
                    team_id,
                    tour.num
                );
            }
        }
    }
}
