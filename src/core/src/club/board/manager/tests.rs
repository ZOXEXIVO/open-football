use super::*;
use crate::academy::ClubAcademy;
use crate::club::board::ClubBoard;
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind, FactClaim};
use crate::club::staff::{StaffClubContract, StaffPosition, StaffStatus};
use crate::club::{BoardResult, Club, StaffStub, Team};
use crate::competitions::GlobalCompetitions;
use crate::continent::Continent;
use crate::league::LeagueCollection;
use crate::shared::Location;
use crate::{
    ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PlayerCollection,
    StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TrainingSchedule,
};
use crate::{SimulatorData, Staff, TeamType};
use chrono::{Datelike, Duration, NaiveDate};
#[test]
fn search_window_scales_with_rep() {
    assert!(
        ManagerCandidateScorer::search_window_days(9000)
            > ManagerCandidateScorer::search_window_days(3000)
    );
    assert!(
        ManagerCandidateScorer::search_window_days(3000)
            > ManagerCandidateScorer::search_window_days(500)
    );
}

#[test]
fn higher_skill_scores_higher_at_matched_rep() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let weak = Fx::coach(1, 45, today, 8);
    let strong = Fx::coach(2, 45, today, 16);

    let weak_score = ManagerCandidateScorer::score_free_agent(&weak, 5000, today).unwrap();
    let strong_score = ManagerCandidateScorer::score_free_agent(&strong, 5000, today).unwrap();

    assert!(strong_score > weak_score);
}

#[test]
fn very_old_candidate_takes_age_penalty() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let young = Fx::coach(1, 45, today, 14);
    let old = Fx::coach(2, 68, today, 14);

    let ys = ManagerCandidateScorer::score_free_agent(&young, 5000, today).unwrap();
    let os = ManagerCandidateScorer::score_free_agent(&old, 5000, today).unwrap();
    assert!(ys > os);
}

#[test]
fn shortlist_returns_top_n_sorted() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let pool: Vec<Staff> = (1..=10).map(|i| Fx::coach(i, 45, today, i as u8)).collect();

    let shortlist = ManagerShortlist::from_free_agents(&pool, 6000, today);
    assert_eq!(shortlist.len(), ManagerShortlist::MAX_LEN);
    for w in shortlist.windows(2) {
        assert!(w[0].fit_score >= w[1].fit_score);
    }
    assert_eq!(shortlist[0].staff_id, 10);
}

#[test]
fn target_salary_grows_with_rep() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let s = Fx::coach(1, 45, today, 14);
    let small = ManagerCandidateScorer::target_salary(&s, 1500, today);
    let big = ManagerCandidateScorer::target_salary(&s, 8500, today);
    assert!(big > small * 3);
}

#[test]
fn take_top_free_agent_drains_pool_and_shortlist() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let pool_staff = Fx::coach(42, 45, today, 14);
    let mut pool = vec![pool_staff];

    let mut board = ClubBoard::new();
    board.manager_shortlist = vec![ManagerCandidate {
        staff_id: 42,
        fit_score: 100,
        target_salary: 250_000,
        source: CandidateSource::FreeAgent,
    }];

    let result = ManagerSearch::take_top_free_agent(&mut board, &mut pool);
    assert!(result.is_some());
    let (staff, salary) = result.unwrap();
    assert_eq!(staff.id, 42);
    assert_eq!(salary, 250_000);
    assert!(pool.is_empty());
    assert!(board.manager_shortlist.is_empty());
}

#[test]
fn build_manager_contract_runs_three_years() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let c: StaffClubContract = ManagerSeat::build_manager_contract(200_000, today);
    assert_eq!(c.salary, 200_000);
    assert_eq!(c.position, StaffPosition::Manager);
    assert_eq!(c.status, StaffStatus::Active);
    assert_eq!(c.expired.year(), today.year() + 3);
}

// ─── Slice C: poaching state-machine tests ───────────────────────────

