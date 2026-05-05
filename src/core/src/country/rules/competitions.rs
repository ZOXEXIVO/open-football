use chrono::NaiveDate;
use log::debug;

#[derive(Debug, Clone)]
pub struct InternationalCompetition {
    pub name: String,
    pub competition_type: CompetitionType,
    pub participating_clubs: Vec<u32>,
    pub current_round: String,
}

impl InternationalCompetition {
    pub fn simulate_round(&mut self, _date: NaiveDate) {
        // Simulate competition rounds
        debug!("Simulating {} round: {}", self.name, self.current_round);
    }
}

#[derive(Debug, Clone)]
pub enum CompetitionType {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
    SuperCup,
}
