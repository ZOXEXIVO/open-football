use crate::simulator::SimulatorData;
use crate::club::board::BoardMoodState;
use crate::club::{StaffClubContract, StaffPosition, StaffStatus};
use chrono::Datelike;
use log::{debug, info};

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
    /// Board has lost confidence — terminate manager contract this tick.
    pub manager_sacked: bool,
    /// Search period (≥30 days) has elapsed — promote the sitting
    /// caretaker to a permanent manager contract.
    pub confirm_new_manager: bool,
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
            manager_sacked: false,
            confirm_new_manager: false,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        if self.club_id == 0 {
            return;
        }

        // Grab the sim date before we take a mutable club borrow.
        let today = data.date.date();

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

        // Sacking: terminate the manager contract on the main team and
        // promote the best available coaching-staff member to caretaker.
        // The caretaker runs the team until the 30-day search concludes
        // (see `confirm_new_manager` below).
        if self.manager_sacked {
            let club_name = club.name.clone();
            if let Some(main_team) = club.teams.main_mut() {
                let mut sacked_salary: u32 = 0;
                if let Some(staff) = main_team.staffs.find_mut_by_position(StaffPosition::Manager) {
                    let id = staff.id;
                    if let Some(c) = &staff.contract {
                        sacked_salary = c.salary;
                    }
                    staff.contract = None;
                    info!(
                        "Board sacked manager (staff id {}) at {} — confidence {}",
                        id, club_name, self.confidence
                    );
                }

                // Promote best existing coaching-staff member to Caretaker.
                // Score: tactical + man_management + motivating + coaching.mental.
                let caretaker_id = main_team.staffs.best_coach_id(|s| {
                    s.staff_attributes.coaching.tactical as u32
                        + s.staff_attributes.mental.man_management as u32
                        + s.staff_attributes.mental.motivating as u32
                        + s.staff_attributes.coaching.mental as u32
                });

                if let Some(id) = caretaker_id {
                    if let Some(staff) = main_team.staffs.find_mut(id) {
                        // Caretaker deal: 60 days at max(current, half of sacked).
                        let current_salary = staff
                            .contract
                            .as_ref()
                            .map(|c| c.salary)
                            .unwrap_or(0);
                        let salary = current_salary.max(sacked_salary / 2);
                        let expires = today
                            .checked_add_signed(chrono::Duration::days(60))
                            .unwrap_or_else(|| {
                                chrono::NaiveDate::from_ymd_opt(
                                    today.year() + 1, today.month(), 1,
                                ).unwrap()
                            });
                        staff.contract = Some(StaffClubContract::new(
                            salary,
                            expires,
                            StaffPosition::CaretakerManager,
                            StaffStatus::Active,
                        ));
                        info!(
                            "Promoted staff {} to caretaker manager at {}",
                            id, club_name
                        );
                    }
                }
            }

            // Start the search clock on the board.
            club.board.manager_search_since = Some(today);
        }

        // Confirm the caretaker (or external hire) after ≥30 days.
        // Interim becomes permanent — simulates the common outcome
        // where the board sticks with the caretaker.
        if self.confirm_new_manager {
            let club_name = club.name.clone();
            if let Some(main_team) = club.teams.main_mut() {
                if let Some(staff) =
                    main_team.staffs.find_mut_by_position(StaffPosition::CaretakerManager)
                {
                    let id = staff.id;
                    let salary = staff.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                    // 3-year full contract — standard appointment.
                    let expires = today
                        .with_year(today.year() + 3)
                        .unwrap_or(today);
                    staff.contract = Some(StaffClubContract::new(
                        salary,
                        expires,
                        StaffPosition::Manager,
                        StaffStatus::Active,
                    ));
                    info!(
                        "Caretaker {} confirmed as permanent manager at {}",
                        id, club_name
                    );
                }
            }
            club.board.manager_search_since = None;
        }
    }
}