#[test]
fn confident_overperforming_source_refuses() {
    assert!(ManagerCandidateScorer::source_refuses_outright(85, true));
    assert!(!ManagerCandidateScorer::source_refuses_outright(60, true));
    assert!(!ManagerCandidateScorer::source_refuses_outright(85, false));
}

#[test]
fn compensation_multiplier_scales_with_rep() {
    assert!(
        ManagerCandidateScorer::compensation_multiplier(8000)
            > ManagerCandidateScorer::compensation_multiplier(5000)
    );
    assert!(
        ManagerCandidateScorer::compensation_multiplier(5000)
            > ManagerCandidateScorer::compensation_multiplier(1000)
    );
}

#[test]
fn employed_candidate_takes_friction_penalty() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let s = Fx::coach(1, 45, today, 14);
    let free = ManagerCandidateScorer::score_free_agent(&s, 6000, today).unwrap();
    let employed = ManagerCandidateScorer::score_employed(&s, 6000, today).unwrap();
    assert!(employed < free);
}

/// The staff, clubs and world state the manager-market tests run on.
struct Fx;

impl Fx {
    fn coach(id: u32, age: u8, today: NaiveDate, skill: u8) -> Staff {
        let mut s = StaffStub::default();
        s.id = id;
        s.birth_date = NaiveDate::from_ymd_opt(today.year() - age as i32, 1, 1).unwrap();
        s.staff_attributes.coaching.tactical = skill;
        s.staff_attributes.coaching.mental = skill;
        s.staff_attributes.mental.man_management = skill;
        s.staff_attributes.mental.motivating = skill;
        s.staff_attributes.knowledge.tactical_knowledge = skill;
        s
    }

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn manager_contract(
        salary: u32,
        today: NaiveDate,
        position: StaffPosition,
    ) -> StaffClubContract {
        let expires = today.with_year(today.year() + 2).unwrap_or(today);
        StaffClubContract::new(salary, expires, position, StaffStatus::Active)
    }

    fn coach_with_contract(
        id: u32,
        today: NaiveDate,
        position: StaffPosition,
        salary: u32,
    ) -> Staff {
        let mut s = Self::coach(id, 45, today, 14);
        s.contract = Some(Self::manager_contract(salary, today, position));
        s
    }

    fn make_main_team(team_id: u32, club_id: u32, staffs: Vec<Staff>) -> Team {
        TeamBuilder::new()
            .id(team_id)
            .league_id(Some(1))
            .club_id(club_id)
            .name(format!("Team{}", team_id))
            .slug(format!("team{}", team_id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(staffs))
            .reputation(TeamReputation::new(3000, 3000, 3000))
            .training_schedule(Self::make_training_schedule())
            .build()
            .unwrap()
    }

    fn make_club_with_main(id: u32, staffs: Vec<Staff>) -> Club {
        let team = Self::make_main_team(id * 10, id, staffs);
        Club::new(
            id,
            format!("Club{}", id),
            Location::new(1),
            ClubFinances::new(10_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            ClubFacilities::default(),
        )
    }

    fn make_data(today: NaiveDate, clubs: Vec<Club>) -> SimulatorData {
        let country = Country::builder()
            .id(1)
            .code("EN".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(Vec::new()))
            .clubs(clubs)
            .build()
            .unwrap();
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            today.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn count_managers(data: &SimulatorData, club_id: u32) -> usize {
        let club = data.club(club_id).unwrap();
        let main = club.teams.main().unwrap();
        main.staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| matches!(c.position, StaffPosition::Manager))
                    .unwrap_or(false)
            })
            .count()
    }

