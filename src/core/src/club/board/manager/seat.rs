use crate::StaffStub;
use crate::club::Club;
use crate::club::Team;
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind};
use crate::club::staff::{StaffClubContract, StaffPosition, StaffStatus};
use crate::shared::fullname::FullName;
use chrono::{Datelike, Duration, NaiveDate};
use log::info;

pub struct ManagerSeat;

impl ManagerSeat {
    /// True when the club's main-team head-coach seat is open to a
    /// permanent appointment: either no head coach at all, or only a
    /// `CaretakerManager` (interim) is sitting in the seat. A permanent
    /// `Manager` blocks any new hire — same-club replacement requires
    /// the incumbent to be sacked first (which removes the `Manager`
    /// and promotes a caretaker).
    pub fn club_has_vacancy(club: &Club) -> bool {
        let Some(main) = club.teams.main() else {
            return false;
        };
        main.staffs
            .find_by_position(StaffPosition::Manager)
            .is_none()
    }

    /// Demote any sitting `CaretakerManager` on the main team back to
    /// a generic `Coach`. Used by both the free-agent appointment path
    /// and the poach-finalize path before a permanent manager is
    /// installed, so we never end a tick with both `Manager` and
    /// `CaretakerManager` holding the head-coach seat simultaneously.
    pub fn clear_caretaker(team: &mut Team) {
        if let Some(caretaker) = team
            .staffs
            .find_mut_by_position(StaffPosition::CaretakerManager)
        {
            if let Some(c) = caretaker.contract.as_mut() {
                c.position = StaffPosition::Coach;
            }
        }
    }

    /// Promote the strongest remaining coach on `team` to
    /// `CaretakerManager` for a 60-day stint. Salary is
    /// `max(current_coach_salary, prior_salary / 2)` — the same formula
    /// the sacking path uses, so a coach who steps up after a poach is
    /// paid like one who steps up after a sacking. Returns `true` if a
    /// caretaker was installed; `false` when no coaching staff are on
    /// the books (small clubs, dev fixtures).
    pub fn promote_best_caretaker(team: &mut Team, prior_salary: u32, today: NaiveDate) -> bool {
        let club_id = team.club_id;
        let caretaker_id = team.staffs.best_coach_id(|s| {
            s.staff_attributes.coaching.tactical as u32
                + s.staff_attributes.mental.man_management as u32
                + s.staff_attributes.mental.motivating as u32
                + s.staff_attributes.coaching.mental as u32
        });
        let Some(id) = caretaker_id else {
            return false;
        };
        let Some(staff) = team.staffs.find_mut(id) else {
            return false;
        };
        let current_salary = staff.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let salary = current_salary.max(prior_salary / 2);
        let expires = today
            .checked_add_signed(Duration::days(60))
            .unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(today.year() + 1, today.month(), 1).unwrap()
            });
        staff.contract = Some(StaffClubContract::new(
            salary,
            expires,
            StaffPosition::CaretakerManager,
            StaffStatus::Active,
        ));
        // A spell in charge he did not ask for and may not keep. It is
        // still the first line of a manager's career for a great many
        // of them, so it is worth remembering.
        staff.remember(
            EpisodeKind::CaretakerSpell,
            ActorRef::club(club_id),
            today,
            club_id,
        );
        info!("Promoted staff {} to caretaker manager", id);
        true
    }

    /// Build a Manager contract for a freshly-signed candidate. Three-year
    /// term, salary as agreed. Status is `Active`.
    pub fn build_manager_contract(salary: u32, today: NaiveDate) -> StaffClubContract {
        let expires = today.with_year(today.year() + 3).unwrap_or(today);
        StaffClubContract::new(salary, expires, StaffPosition::Manager, StaffStatus::Active)
    }

    /// Id base for synthetic emergency caretakers. Sits far above any
    /// generator-allocated staff id, and adding `club_id` keeps each club's
    /// emergency seat unique and stable across ticks (so it can never
    /// duplicate). Mirrors the project's high-id-range convention used
    /// elsewhere (e.g. domestic-cup ids at 800M+).
    const EMERGENCY_CARETAKER_ID_BASE: u32 = 900_000_000;

    /// Token interim salary — a stopgap caretaker isn't on a headline deal.
    const EMERGENCY_CARETAKER_SALARY: u32 = 24_000;

    /// Number of permanent `Manager` contracts on the team. The head-coach
    /// seat is unique by construction, so this is normally 0 or 1; anything
    /// higher is a bug the repair pass collapses back to one.
    pub fn manager_count(team: &Team) -> usize {
        team.staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| matches!(c.position, StaffPosition::Manager))
                    .unwrap_or(false)
            })
            .count()
    }

    /// Is an interim caretaker currently sitting in the head-coach seat?
    pub fn has_caretaker(team: &Team) -> bool {
        team.staffs
            .find_by_position(StaffPosition::CaretakerManager)
            .is_some()
    }

    /// Collapse duplicate permanent managers down to a single seat: keep the
    /// best-fitting manager (by `relevance_score_for`) and demote the rest to
    /// generic coaches, preserving their salaries. No-op with 0 or 1 manager.
    pub fn dedupe_managers(team: &mut Team) {
        let keep_id = team
            .staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| matches!(c.position, StaffPosition::Manager))
                    .unwrap_or(false)
            })
            .max_by_key(|s| s.relevance_score_for(&StaffPosition::Manager))
            .map(|s| s.id);
        let Some(keep_id) = keep_id else {
            return;
        };
        for staff in team.staffs.iter_mut() {
            let is_extra_manager = staff
                .contract
                .as_ref()
                .map(|c| matches!(c.position, StaffPosition::Manager))
                .unwrap_or(false)
                && staff.id != keep_id;
            if is_extra_manager {
                if let Some(c) = staff.contract.as_mut() {
                    c.position = StaffPosition::Coach;
                }
            }
        }
    }

    /// Install a minimal emergency caretaker when a club has lost its entire
    /// coaching staff and there is nobody internal to promote. Modelled on
    /// the staff-stub conventions but with a real, club-unique id and modest
    /// (not floor-1) attributes so the dugout isn't run by a phantom. The
    /// 60-day interim contract gives the manager market time to find a
    /// permanent appointment.
    pub fn install_emergency_caretaker(team: &mut Team, club_id: u32, today: NaiveDate) {
        let caretaker_id = Self::EMERGENCY_CARETAKER_ID_BASE.saturating_add(club_id);

        let mut staff = StaffStub::default();
        staff.id = caretaker_id;
        staff.full_name = FullName::new("Interim".to_string(), "Coach".to_string());
        staff.job_satisfaction = 50.0;

        // Lift the key man-management / tactical attributes off the stub
        // floor so selection, training and morale read a journeyman caretaker
        // rather than a 1-rated ghost.
        staff.staff_attributes.coaching.tactical = 7;
        staff.staff_attributes.coaching.mental = 7;
        staff.staff_attributes.mental.man_management = 7;
        staff.staff_attributes.mental.motivating = 7;
        staff.staff_attributes.knowledge.tactical_knowledge = 7;

        let expires = today
            .checked_add_signed(Duration::days(60))
            .unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(today.year() + 1, today.month(), 1).unwrap()
            });
        staff.contract = Some(StaffClubContract::new(
            Self::EMERGENCY_CARETAKER_SALARY,
            expires,
            StaffPosition::CaretakerManager,
            StaffStatus::Active,
        ));
        team.staffs.push(staff);
        info!(
            "Installed emergency caretaker (staff {}) at club {}",
            caretaker_id, club_id
        );
    }
}
