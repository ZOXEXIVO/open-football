//! Global free-agent staff pool.
//!
//! Coaches who exit a club seat — sacked, contract expired, resigned —
//! are moved here rather than parked on their old club's roster. The
//! manager-market pipeline (`board::manager_market`) shortlists from
//! this pool when a club's seat opens up, mirroring the way the player
//! transfer pipeline reads from `SimulatorData.free_agents`.
//!
//! All entries are bare `Staff` values — same type as a contracted
//! coach. The fact that `contract` is `None` is what marks them as a
//! free agent. Aging, retirement, and reputation decay run via
//! `tick_free_agent_staff_pool`.

use crate::club::staff::contract::StaffPosition;
use crate::utils::DateUtils;
use crate::Staff;
use chrono::NaiveDate;
use log::debug;

/// Hard ceiling on age — coaches retire if they're still unsigned past
/// this birthday. Real-world managers occasionally work into their 70s
/// (Capello, Trapattoni) so we set it generously rather than at 65.
const COACH_RETIREMENT_AGE: u8 = 72;

/// Soft retirement: an unsigned coach over 65 who's been out of a job
/// for this many years gives up and retires. Picks up the post-career
/// fade-out without forcing it on coaches who are actively in demand.
const SOFT_RETIREMENT_YEARS_UNSIGNED: i64 = 3;

/// Per-day decay applied to `job_satisfaction` for unsigned coaches.
/// Long unemployment slowly erodes their negotiating leverage —
/// shortlist scoring reads `job_satisfaction` as a soft proxy for
/// "still wants top jobs", so a decayed coach is more willing to
/// accept lesser appointments.
const JOB_SATISFACTION_DAILY_DECAY: f32 = 0.02;

/// Date the staff member entered the pool, recorded on first sight.
/// We don't have a dedicated field on `Staff` for this — instead we
/// piggyback on the existing `recent_performance.last_evaluation_date`,
/// which is otherwise unused for unsigned coaches and is already
/// `Option<NaiveDate>`. Avoids growing the `Staff` struct just for
/// pool bookkeeping.
fn pool_entry_date(staff: &Staff) -> Option<NaiveDate> {
    staff.recent_performance.last_evaluation_date
}

fn set_pool_entry_date(staff: &mut Staff, date: NaiveDate) {
    staff.recent_performance.last_evaluation_date = Some(date);
}

/// Move a staff member into the pool. Clears their contract and resets
/// their player-relationship map so the dangling references to their
/// former squad don't accumulate. Caller is responsible for already
/// having removed them from their previous team's `StaffCollection`.
pub fn admit_to_pool(pool: &mut Vec<Staff>, mut staff: Staff, today: NaiveDate) {
    staff.contract = None;
    // Wipe relations — they refer to the previous squad's player ids
    // which the pool member no longer has any line of sight to.
    staff.relations = crate::Relations::new();
    // Drop fatigue accrued from the last job; a free agent is "rested".
    staff.fatigue = 0.0;
    // Job satisfaction stays — a happy departing coach is more selective
    // than a bitterly-sacked one, which the shortlist scorer reads later.
    set_pool_entry_date(&mut staff, today);
    debug!(
        "Free-agent pool: admitted staff id {} (age {}, satisfaction {:.0})",
        staff.id,
        DateUtils::age(staff.birth_date, today),
        staff.job_satisfaction
    );
    pool.push(staff);
}

/// Daily tick over every pool entry: decay satisfaction and retire the
/// over-the-hill ones. `O(N)` over the pool — the pool is small (a
/// few thousand globally at most), so a linear sweep per day is fine.
pub fn tick_free_agent_staff_pool(pool: &mut Vec<Staff>, today: NaiveDate) {
    pool.retain_mut(|staff| {
        let age = DateUtils::age(staff.birth_date, today);

        if age >= COACH_RETIREMENT_AGE {
            debug!("Free-agent pool: retired staff id {} at age {}", staff.id, age);
            return false;
        }

        if age >= 65 {
            let years_unsigned = pool_entry_date(staff)
                .map(|d| (today - d).num_days() / 365)
                .unwrap_or(0);
            if years_unsigned >= SOFT_RETIREMENT_YEARS_UNSIGNED {
                debug!(
                    "Free-agent pool: soft-retired staff id {} (age {}, {}y unsigned)",
                    staff.id, age, years_unsigned
                );
                return false;
            }
        }

        staff.job_satisfaction =
            (staff.job_satisfaction - JOB_SATISFACTION_DAILY_DECAY).max(0.0);

        true
    });
}

