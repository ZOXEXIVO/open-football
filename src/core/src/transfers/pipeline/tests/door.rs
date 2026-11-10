//! The door: every write of attention state hands in the gate's reading,
//! is refused under the stage its kind of file needs, and leaves the
//! reading behind whether it acted or not.

use crate::transfers::gate::{TransferMoveStage, TransferPlausibilityReason};
use crate::transfers::pipeline::{
    ClubTransferPlan, FileObservation, FileReading, RecommendationSource, RecommendationType,
    ShortlistCandidate, ShortlistCandidateStatus, StaffRecommendation, TransferShortlist,
    WatchAvailability, WatchlistEntry,
};
use crate::transfers::scouting::recruitment::ScoutMonitoringSource;
use crate::transfers::squad::plan::BriefTier;
use crate::transfers::tests::kit::TestDate;
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

const PLAYER: u32 = 30;

struct Fx;

impl Fx {
    fn reading(stage: TransferMoveStage) -> FileReading {
        FileReading {
            stage,
            stopped_by: (stage == TransferMoveStage::Blocked
                || stage == TransferMoveStage::CanScoutQuietly)
                .then_some(TransferPlausibilityReason::BelowHisLevel),
            read_on: TestDate::today(),
        }
    }

    fn seen() -> FileObservation {
        FileObservation {
            scout_staff_id: 1,
            player_id: PLAYER,
            source: ScoutMonitoringSource::StaffRecommendation,
            transfer_request_id: None,
            origin_assignment_id: None,
            region: None,
            assessed_ability: 110,
            assessed_potential: 120,
            confidence: 0.7,
            role_fit: 1.0,
            estimated_value: 500_000.0,
            risk_flags: Vec::new(),
            is_match: false,
        }
    }

    fn entry() -> WatchlistEntry {
        WatchlistEntry {
            player_id: PLAYER,
            club_id: 2,
            country_id: 1,
            position: PlayerPositionType::MidfielderCenter,
            position_group: PlayerFieldPositionGroup::Midfielder,
            age: 24,
            believed_ability: 110,
            estimated_value: 500_000.0,
            score: 10.0,
            availability: WatchAvailability::None,
            slot_position: PlayerPositionType::MidfielderCenter,
            slot_tier: BriefTier::B,
            last_refreshed: TestDate::today(),
            is_loan: false,
        }
    }

    fn recommendation() -> StaffRecommendation {
        StaffRecommendation {
            player_id: PLAYER,
            recommender_staff_id: 1,
            source: RecommendationSource::ScoutNetwork,
            recommendation_type: RecommendationType::ReadyForStepUp,
            assessed_ability: 110,
            assessed_potential: 120,
            confidence: 0.7,
            estimated_fee: 500_000.0,
            date_recommended: TestDate::today(),
        }
    }

    fn candidate() -> ShortlistCandidate {
        ShortlistCandidate {
            player_id: PLAYER,
            score: 0.5,
            estimated_fee: 500_000.0,
            status: ShortlistCandidateStatus::Available,
        }
    }

    fn on_shortlist(plan: &ClubTransferPlan) -> bool {
        plan.shortlists
            .iter()
            .any(|s| s.candidates.iter().any(|c| c.player_id == PLAYER))
    }

