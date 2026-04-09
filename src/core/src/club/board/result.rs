use crate::simulator::SimulatorData;
use crate::club::board::BoardMoodState;
use log::debug;

pub struct BoardResult {
    pub club_id: u32,
    pub players_loan_listed: u32,
    pub players_transfer_listed: u32,
    pub mood: BoardMoodState,
    pub confidence: i32,
    pub cut_transfer_budget: bool,
    /// Board releases extra funds for overperformance
    pub bonus_transfer_funds: bool,
    pub squad_over_limit: bool,
    pub squad_excess: usize,
    pub squad_under_limit: bool,
    /// Team is significantly below expected league position
    pub underperforming: bool,
}

impl BoardResult {
    pub fn new() -> Self {
        BoardResult {
            club_id: 0,
            players_loan_listed: 0,
            players_transfer_listed: 0,
            mood: BoardMoodState::Normal,
            confidence: 65,
            cut_transfer_budget: false,
            bonus_transfer_funds: false,
            squad_over_limit: false,
            squad_excess: 0,
            squad_under_limit: false,
            underperforming: false,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        if self.club_id == 0 {
            return;
        }

        let club = match data.club_mut(self.club_id) {
            Some(c) => c,
            None => return,
        };

        // Poor mood: board pressures the club by reducing transfer budget
        if matches!(self.mood, BoardMoodState::Poor) {
            debug!("Board mood Poor at {} (confidence: {}) — cutting transfer budget by 25%",
                club.name, self.confidence);
            if let Some(ref mut budget) = club.finance.transfer_budget {
                budget.amount *= 0.75;
            }
        }

        // Excellent mood + overperforming: board adds 20% bonus to transfer budget
        if self.bonus_transfer_funds {
            debug!("Board pleased at {} — releasing extra transfer funds (+20%)", club.name);
            if let Some(ref mut budget) = club.finance.transfer_budget {
                budget.amount *= 1.20;
            }
        }
    }
}
