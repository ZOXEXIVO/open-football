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

        let (season_year_start, _season_year_end) = match season {
            Season::OneYear(year) => (year, year),
            Season::TwoYear(start_year, end_year) => (start_year, end_year),
        };

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
    let team_len = teams.len() as u32;
    let games_count = (team_len / 2) as usize;

    let mut result = Vec::with_capacity(tours_count);

    let mut games_offset = 0;

    let games = generate_game_pairs(&teams, tours_count);

    for tour in 0..tours_count {
        let mut tour = ScheduleTour::new((tour + 1) as u8, games_count);

        for game_idx in 0..games_count {
            let (home_team_id, away_team_id) = games[games_offset + game_idx as usize];

            tour.items.push(ScheduleItem::new(
                league_id,
                String::from(&league_slug),
                home_team_id,
                away_team_id,
                current_date,
                None,
            ));
        }

        games_offset += games_count;
        current_date += Duration::days(7);

        result.push(tour);
    }

    result
}

fn generate_game_pairs(teams: &[u32], tours_count: usize) -> Vec<(u32, u32)> {
    let mut result = Vec::with_capacity(tours_count);

    let team_len = teams.len() as u32;
    let team_len_half = team_len / 2u32;

    let mut temp_vec = Vec::with_capacity((team_len_half + 1) as usize);

    for team in 0..team_len_half {
        temp_vec.push((teams[team as usize], teams[(team_len - team - 1) as usize]))
    }

    for team in &temp_vec {
        result.push((team.0, team.1));
    }

    for _ in 0..tours_count {
        rotate(&mut temp_vec);

        for team in &temp_vec {
            result.push((team.0, team.1));
        }
    }

    result
}

fn rotate(clubs: &mut Vec<(u32, u32)>) {
    let teams_len = clubs.len();

    let right_top = clubs[0].1;
    let left_bottom = clubs[teams_len - 1].0;

    for i in 0..teams_len - 1 {
        clubs[i].1 = clubs[i + 1].1;
        clubs[teams_len - i - 1].0 = clubs[teams_len - i - 2].0;
    }

    clubs[0].0 = right_top;
    clubs[teams_len - 1].1 = left_bottom;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::league::DayMonthPeriod;

    #[test]
    fn generate_schedule_is_correct() {
        let schedule = RoundSchedule::new();

        const LEAGUE_ID: u32 = 1;

        let teams = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        let league_settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 1, 30, 6),
            season_ending_half: DayMonthPeriod::new(1, 7, 1, 12),
        };

        let schedule_tours = schedule
            .generate(
                LEAGUE_ID,
                "slug",
                Season::TwoYear(2020, 2021),
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
