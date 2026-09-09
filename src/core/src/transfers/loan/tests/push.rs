//! Moved verbatim out of `loan_market.rs` — see that file's `mod loan_push_gate_tests`.

use super::super::*;
use crate::academy::ClubAcademy;
use crate::club::player::builder::PlayerBuilder;
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason, TransferRequest};
use crate::{
    ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
    PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, StaffCollection, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
};
use chrono::{NaiveDate, NaiveTime};

struct PushFx;

impl PushFx {
    fn player(id: u32, roles: &[(PlayerPositionType, u8)], ca: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Loan".to_string(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: roles
                    .iter()
                    .map(|(position, level)| PlayerPosition {
                        position: *position,
                        level: *level,
                    })
                    .collect(),
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn team(players: Vec<Player>, world_rep: u16) -> Team {
        Team::builder()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Borrower".to_string())
            .slug("borrower".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(world_rep, world_rep, world_rep))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    /// A Continental giant whose front line is carried by wide forwards
    /// filed as midfielders, with only makeshift centre-forwards actually
    /// wearing a forward label. Spartak's shape, in miniature.
    fn giant_with_hidden_attack() -> Club {
        let main = Self::team(
            vec![
                // The real attack — natural centre-forwards, filed under
                // Midfielder because their record leads with a wing.
                Self::player(
                    1,
                    &[
                        (PlayerPositionType::AttackingMidfielderRight, 20),
                        (PlayerPositionType::Striker, 20),
                    ],
                    150,
                ),
                Self::player(
                    2,
                    &[
                        (PlayerPositionType::AttackingMidfielderLeft, 20),
                        (PlayerPositionType::Striker, 20),
                    ],
                    148,
                ),
                Self::player(
                    3,
                    &[
                        (PlayerPositionType::AttackingMidfielderCenter, 20),
                        (PlayerPositionType::Striker, 20),
                    ],
                    145,
                ),
                // …and the forward line as the label sees it.
                Self::player(4, &[(PlayerPositionType::Striker, 20)], 92),
                Self::player(5, &[(PlayerPositionType::Striker, 20)], 90),
                // Enough bodies elsewhere that no group is in crisis.
                Self::player(6, &[(PlayerPositionType::Goalkeeper, 20)], 130),
                Self::player(7, &[(PlayerPositionType::Goalkeeper, 20)], 125),
                Self::player(8, &[(PlayerPositionType::DefenderCenter, 20)], 130),
                Self::player(9, &[(PlayerPositionType::DefenderLeft, 20)], 130),
                Self::player(10, &[(PlayerPositionType::DefenderRight, 20)], 130),
                Self::player(11, &[(PlayerPositionType::DefensiveMidfielder, 20)], 130),
                Self::player(12, &[(PlayerPositionType::MidfielderCenter, 20)], 130),
                Self::player(13, &[(PlayerPositionType::MidfielderLeft, 20)], 130),
                Self::player(14, &[(PlayerPositionType::MidfielderRight, 20)], 130),
            ],
            7600,
        );
        let mut club = Club::new(
            1,
            "Giant FC".to_string(),
            Location::new(1),
            ClubFinances::new(100_000_000, Vec::new()),
            ClubAcademy::new(10),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![main]),
            ClubFacilities::default(),
        );
        club.transfer_plan.initialized = true;
        club
    }

    fn main_team(club: &Club) -> &Team {
        club.teams.main().expect("fixture has a main team")
    }

    /// The club's own shopping list: a centre-forward good enough to lead
    /// the line, which is what a giant with this attack would ask for.
    fn striker_request(min_ability: u8) -> TransferRequest {
        TransferRequest::new(
            1,
            PlayerPositionType::Striker,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
            min_ability,
            min_ability + 13,
            35_000_000.0,
        )
    }
}

/// The minutes gate used to count only the men FILED as forwards, so a
/// squad whose front line is three outstanding wide forwards read as
/// having nobody up front — and became the most attractive destination in
/// the country for other clubs' teenage strikers precisely because its
/// attack looked empty.
#[test]
fn a_hidden_attack_still_blocks_a_teenage_striker_loan() {
    let club = PushFx::giant_with_hidden_attack();
    let team = PushFx::main_team(&club);

    let labelled_forwards = team
        .players
        .iter()
        .filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Forward)
        .count();
    assert_eq!(
        labelled_forwards, 2,
        "precondition: only the makeshift men carry a forward label"
    );

    let depth = BorrowerPositionDepth::snapshot(team);
    assert!(
        !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Forward, 95, true, 0),
        "three natural centre-forwards ahead of him is not a route to minutes"
    );
}

