use crate::league::LeagueMatch;

pub struct ScheduleResult {
    pub generated: bool,
    pub scheduled_matches: Vec<LeagueMatch>,
}

impl Default for ScheduleResult {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleResult {
    pub fn new() -> Self {
        ScheduleResult {
            generated: false,
            scheduled_matches: Vec::new(),
        }
    }

    pub fn is_match_scheduled(&self) -> bool {
        !self.scheduled_matches.is_empty()
    }
}
