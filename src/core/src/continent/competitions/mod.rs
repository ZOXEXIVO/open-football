mod champions_league;
mod conference_league;
mod europa_league;
mod super_cup;
mod types;

pub use champions_league::*;
pub use conference_league::*;
pub use europa_league::*;
pub use super_cup::*;
pub use types::*;

#[derive(Debug, Clone)]
pub struct ContinentalCompetitions {
    pub champions_league: ChampionsLeague,
    pub europa_league: EuropaLeague,
    pub conference_league: ConferenceLeague,
    pub super_cup: SuperCup,
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
            super_cup: SuperCup::new(),
        }
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        let mut points = 0.0;

        points += self.champions_league.get_club_points(club_id);
        points += self.europa_league.get_club_points(club_id);
        points += self.conference_league.get_club_points(club_id);

        points
    }

    pub fn get_total_prize_pool(&self) -> f64 {
        self.champions_league.prize_pool
            + self.europa_league.prize_pool
            + self.conference_league.prize_pool
            + self.super_cup.prize_pool
    }
}