    fn count_caretakers(data: &SimulatorData, club_id: u32) -> usize {
        let club = data.club(club_id).unwrap();
        let main = club.teams.main().unwrap();
        main.staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| matches!(c.position, StaffPosition::CaretakerManager))
                    .unwrap_or(false)
            })
            .count()
    }

    /// Combined head-coach seat count — anything filling the role,
    /// permanent or interim. The seat is unique by definition, so this
    /// must always be 0 or 1.
    fn count_head_coaches(data: &SimulatorData, club_id: u32) -> usize {
        Self::count_managers(data, club_id) + Self::count_caretakers(data, club_id)
    }

    fn coach_with_skill_and_role(
        id: u32,
        today: NaiveDate,
        skill: u8,
        position: StaffPosition,
        salary: u32,
    ) -> Staff {
        let mut s = Self::coach(id, 45, today, skill);
        s.contract = Some(Self::manager_contract(salary, today, position));
        s
    }

    fn coach_with_expired_contract(
        id: u32,
        today: NaiveDate,
        position: StaffPosition,
        salary: u32,
    ) -> Staff {
        let mut s = Self::coach(id, 45, today, 14);
        let expired = today - chrono::Duration::days(10);
        s.contract = Some(StaffClubContract::new(
            salary,
            expired,
            position,
            StaffStatus::Active,
        ));
        s
    }

    fn caretaker_id(data: &SimulatorData, club_id: u32) -> Option<u32> {
        data.club(club_id)
            .and_then(|c| c.teams.main())
            .and_then(|t| {
                t.staffs
                    .iter()
                    .find(|s| {
                        s.contract
                            .as_ref()
                            .map(|c| matches!(c.position, StaffPosition::CaretakerManager))
                            .unwrap_or(false)
                    })
                    .map(|s| s.id)
            })
    }

    fn main_staff_len(data: &SimulatorData, club_id: u32) -> usize {
        data.club(club_id)
            .and_then(|c| c.teams.main())
            .map(|t| t.staffs.len())
            .unwrap_or(0)
    }

    /// The manager the approach is for, at the source club.
    fn poachable(id: u32, today: NaiveDate, salary: u32) -> Staff {
        Self::coach_with_contract(id, today, StaffPosition::Manager, salary)
    }

    fn approach_for(staff_id: u32, offered_salary: u32) -> ManagerApproach {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        ManagerApproach {
            requesting_club_id: 1,
            source_club_id: 2,
            staff_id,
            state: ApproachState::CompensationAgreed,
            offered_salary,
            created_at: today,
            last_action: today,
            compensation_paid: None,
        }
    }

    /// Source club 2 (the manager's current employer) and requesting
    /// club 1, both at 3000 world reputation so prestige is a wash and
    /// the salary is the only number in play.
    fn market_with(manager: Staff) -> SimulatorData {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let requesting = Self::make_club_with_main(1, Vec::new());
        let source = Self::make_club_with_main(2, vec![manager]);
        Self::make_data(today, vec![requesting, source])
    }
}

#[test]
fn execute_appointment_does_not_add_second_manager_when_permanent_manager_exists() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let incumbent = Fx::coach_with_contract(100, today, StaffPosition::Manager, 200_000);
    let club = Fx::make_club_with_main(1, vec![incumbent]);
    let mut data = Fx::make_data(today, vec![club]);

    let candidate = Fx::coach(42, 45, today, 14);
    data.free_agent_staff.push(candidate);

    if let Some(club) = data.club_mut(1) {
        club.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 4, 1).unwrap());
        club.board.search_window_days = 30;
        club.board.manager_shortlist = vec![ManagerCandidate {
            staff_id: 42,
            fit_score: 100,
            target_salary: 250_000,
            source: CandidateSource::FreeAgent,
        }];
    }

    ManagerMarketTick::execute_appointment(&mut data, 1, today);

    assert_eq!(
        Fx::count_managers(&data, 1),
        1,
        "must still have exactly one permanent manager"
    );
    assert_eq!(
        Fx::count_head_coaches(&data, 1),
        1,
        "head-coach seat is unique"
    );
    assert!(
        data.free_agent_staff.iter().any(|s| s.id == 42),
        "candidate should remain in the pool"
    );
    let club = data.club(1).unwrap();
    assert!(club.board.manager_search_since.is_none());
    assert!(club.board.manager_shortlist.is_empty());
}