/// Sweep every team's `StaffCollection` and move fully-expired
/// non-manager contracts into the pool. The Manager and CaretakerManager
/// seats are excluded — those flow through the board's sacking /
/// confirmation pipeline so we don't double-handle them.
///
/// Returns the number of staff moved this tick. Cheap when nothing
/// expires (the common case): just a contract-date comparison per
/// staff member.
pub fn harvest_expired_staff(
    data: &mut crate::SimulatorData,
    today: NaiveDate,
) -> usize {
    let mut harvested: Vec<Staff> = Vec::new();

    for continent in &mut data.continents {
        for country in &mut continent.countries {
            for club in &mut country.clubs {
                for team in club.teams.iter_mut() {
                    // Snapshot of expiring ids — can't mutate while iterating.
                    let expired_ids: Vec<u32> = team
                        .staffs
                        .iter()
                        .filter(|s| {
                            let Some(c) = &s.contract else { return false; };
                            // Skip manager-seat handling — the board owns it.
                            if matches!(
                                c.position,
                                StaffPosition::Manager | StaffPosition::CaretakerManager
                            ) {
                                return false;
                            }
                            c.expired < today
                        })
                        .map(|s| s.id)
                        .collect();

                    for id in expired_ids {
                        if let Some(staff) = team.staffs.take_by_id(id) {
                            harvested.push(staff);
                        }
                    }
                }
            }
        }
    }

    let n = harvested.len();
    for staff in harvested {
        admit_to_pool(&mut data.free_agent_staff, staff, today);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::staff::contract::{StaffClubContract, StaffStatus};
    use crate::club::StaffStub;
    use chrono::{Datelike, NaiveDate};

    fn make_staff(id: u32, age: u8, today: NaiveDate) -> Staff {
        let mut staff = StaffStub::default();
        staff.id = id;
        staff.birth_date = NaiveDate::from_ymd_opt(today.year() - age as i32, 1, 1).unwrap();
        staff
    }

    #[test]
    fn admit_clears_contract_and_relations() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let mut staff = make_staff(1, 50, today);
        staff.contract = Some(StaffClubContract::new(
            100_000,
            today,
            StaffPosition::Manager,
            StaffStatus::Active,
        ));
        staff.fatigue = 70.0;

        let mut pool = Vec::new();
        admit_to_pool(&mut pool, staff, today);

        assert_eq!(pool.len(), 1);
        assert!(pool[0].contract.is_none());
        assert_eq!(pool[0].fatigue, 0.0);
        assert_eq!(pool_entry_date(&pool[0]), Some(today));
    }

    #[test]
    fn pool_tick_retires_old_coaches() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let young = make_staff(1, 50, today);
        let old = make_staff(2, 75, today);
        let mut pool = vec![young, old];

        tick_free_agent_staff_pool(&mut pool, today);

        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].id, 1);
    }

    #[test]
    fn pool_tick_soft_retires_unemployed_seniors() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let admit_day = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(); // 4y ago
        let mut senior = make_staff(1, 67, today);
        set_pool_entry_date(&mut senior, admit_day);
        let mut pool = vec![senior];

        tick_free_agent_staff_pool(&mut pool, today);

        assert!(pool.is_empty());
    }

    #[test]
    fn pool_tick_decays_job_satisfaction() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let mut staff = make_staff(1, 45, today);
        staff.job_satisfaction = 50.0;
        let mut pool = vec![staff];

        tick_free_agent_staff_pool(&mut pool, today);

        assert!(pool[0].job_satisfaction < 50.0);
        assert!(pool[0].job_satisfaction > 49.9);
    }
}
