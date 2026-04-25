use crate::simulator::SimulatorData;
use crate::club::board::manager_market;
use crate::club::board::BoardMoodState;
use crate::club::staff::free_pool;
use crate::club::{StaffClubContract, StaffPosition, StaffStatus};
use crate::TeamType;
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
    /// Signed delta applied to the main-team manager's job_satisfaction
    /// this tick. Positive when the board is happy; negative when
    /// confidence is sliding. Applied in `process`.
    pub manager_satisfaction_delta: f32,
    /// Trigger a mid/late-contract renewal offer to the incumbent
    /// manager — set at season start when board confidence is high
    /// and the contract is approaching its tail end.
    pub offer_manager_renewal: bool,
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
            manager_satisfaction_delta: 0.0,
            offer_manager_renewal: false,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        if self.club_id == 0 {
            return;
        }

        // Grab the sim date before we take a mutable club borrow.
        let today = data.date.date();

        // Sacked staff is collected during the club-mut block and admitted
        // to the global free-agent pool *after* the club borrow ends —
        // `data.free_agent_staff` is on the same `data` and the borrow
        // checker won't allow both mut paths simultaneously.
        let mut sacked_staff: Option<crate::Staff> = None;
        // Mirror for confirm-new-manager: the appointment runs in
        // `manager_market::execute_appointment` after the club borrow
        // ends because it needs concurrent access to the pool.
        let do_confirm = self.confirm_new_manager;

        {
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

        // Push the board's mood onto the manager's own job satisfaction —
        // a coach at a happy club feels secure, a coach under Poor mood
        // feels the pressure building. Applied after the sacking path so
        // we don't adjust a seat that's just been vacated.
        if self.manager_satisfaction_delta.abs() > 0.01 && !self.manager_sacked {
            if let Some(main_team) = club.teams.main_mut() {
                if let Some(mgr) = main_team.staffs.find_mut_by_position(StaffPosition::Manager) {
                    mgr.job_satisfaction = (mgr.job_satisfaction
                        + self.manager_satisfaction_delta)
                        .clamp(0.0, 100.0);
                }
            }
        }

        // Season-start renewal: if the board wants to keep the manager,
        // extend the contract by two years and give a salary bump. The
        // manager is trusted, so the terms are friendly; this prevents
        // a successful coach from running down their deal and walking
        // for free. Only fires when the current contract is short enough
        // to genuinely be at risk (≤18 months out).
        if self.offer_manager_renewal && !self.manager_sacked {
            if let Some(main_team) = club.teams.main_mut() {
                if let Some(mgr) = main_team.staffs.find_mut_by_position(StaffPosition::Manager) {
                    let should_offer = mgr
                        .contract
                        .as_ref()
                        .map(|c| (c.expired - today).num_days() < 540)
                        .unwrap_or(true);
                    if should_offer {
                        if let Some(contract) = mgr.contract.as_mut() {
                            let new_expires = today
                                .with_year(today.year() + 2)
                                .unwrap_or(contract.expired);
                            if new_expires > contract.expired {
                                contract.expired = new_expires;
                            }
                            contract.salary = ((contract.salary as f32) * 1.15) as u32;
                            mgr.job_satisfaction = (mgr.job_satisfaction + 10.0).clamp(0.0, 100.0);
                            info!(
                                "Board offered renewal (+2y, +15% salary) to manager {} at {}",
                                mgr.id, club.name
                            );
                        }
                    }
                }
            }
        }

        // Sacking: terminate the manager contract on the main team and
        // promote the best available coaching-staff member to caretaker.
        // The caretaker runs the team until the 30-day search concludes
        // (see `confirm_new_manager` below). The sacked staff member is
        // *removed* from the team's roster (not just stripped of contract)
        // and routed into the global free-agent pool below the block, so
        // a rival club can sign them next tick.
        if self.manager_sacked {
            let club_name = club.name.clone();
            if let Some(main_team) = club.teams.main_mut() {
                let mut sacked_salary: u32 = 0;
                if let Some(staff) = main_team.staffs.take_by_position(StaffPosition::Manager) {
                    let id = staff.id;
                    if let Some(c) = &staff.contract {
                        sacked_salary = c.salary;
                    }
                    info!(
                        "Board sacked manager (staff id {}) at {} — confidence {}",
                        id, club_name, self.confidence
                    );
                    sacked_staff = Some(staff);
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

            // Start the search clock on the board, locking in the
            // rep-scaled search window. Top clubs hunt for ~60 days,
            // small clubs ~21. The window stays stable across the
            // search even if reputation fluctuates.
            let club_rep = club
                .teams
                .iter()
                .find(|t| matches!(t.team_type, TeamType::Main))
                .map(|t| t.reputation.world)
                .unwrap_or(0);
            manager_market::open_manager_search(&mut club.board, today, club_rep);
        }

        } // end of `club` mutable-borrow scope

        // The club borrow has ended — we can now mutate the global
        // free-agent pool that lives on the same `data` value.
        if let Some(staff) = sacked_staff {
            free_pool::admit_to_pool(&mut data.free_agent_staff, staff, today);
        }

        // Permanent appointment: free-agent hire (preferred) or
        // caretaker promotion (fallback). Lives in `manager_market`
        // because it needs to weave between the global pool and the
        // club's staff collection across multiple short borrows.
        if do_confirm {
            manager_market::execute_appointment(data, self.club_id, today);
        }
    }
}
