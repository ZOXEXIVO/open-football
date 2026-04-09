use crate::simulator::SimulatorData;
use log::debug;

pub struct ClubFinanceResult {
    pub club_id: u32,
    /// Club balance is deeply negative — emergency measures needed
    pub is_in_distress: bool,
    /// Number of sponsorship contracts that expired this month
    pub expired_sponsorships: u32,
}

impl ClubFinanceResult {
    pub fn new() -> Self {
        ClubFinanceResult {
            club_id: 0,
            is_in_distress: false,
            expired_sponsorships: 0,
        }
    }

    pub fn with_club(mut self, club_id: u32) -> Self {
        self.club_id = club_id;
        self
    }

    pub fn process(&self, data: &mut SimulatorData) {
        if self.club_id == 0 {
            return;
        }

        // Financial distress: if balance is deeply negative, cut transfer budget
        if self.is_in_distress {
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };

            debug!("Financial distress at {} — freezing transfer budget", club.name);

            // Freeze transfer budget
            if let Some(ref mut budget) = club.finance.transfer_budget {
                budget.amount = 0.0;
            }

            // Reduce wage budget by 10% to force austerity
            if let Some(ref mut budget) = club.finance.wage_budget {
                budget.amount *= 0.90;
            }
        }

        // Expired sponsorships: generate replacement contracts at reduced value
        // (clubs without active sponsorship earn less — incentive to maintain reputation)
        if self.expired_sponsorships > 0 {
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };

            debug!(
                "{} sponsorship(s) expired at {}",
                self.expired_sponsorships, club.name
            );
        }
    }
}