#[test]
fn execute_appointment_replaces_caretaker_with_permanent_manager() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let caretaker = Fx::coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
    let club = Fx::make_club_with_main(1, vec![caretaker]);
    let mut data = Fx::make_data(today, vec![club]);

    let candidate = Fx::coach(42, 45, today, 14);
    data.free_agent_staff.push(candidate);

    if let Some(club) = data.club_mut(1) {
        club.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 4, 1).unwrap());
        club.board.search_window_days = 30;
        club.board.manager_shortlist = vec![ManagerCandidate {
            staff_id: 42,
            fit_score: 100,
            target_salary: 250_000,
            source: CandidateSource::FreeAgent,
        }];
    }

    ManagerMarketTick::execute_appointment(&mut data, 1, today);

    assert_eq!(Fx::count_managers(&data, 1), 1);
    assert_eq!(Fx::count_caretakers(&data, 1), 0);
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    assert!(
        data.free_agent_staff.iter().all(|s| s.id != 42),
        "candidate should be drawn from the pool"
    );
    let club = data.club(1).unwrap();
    let main = club.teams.main().unwrap();
    assert!(main.staffs.iter().any(|s| s.id == 42));
    assert!(club.board.manager_search_since.is_none());
}

#[test]
fn finalize_approach_does_not_poach_into_filled_seat() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let source_mgr = Fx::coach_with_contract(200, today, StaffPosition::Manager, 300_000);
    let source = Fx::make_club_with_main(2, vec![source_mgr]);
    let req_mgr = Fx::coach_with_contract(100, today, StaffPosition::Manager, 250_000);
    let requesting = Fx::make_club_with_main(1, vec![req_mgr]);
    let mut data = Fx::make_data(today, vec![requesting, source]);

    let approach = ManagerApproach {
        requesting_club_id: 1,
        source_club_id: 2,
        staff_id: 200,
        state: ApproachState::TermsAccepted,
        offered_salary: 350_000,
        created_at: today - chrono::Duration::days(5),
        last_action: today - chrono::Duration::days(1),
        compensation_paid: None,
    };
    data.pending_manager_approaches.push(approach);

    ManagerMarketTick::tick_approaches(&mut data);

    assert_eq!(
        Fx::count_managers(&data, 2),
        1,
        "source manager must not be removed"
    );
    let src_club = data.club(2).unwrap();
    let src_main = src_club.teams.main().unwrap();
    assert!(
        src_main.staffs.iter().any(|s| s.id == 200),
        "target staff must remain on source roster"
    );
    assert_eq!(Fx::count_managers(&data, 1), 1);
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    assert!(src_club.board.manager_search_since.is_none());
    assert!(data.pending_manager_approaches.is_empty());
}

