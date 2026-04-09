#[derive(Clone)]
pub struct ClubContext<'c> {
    pub id: u32,
    pub name: &'c str,
    pub training_facility_quality: f32,
    pub youth_facility_quality: f32,
    pub academy_quality: f32,
    pub recruitment_quality: f32,
    /// Main team's current league position (1-based, 0 = unknown)
    pub league_position: u8,
    /// Total teams in the league
    pub league_size: u8,
    /// Total matches in a full season (for season progress calculation)
    pub total_league_matches: u8,
}

impl<'c> ClubContext<'c> {
    pub fn new(id: u32, name: &'c str) -> Self {
        ClubContext {
            id,
            name,
            training_facility_quality: 0.35,
            youth_facility_quality: 0.35,
            academy_quality: 0.35,
            recruitment_quality: 0.35,
            league_position: 0,
            league_size: 0,
            total_league_matches: 0,
        }
    }

    pub fn with_facilities(mut self, training: f32, youth: f32, academy: f32, recruitment: f32) -> Self {
        self.training_facility_quality = training;
        self.youth_facility_quality = youth;
        self.academy_quality = academy;
        self.recruitment_quality = recruitment;
        self
    }

    pub fn with_league_position(mut self, position: u8, league_size: u8, total_matches: u8) -> Self {
        self.league_position = position;
        self.league_size = league_size;
        self.total_league_matches = total_matches;
        self
    }
}
