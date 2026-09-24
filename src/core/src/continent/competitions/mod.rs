mod champions_league;
mod conference_league;
mod copa_libertadores;
mod europa_league;
mod qualification;
mod super_cup;
mod types;

pub use champions_league::*;
pub use conference_league::*;
pub use copa_libertadores::*;
pub use europa_league::*;
pub use qualification::*;
pub use super_cup::*;
pub use types::*;

use chrono::NaiveDate;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ContinentalCompetitions {
    pub champions_league: ChampionsLeague,
    pub europa_league: EuropaLeague,
    pub conference_league: ConferenceLeague,
    pub copa_libertadores: CopaLibertadores,
    pub super_cup: SuperCup,
}

/// Continental fixture dates by club id, from
/// [`ContinentalCompetitions::commitments`].
#[derive(Debug, Clone, Default)]
pub struct ContinentalCommitments {
    by_club: HashMap<u32, Vec<NaiveDate>>,
}

impl ContinentalCommitments {
    pub fn is_empty(&self) -> bool {
        self.by_club.is_empty()
    }

    /// The club's continental dates, ascending; empty when it has none.
    pub fn dates(&self, club_id: u32) -> &[NaiveDate] {
        self.by_club.get(&club_id).map(Vec::as_slice).unwrap_or(&[])
    }
}

impl Default for ContinentalCompetitions {
    fn default() -> Self {
        Self::new()
    }
}

impl ContinentalCompetitions {
    pub fn new() -> Self {
        ContinentalCompetitions {
            champions_league: ChampionsLeague::new(),
            europa_league: EuropaLeague::new(),
            conference_league: ConferenceLeague::new(),
            copa_libertadores: CopaLibertadores::new(),
            super_cup: SuperCup::new(),
        }
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        let mut points = 0.0;

        points += self.champions_league.get_club_points(club_id);
        points += self.europa_league.get_club_points(club_id);
        points += self.conference_league.get_club_points(club_id);
        points += self.copa_libertadores.get_club_points(club_id);

        points
    }

    /// Every club already drawn into one of the continent's club
    /// competitions for the season that starts in `year`.
    pub fn drawn_this_season(&self, year: u16) -> HashSet<u32> {
        [
            (
                self.champions_league.season_year,
                &self.champions_league.participating_clubs,
            ),
            (
                self.europa_league.season_year,
                &self.europa_league.participating_clubs,
            ),
            (
                self.conference_league.season_year,
                &self.conference_league.participating_clubs,
            ),
            (
                self.copa_libertadores.season_year,
                &self.copa_libertadores.participating_clubs,
            ),
        ]
        .into_iter()
        .filter(|(season, _)| *season == year)
        .flat_map(|(_, clubs)| clubs.iter().copied())
        .collect()
    }

    /// Every club's continental fixture dates between `from` and `to`,
    /// inclusive — the nights a domestic fixture has to keep clear of.
    pub fn commitments(&self, from: NaiveDate, to: NaiveDate) -> ContinentalCommitments {
        let mut by_club: HashMap<u32, Vec<NaiveDate>> = HashMap::new();
        let fixtures = self
            .champions_league
            .matches
            .iter()
            .chain(&self.europa_league.matches)
            .chain(&self.conference_league.matches)
            .chain(&self.copa_libertadores.matches)
            .filter(|m| m.date >= from && m.date <= to);
        for m in fixtures {
            by_club.entry(m.home_team).or_default().push(m.date);
            by_club.entry(m.away_team).or_default().push(m.date);
        }
        for dates in by_club.values_mut() {
            dates.sort_unstable();
            dates.dedup();
        }
        ContinentalCommitments { by_club }
    }

    pub fn get_total_prize_pool(&self) -> f64 {
        self.champions_league.prize_pool
            + self.europa_league.prize_pool
            + self.conference_league.prize_pool
            + self.copa_libertadores.prize_pool
            + self.super_cup.prize_pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continent::ContinentalRankings;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn commitments_collect_each_clubs_continental_nights_in_the_window() {
        let mut competitions = ContinentalCompetitions::new();
        let rankings = ContinentalRankings::new();
        competitions.champions_league.conduct_draw(
            &(1..=32).collect::<Vec<u32>>(),
            &rankings,
            d(8, 15),
        );
        competitions.europa_league.conduct_draw(
            &(101..=132).collect::<Vec<u32>>(),
            &rankings,
            d(8, 20),
        );

        let autumn = competitions.commitments(d(9, 1), d(12, 31));
        for club in [1, 32, 101, 132] {
            let dates = autumn.dates(club);
            assert_eq!(dates.len(), 6, "club {club}: six group nights");
            assert!(dates.windows(2).all(|w| w[0] < w[1]));
        }
        assert!(
            autumn.dates(500).is_empty(),
            "a club not in Europe has none"
        );

        let first_matchday = competitions.commitments(d(9, 1), d(9, 20));
        assert_eq!(first_matchday.dates(1).len(), 1);
        assert_eq!(
            first_matchday.dates(101),
            &[d(9, 17)],
            "Europa League MD1 is Thursday 17th"
        );
    }
}