#[test]
fn finalize_approach_opens_source_search_after_successful_poach() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let source_mgr = Fx::coach_with_contract(200, today, StaffPosition::Manager, 300_000);
    let source_coach = Fx::coach_with_contract(201, today, StaffPosition::Coach, 60_000);
    let source = Fx::make_club_with_main(2, vec![source_mgr, source_coach]);
    let caretaker = Fx::coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
    let requesting = Fx::make_club_with_main(1, vec![caretaker]);
    let mut data = Fx::make_data(today, vec![requesting, source]);

    if let Some(req) = data.club_mut(1) {
        req.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 5, 1).unwrap());
        req.board.search_window_days = 30;
    }

    let approach = ManagerApproach {
        requesting_club_id: 1,
        source_club_id: 2,
        staff_id: 200,
        state: ApproachState::TermsAccepted,
        offered_salary: 350_000,
        created_at: today - chrono::Duration::days(5),
        last_action: today - chrono::Duration::days(1),
        compensation_paid: None,
    };
    data.pending_manager_approaches.push(approach);

    ManagerMarketTick::tick_approaches(&mut data);

    assert_eq!(Fx::count_managers(&data, 1), 1);
    assert_eq!(
        Fx::count_caretakers(&data, 1),
        0,
        "old caretaker must be demoted"
    );
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    let req_club = data.club(1).unwrap();
    assert!(req_club.board.manager_search_since.is_none());
    assert!(
        req_club
            .teams
            .main()
            .unwrap()
            .staffs
            .iter()
            .any(|s| s.id == 200)
    );

    let src_club = data.club(2).unwrap();
    assert_eq!(src_club.board.manager_search_since, Some(today));
    assert!(src_club.board.search_window_days > 0);
    assert_eq!(Fx::count_managers(&data, 2), 0);
    assert_eq!(
        Fx::count_caretakers(&data, 2),
        1,
        "source must get an interim caretaker"
    );
    assert_eq!(Fx::count_head_coaches(&data, 2), 1);
    assert!(
        src_club
            .teams
            .main()
            .unwrap()
            .staffs
            .iter()
            .all(|s| s.id != 200)
    );
}

#[test]
fn refresh_shortlists_clears_stale_search_when_permanent_manager_in_post() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let incumbent = Fx::coach_with_contract(100, today, StaffPosition::Manager, 200_000);
    let club = Fx::make_club_with_main(1, vec![incumbent]);
    let mut data = Fx::make_data(today, vec![club]);

    if let Some(club) = data.club_mut(1) {
        club.board.manager_search_since = Some(today - chrono::Duration::days(20));
        club.board.search_window_days = 30;
        club.board.manager_shortlist = vec![ManagerCandidate {
            staff_id: 999,
            fit_score: 50,
            target_salary: 100_000,
            source: CandidateSource::FreeAgent,
        }];
        club.board.shortlist_built_at = Some(today - chrono::Duration::days(20));
    }

    ManagerMarketTick::refresh_shortlists(&mut data);

    let club = data.club(1).unwrap();
    assert!(
        club.board.manager_search_since.is_none(),
        "stale search must be cleared when permanent manager is still in post"
    );
    assert!(club.board.manager_shortlist.is_empty());
}

#[test]
fn compensation_not_paid_when_requesting_seat_filled() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let source_mgr = Fx::coach_with_contract(200, today, StaffPosition::Manager, 300_000);
    let source = Fx::make_club_with_main(2, vec![source_mgr]);
    let req_mgr = Fx::coach_with_contract(100, today, StaffPosition::Manager, 250_000);
    let requesting = Fx::make_club_with_main(1, vec![req_mgr]);
    let mut data = Fx::make_data(today, vec![requesting, source]);

    let starting_balance = data.club(1).unwrap().finance.balance.balance;
    let starting_outcome = data.club(1).unwrap().finance.balance.outcome;

    let approach = ManagerApproach {
        requesting_club_id: 1,
        source_club_id: 2,
        staff_id: 200,
        state: ApproachState::CompensationDemanded { amount: 500_000 },
        offered_salary: 350_000,
        created_at: today - chrono::Duration::days(3),
        last_action: today - chrono::Duration::days(1),
        compensation_paid: None,
    };
    data.pending_manager_approaches.push(approach);

    ManagerMarketTick::tick_approaches(&mut data);

    let req = data.club(1).unwrap();
    assert_eq!(req.finance.balance.balance, starting_balance);
    assert_eq!(req.finance.balance.outcome, starting_outcome);
    assert_eq!(Fx::count_managers(&data, 2), 1);
    assert!(data.pending_manager_approaches.is_empty());
}

