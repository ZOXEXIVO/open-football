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

/// The push used to ask the borrower nothing at all, and the borrower's
/// own scan asked "does this club shop at all" and walked away from a
/// no. Both are prices now, so what is left to assert is that the price
/// moves in the right direction.
///
/// A club that shops rarely is still a destination — it wants a loanee
/// less than a small club does, which is a smaller number rather than a
/// closed door, and the refusal was exactly what walked a giant's
/// near-ready youngster down two divisions a fortnight at a time.
#[test]
fn a_club_that_shops_rarely_is_still_a_destination() {
    let elite = BorrowerAppetite::base_for_tier(5);
    let regional = BorrowerAppetite::base_for_tier(2);
    assert!(elite > 0.0, "fewer loans is a smaller number, not a no");
    assert!(regional > elite);
}

/// A club that genuinely cannot field a balanced side is short, and the
/// emergency reading has to keep working — it is the one thing left in
/// the appetite model that is a fact rather than a policy.
#[test]
fn a_club_in_crisis_reads_as_short() {
    let club = PushFx::giant_with_hidden_attack();
    assert!(
        !LoanBorrowerAppetite::assess(PushFx::main_team(&club)).critical_shortage,
        "precondition: no group is below a fieldable minimum"
    );

    let mut team = PushFx::main_team(&club).clone();
    team.players
        .players
        .retain(|p| p.position().position_group() != PlayerFieldPositionGroup::Forward);
    assert!(
        LoanBorrowerAppetite::assess(&team).critical_shortage,
        "a side with no forwards at all is short"
    );
}