/// …and the gate still opens where the competition is genuinely thin.
#[test]
fn a_thin_attack_still_admits_a_development_loan() {
    let club = PushFx::giant_with_hidden_attack();
    let mut team = PushFx::main_team(&club).clone();
    team.players.players.retain(|p| p.id > 3);

    let depth = BorrowerPositionDepth::snapshot(&team);
    assert!(
        depth.would_get_loan_minutes(PlayerFieldPositionGroup::Forward, 95, true, 0),
        "with the wide forwards gone he is competing for the shirt"
    );
}

/// The push used to ask the borrower nothing at all. A Continental club
/// that shops the loan market only in January, and only while in the red,
/// was handed teenagers in August because it happened to be the biggest
/// name that would play them.
#[test]
fn a_club_that_does_not_shop_is_not_a_destination() {
    let club = PushFx::giant_with_hidden_attack();
    let appetite = LoanBorrowerAppetite::assess(&club, PushFx::main_team(&club), false);
    assert!(
        !appetite.scans,
        "precondition: this club runs no loan scans"
    );
    assert!(
        !appetite.critical_shortage,
        "precondition: no group is below a fieldable minimum"
    );

    assert!(
        !appetite.accepts_push(&club, PlayerFieldPositionGroup::Forward, 92, 17, 0, false),
        "reputation makes a club attractive; it is not consent"
    );
}

/// An open request opens the door — but only to the player it was asking
/// for. A side shopping for a centre-forward who can lead its line has not
/// agreed to take any centre-forward alive.
#[test]
fn an_open_request_admits_only_the_player_it_asked_for() {
    let mut club = PushFx::giant_with_hidden_attack();
    club.transfer_plan
        .transfer_requests
        .push(PushFx::striker_request(129));
    let team_rep_snapshot = PushFx::main_team(&club).reputation.world;
    assert_eq!(team_rep_snapshot, 7600);

    let appetite = LoanBorrowerAppetite::assess(&club, PushFx::main_team(&club), false);
    assert!(
        !appetite.accepts_push(&club, PlayerFieldPositionGroup::Forward, 92, 17, 0, false),
        "a 92-rated seventeen-year-old is not what a 129-minimum brief asked for"
    );
    assert!(
        appetite.accepts_push(&club, PlayerFieldPositionGroup::Forward, 132, 24, 0, false),
        "…and the centre-forward it did ask for is welcome"
    );
    assert!(
        !appetite.accepts_push(&club, PlayerFieldPositionGroup::Defender, 132, 24, 0, false),
        "the request was for a forward, not a defender"
    );
}

/// A club that genuinely cannot field a balanced side takes what it is
/// offered — the emergency arm has to keep working.
#[test]
fn a_club_in_crisis_still_takes_what_it_is_offered() {
    let club = PushFx::giant_with_hidden_attack();
    let mut team = PushFx::main_team(&club).clone();
    team.players
        .players
        .retain(|p| p.position().position_group() != PlayerFieldPositionGroup::Forward);

    let appetite = LoanBorrowerAppetite::assess(&club, &team, false);
    assert!(
        appetite.critical_shortage,
        "a side with no forwards at all is short"
    );
    assert!(appetite.accepts_push(&club, PlayerFieldPositionGroup::Forward, 92, 17, 0, false));
}

/// WI-7: the upgrade acceptance. A club that "only loans in January,
/// and only while in the red" does not turn down a genuine first-team
/// upgrade in August — and its refusal was exactly what walked a
/// giant's near-ready youngster down to the tier below, a fortnight at
/// a time.
#[test]
fn a_clear_upgrade_is_welcome_in_any_month() {
    let club = PushFx::giant_with_hidden_attack();
    let appetite = LoanBorrowerAppetite::assess(&club, PushFx::main_team(&club), false);
    assert!(
        !appetite.scans,
        "precondition: this club runs no loan scans"
    );

    let best_here = 130u8;
    assert!(
        appetite.accepts_push(
            &club,
            PlayerFieldPositionGroup::Forward,
            best_here + LoanBorrowerAppetite::UPGRADE_MARGIN,
            21,
            best_here,
            true,
        ),
        "a loanee clearly better than anything here, whose wage it can carry"
    );
    assert!(
        !appetite.accepts_push(
            &club,
            PlayerFieldPositionGroup::Forward,
            best_here + LoanBorrowerAppetite::UPGRADE_MARGIN,
            21,
            best_here,
            false,
        ),
        "…but only inside the guard's reach: an unaffordable upgrade is not one"
    );
    assert!(
        !appetite.accepts_push(
            &club,
            PlayerFieldPositionGroup::Forward,
            best_here,
            21,
            best_here,
            true,
        ),
        "a comparable body is not an upgrade, guard or no guard"
    );
}