#[test]
fn approach_rejects_when_source_target_no_longer_manager() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let demoted = Fx::coach_with_contract(200, today, StaffPosition::Coach, 60_000);
    let real_mgr = Fx::coach_with_contract(201, today, StaffPosition::Manager, 280_000);
    let source = Fx::make_club_with_main(2, vec![demoted, real_mgr]);
    let caretaker = Fx::coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
    let requesting = Fx::make_club_with_main(1, vec![caretaker]);
    let mut data = Fx::make_data(today, vec![requesting, source]);

    if let Some(req) = data.club_mut(1) {
        req.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 5, 1).unwrap());
        req.board.search_window_days = 30;
    }

    let starting_balance = data.club(1).unwrap().finance.balance.balance;

    let approach = ManagerApproach {
        requesting_club_id: 1,
        source_club_id: 2,
        staff_id: 200,
        state: ApproachState::Made,
        offered_salary: 350_000,
        created_at: today - chrono::Duration::days(1),
        last_action: today - chrono::Duration::days(1),
        compensation_paid: None,
    };
    data.pending_manager_approaches.push(approach);

    ManagerMarketTick::tick_approaches(&mut data);

    assert_eq!(Fx::count_managers(&data, 2), 1);
    let src_club = data.club(2).unwrap();
    assert!(
        src_club.board.manager_search_since.is_none(),
        "no cascade for a rejected approach"
    );
    let src_main = src_club.teams.main().unwrap();
    assert!(src_main.staffs.iter().any(|s| s.id == 200));
    assert!(src_main.staffs.iter().any(|s| s.id == 201));
    assert_eq!(Fx::count_managers(&data, 1), 0);
    assert_eq!(Fx::count_caretakers(&data, 1), 1);
    assert_eq!(
        data.club(1).unwrap().finance.balance.balance,
        starting_balance
    );
    assert!(data.pending_manager_approaches.is_empty());
}

// ─── Manager-seat repair (vacancy invariant) tests ───────────────────

#[test]
fn run_repairs_empty_main_team_into_caretaker_with_open_search() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let club = Fx::make_club_with_main(1, Vec::new());
    let mut data = Fx::make_data(today, vec![club]);

    ManagerMarketTick::run(&mut data, today);

    assert!(
        Fx::main_staff_len(&data, 1) >= 1,
        "an emptied main team must not stay empty"
    );
    assert_eq!(Fx::count_caretakers(&data, 1), 1, "interim cover installed");
    assert_eq!(
        Fx::count_head_coaches(&data, 1),
        1,
        "head-coach seat is unique"
    );
    // Synthetic caretaker lives in the dedicated high-id range.
    let id = Fx::caretaker_id(&data, 1).unwrap();
    assert!(
        id >= 900_000_000,
        "expected emergency caretaker id, got {id}"
    );
    let club = data.club(1).unwrap();
    assert!(
        club.board.manager_search_since.is_some(),
        "board must open a search for the vacant seat"
    );
}

#[test]
fn repair_promotes_best_internal_coach_when_no_manager() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let weak = Fx::coach_with_skill_and_role(201, today, 8, StaffPosition::Coach, 50_000);
    let strong = Fx::coach_with_skill_and_role(202, today, 16, StaffPosition::Coach, 50_000);
    let club = Fx::make_club_with_main(1, vec![weak, strong]);
    let mut data = Fx::make_data(today, vec![club]);

    ManagerSeatRepair::run(&mut data, today);

    assert_eq!(Fx::count_caretakers(&data, 1), 1);
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    assert_eq!(
        Fx::caretaker_id(&data, 1),
        Some(202),
        "the stronger coach should step up, not a synthetic caretaker"
    );
    assert!(data.club(1).unwrap().board.manager_search_since.is_some());
}

#[test]
fn repair_opens_search_for_caretaker_without_manager() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let caretaker = Fx::coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
    let club = Fx::make_club_with_main(1, vec![caretaker]);
    let mut data = Fx::make_data(today, vec![club]);
    assert!(data.club(1).unwrap().board.manager_search_since.is_none());

    ManagerSeatRepair::run(&mut data, today);

    assert_eq!(
        Fx::count_caretakers(&data, 1),
        1,
        "caretaker is not disturbed"
    );
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    assert!(
        data.club(1).unwrap().board.manager_search_since.is_some(),
        "a caretaker-run club must have an open manager search"
    );
}