    /// Each door, its file's presence, and the stage under which it must
    /// refuse.
    fn doors() -> Vec<(
        &'static str,
        fn(&mut ClubTransferPlan, FileReading) -> bool,
        fn(&ClubTransferPlan) -> bool,
        TransferMoveStage,
    )> {
        vec![
            (
                "keep_file",
                |plan, reading| plan.keep_file(reading, Fx::seen(), TestDate::today()),
                |plan| !plan.monitorings_for_player(PLAYER).is_empty(),
                TransferMoveStage::Blocked,
            ),
            (
                "board",
                |plan, reading| plan.board(reading, Fx::entry()),
                |plan| plan.watchlist.iter().any(|e| e.player_id == PLAYER),
                TransferMoveStage::Blocked,
            ),
            (
                "recommend",
                |plan, reading| plan.recommend(reading, Fx::recommendation()),
                |plan| {
                    plan.staff_recommendations
                        .iter()
                        .any(|r| r.player_id == PLAYER)
                },
                TransferMoveStage::Blocked,
            ),
            (
                "shortlist",
                |plan, reading| plan.shortlist(reading, 5, 1_000_000.0, Fx::candidate(), None),
                Fx::on_shortlist,
                TransferMoveStage::CanScoutQuietly,
            ),
        ]
    }
}

#[test]
fn each_door_refuses_a_reading_under_its_stage_and_remembers_it() {
    for (name, open, holds, refused_at) in Fx::doors() {
        let mut plan = ClubTransferPlan::new();
        let reading = Fx::reading(refused_at);

        assert!(!open(&mut plan, reading), "{name} acted under its stage");
        assert!(!holds(&plan), "{name} wrote a file it refused");
        assert_eq!(plan.file_reading(PLAYER), Some(&reading), "{name}");
    }
}

#[test]
fn each_door_acts_on_a_reading_at_its_stage_and_remembers_it() {
    for (name, open, holds, refused_at) in Fx::doors() {
        let stage = match refused_at {
            TransferMoveStage::Blocked => TransferMoveStage::CanScoutQuietly,
            _ => TransferMoveStage::CanShortlistInternally,
        };
        let mut plan = ClubTransferPlan::new();
        let reading = Fx::reading(stage);

        assert!(open(&mut plan, reading), "{name} refused at its stage");
        assert!(holds(&plan), "{name} acted but wrote nothing");
        assert_eq!(plan.file_reading(PLAYER), Some(&reading), "{name}");
    }
}

#[test]
fn a_later_reading_replaces_the_one_remembered() {
    let mut plan = ClubTransferPlan::new();
    plan.keep_file(
        Fx::reading(TransferMoveStage::CanCompleteMove),
        Fx::seen(),
        TestDate::today(),
    );
    let refused = Fx::reading(TransferMoveStage::Blocked);

    plan.board(refused, Fx::entry());

    assert_eq!(plan.file_reading(PLAYER), Some(&refused));
    assert_eq!(
        plan.file_reading(PLAYER).and_then(|r| r.stopped_by),
        Some(TransferPlausibilityReason::BelowHisLevel)
    );
}

#[test]
fn a_built_shortlist_keeps_only_the_names_read_that_far() {
    let mut plan = ClubTransferPlan::new();
    let mut list = TransferShortlist::new(5, 1_000_000.0);
    list.candidates.push(Fx::candidate());
    list.candidates.push(ShortlistCandidate {
        player_id: PLAYER + 1,
        ..Fx::candidate()
    });

    assert!(plan.open_shortlist(
        list,
        &[
            (PLAYER, Fx::reading(TransferMoveStage::CanScoutQuietly)),
            (
                PLAYER + 1,
                Fx::reading(TransferMoveStage::CanShortlistInternally)
            ),
        ],
    ));

    let names: Vec<u32> = plan.shortlists[0]
        .candidates
        .iter()
        .map(|c| c.player_id)
        .collect();
    assert_eq!(names, vec![PLAYER + 1]);
    assert!(plan.file_reading(PLAYER).is_some());
}

#[test]
fn a_shortlist_with_no_name_read_that_far_is_not_opened() {
    let mut plan = ClubTransferPlan::new();
    let mut list = TransferShortlist::new(5, 1_000_000.0);
    list.candidates.push(Fx::candidate());

    assert!(!plan.open_shortlist(
        list,
        &[(PLAYER, Fx::reading(TransferMoveStage::CanScoutQuietly))],
    ));
    assert!(plan.shortlists.is_empty());
}
