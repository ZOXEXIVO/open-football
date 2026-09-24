//! Moved verbatim out of `loan_market.rs` — see that file's `mod loan_push_gate_tests`.

use super::super::*;
use crate::ReputationLevel;
use crate::academy::ClubAcademy;
use crate::club::player::builder::PlayerBuilder;
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::{
    ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
    PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, StaffCollection, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
};
use chrono::{NaiveDate, NaiveTime};

struct PushFx;

impl PushFx {
    fn player(id: u32, roles: &[(PlayerPositionType, u8)], level: u8) -> Player {
        let attrs = PlayerAttributes {
            current_ability: level,
            ..Default::default()
        };
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Loan".to_string(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            // The depth chart a loan is priced against is the observable
            // one, so the fixture moves the skills it is read from.
            .skills(PlayerSkills::flat_for_ability(level))
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
    let elite = BorrowerAppetite::base_for_tier(ReputationLevel::Elite);
    let regional = BorrowerAppetite::base_for_tier(ReputationLevel::Regional);
    assert!(elite > 0.0, "fewer loans is a smaller number, not a no");
    assert!(regional > elite);
}

/// A club that genuinely cannot field a balanced side wants a body, and
/// wants it more than a club whose shirt is already taken. The emergency
/// is a vacancy now — a term in `BorrowerNeed` — rather than a flag on a
/// separate appetite model.
#[test]
fn a_club_in_crisis_still_takes_what_it_is_offered() {
    let stocked = BorrowerNeed {
        requested: false,
        level_shortfall: 0,
        age_excess: 0,
        vacancy: 0.0,
    };
    let empty_line = BorrowerNeed {
        vacancy: 1.0,
        ..stocked
    };
    assert!(empty_line.score() > stocked.score());
    assert!(
        stocked.score() > 0.0,
        "somebody is always worth a look, shortage or not"
    );
}

/// An open request is a band, not a shirt.
///
/// The scan used to give a flat 1.0 to any candidate whose position group
/// matched an open request, so a club shopping for a first-team striker
/// wanted a raw teenager exactly as badly as the man it had asked for.
/// `BorrowerNeed` tapers with how far short of the request he falls, and
/// with how far over the age band he is.
#[test]
fn an_open_request_admits_only_the_player_it_asked_for() {
    let asked_for = BorrowerNeed {
        requested: true,
        level_shortfall: 0,
        age_excess: 0,
        vacancy: 0.0,
    };
    let a_bit_short = BorrowerNeed {
        level_shortfall: 5,
        ..asked_for
    };
    let nothing_like_it = BorrowerNeed {
        level_shortfall: 20,
        ..asked_for
    };
    let too_old = BorrowerNeed {
        age_excess: 8,
        ..asked_for
    };
    let nobody_asked = BorrowerNeed::none();

    assert!(asked_for.score() > a_bit_short.score());
    assert!(a_bit_short.score() > nothing_like_it.score());
    assert!(
        (nothing_like_it.score() - nobody_asked.score()).abs() < 1e-6,
        "a request he answers none of is not a request for him"
    );
    assert!(too_old.score() < asked_for.score());
}

/// A club that has asked for a man wants him more than one that has not,
/// whatever else is true of either.
#[test]
fn a_club_that_asked_wants_him_more_than_one_that_did_not() {
    let asked = BorrowerNeed {
        requested: true,
        level_shortfall: 0,
        age_excess: 0,
        vacancy: 0.0,
    };
    assert!(asked.score() > BorrowerNeed::none().score());
}