#[test]
fn repair_clears_stale_search_and_keeps_single_manager() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let manager = Fx::coach_with_contract(100, today, StaffPosition::Manager, 200_000);
    let club = Fx::make_club_with_main(1, vec![manager]);
    let mut data = Fx::make_data(today, vec![club]);
    if let Some(club) = data.club_mut(1) {
        club.board.manager_search_since = Some(today - chrono::Duration::days(5));
        club.board.search_window_days = 30;
    }

    ManagerSeatRepair::run(&mut data, today);

    assert_eq!(Fx::count_managers(&data, 1), 1, "no second manager appears");
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    assert!(
        data.club(1).unwrap().board.manager_search_since.is_none(),
        "stale search cleared while a permanent manager is in post"
    );
}

#[test]
fn repair_collapses_duplicate_permanent_managers() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let weak = Fx::coach_with_skill_and_role(101, today, 8, StaffPosition::Manager, 200_000);
    let strong = Fx::coach_with_skill_and_role(102, today, 16, StaffPosition::Manager, 200_000);
    let club = Fx::make_club_with_main(1, vec![weak, strong]);
    let mut data = Fx::make_data(today, vec![club]);

    ManagerSeatRepair::run(&mut data, today);

    assert_eq!(
        Fx::count_managers(&data, 1),
        1,
        "duplicate managers collapse to a single seat"
    );
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    // The stronger candidate keeps the seat.
    let main = data.club(1).unwrap().teams.main().unwrap();
    let kept = main
        .staffs
        .iter()
        .find(|s| {
            s.contract
                .as_ref()
                .map(|c| matches!(c.position, StaffPosition::Manager))
                .unwrap_or(false)
        })
        .unwrap();
    assert_eq!(kept.id, 102);
}

#[test]
fn harvest_then_repair_recovers_an_emptied_main_team() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    // No manager; only expiring non-manager staff that the harvest will
    // sweep into the free-agent pool, emptying the main team.
    let coach = Fx::coach_with_expired_contract(201, today, StaffPosition::Coach, 60_000);
    let physio = Fx::coach_with_expired_contract(202, today, StaffPosition::Physio, 30_000);
    let club = Fx::make_club_with_main(1, vec![coach, physio]);
    let mut data = Fx::make_data(today, vec![club]);

    ManagerMarketTick::run(&mut data, today);

    // The expired staff were harvested...
    assert!(data.free_agent_staff.iter().any(|s| s.id == 201));
    assert!(data.free_agent_staff.iter().any(|s| s.id == 202));
    // ...but the club did not end the tick unmanaged.
    assert!(Fx::main_staff_len(&data, 1) >= 1);
    assert_eq!(Fx::count_caretakers(&data, 1), 1);
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    assert!(data.club(1).unwrap().board.manager_search_since.is_some());
}

#[test]
fn sacking_removes_manager_pools_him_and_promotes_caretaker() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let manager = Fx::coach_with_contract(100, today, StaffPosition::Manager, 200_000);
    let coach = Fx::coach_with_contract(201, today, StaffPosition::Coach, 60_000);
    let club = Fx::make_club_with_main(1, vec![manager, coach]);
    let mut data = Fx::make_data(today, vec![club]);

    let mut result = BoardResult::new();
    result.club_id = 1;
    result.manager_sacked = true;
    result.process(&mut data);

    // Manager off the roster and into the global free-agent pool.
    assert_eq!(Fx::count_managers(&data, 1), 0);
    assert!(
        data.free_agent_staff.iter().any(|s| s.id == 100),
        "sacked manager joins the free-agent staff pool"
    );
    // Best internal coach steps up; the seat stays filled and unique.
    assert_eq!(Fx::count_caretakers(&data, 1), 1);
    assert_eq!(Fx::count_head_coaches(&data, 1), 1);
    // Board opens its search.
    assert!(data.club(1).unwrap().board.manager_search_since.is_some());
}

// ── Personal terms, now with a memory ───────────────────────

#[test]
fn a_manager_with_no_history_of_the_club_gets_exactly_the_old_answer() {
    // The conservatism the conversion rests on: an empty verdict
    // means the three booleans decide, unchanged. Every manager in
    // a fresh world is this manager.
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();

    let good_money = Fx::market_with(Fx::poachable(200, today, 100_000));
    assert!(ManagerCandidateScorer::candidate_accepts_terms(
        &good_money,
        &Fx::approach_for(200, 200_000)
    ));

    let bad_money = Fx::market_with(Fx::poachable(200, today, 100_000));
    assert!(!ManagerCandidateScorer::candidate_accepts_terms(
        &bad_money,
        &Fx::approach_for(200, 100_000)
    ));
}

#[test]
fn a_manager_they_starved_turns_down_money_he_would_otherwise_take() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let mut manager = Fx::poachable(200, today, 100_000);

    // Four windows of being told no at club 1, consolidated into
    // the conviction a manager actually carries between jobs.
    for window in 0..4 {
        manager.remember(
            EpisodeKind::BoardRefusedMyTarget,
            ActorRef::board(1),
            today - Duration::days(400 - window * 60),
            1,
        );
    }
    let ctx = manager.mind_context(today, 2);
    manager.mind.memory_mut().maybe_consolidate(&ctx.memory());
    assert!(
        manager
            .mind
            .believes(FactClaim::TheyNeverBackedMe, ActorRef::club(1))
            > 0.0,
        "the conviction has to exist for the test to mean anything"
    );

    let data = Fx::market_with(manager);
    // A 100% pay rise — the old check accepts this without a pause.
    assert!(
        !ManagerCandidateScorer::candidate_accepts_terms(&data, &Fx::approach_for(200, 200_000)),
        "he has been there, and the money is not the point"
    );
}

#[test]
fn a_club_that_sacked_him_is_a_job_he_turns_down() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let mut manager = Fx::poachable(200, today, 100_000);

    manager.remember(
        EpisodeKind::SackedByClub,
        ActorRef::club(1),
        today - Duration::days(700),
        1,
    );
    manager.leave_club(1);
    let ctx = manager.mind_context(today, 2);
    manager.mind.memory_mut().maybe_consolidate(&ctx.memory());
    assert!(
        manager
            .mind
            .believes(FactClaim::TheySackedMe, ActorRef::club(1))
            > 0.0
    );

    let data = Fx::market_with(manager);
    assert!(
        !ManagerCandidateScorer::candidate_accepts_terms(&data, &Fx::approach_for(200, 200_000)),
        "a doubled salary does not buy back a sacking"
    );
}

#[test]
fn a_place_he_built_something_takes_a_job_the_numbers_say_no_to() {
    let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
    let mut manager = Fx::poachable(200, today, 100_000);

    // He took them up. That is the one thing that outlives
    // everything else about a spell.
    manager.remember(
        EpisodeKind::Promoted,
        ActorRef::club(1),
        today - Duration::days(1_400),
        1,
    );
    manager.leave_club(1);
    let ctx = manager.mind_context(today, 2);
    manager.mind.memory_mut().maybe_consolidate(&ctx.memory());
    assert!(
        manager
            .mind
            .believes(FactClaim::IBuiltSomethingThere, ActorRef::club(1))
            > 0.0
    );

    let data = Fx::market_with(manager);
    // Flat salary, matched prestige: the three booleans say no.
    assert!(
        ManagerCandidateScorer::candidate_accepts_terms(&data, &Fx::approach_for(200, 100_000)),
        "there is a reason to go back that no number expresses"
    );
}
