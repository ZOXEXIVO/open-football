//! Moved verbatim out of `helpers.rs` — see that file's `mod buyer_role_match_tests`.

use crate::transfers::pipeline::advice::BuyerNeedPicture;
use crate::{
    PlayerFieldPositionGroup, PlayerPosition, PlayerPositionType, PlayerPositions, PositionCoverage,
};

struct RoleMatchFx;

impl RoleMatchFx {
    /// A wide forward: filed under Midfielder because his record leads
    /// with a wing, a natural centre-forward all the same.
    fn wide_forward() -> PositionCoverage {
        PositionCoverage::of(&PlayerPositions {
            positions: vec![
                PlayerPosition {
                    position: PlayerPositionType::AttackingMidfielderRight,
                    level: 20,
                },
                PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                },
            ],
        })
    }

    /// A club stacked in midfield and short up front — Spartak's shape.
    fn stacked_midfield_thin_attack() -> BuyerNeedPicture {
        let mut picture = BuyerNeedPicture::default();
        picture.best_in_group[PlayerFieldPositionGroup::Midfielder.index()] = 150;
        picture.best_in_group[PlayerFieldPositionGroup::Forward.index()] = 92;
        picture
    }
}

/// The gap this closed: judged by his label a wide forward was measured
/// against the buyer's MIDFIELD — no request, no room, a very high
/// best-in-group — and rejected as no upgrade, while the centre-forward
/// shirt he would have filled went on unaddressed.
#[test]
fn a_versatile_attacker_is_judged_where_the_club_is_short() {
    let picture = RoleMatchFx::stacked_midfield_thin_attack();
    assert_eq!(
        picture.role_for(
            RoleMatchFx::wide_forward(),
            PlayerFieldPositionGroup::Midfielder
        ),
        PlayerFieldPositionGroup::Forward,
        "the weaker of the two lines he can play is where he does most good"
    );
}

/// An open request outranks the raw quality picture: what the club has
/// actually asked for wins.
#[test]
fn an_open_request_wins_over_the_weakest_line() {
    let mut picture = RoleMatchFx::stacked_midfield_thin_attack();
    picture.open_request[PlayerFieldPositionGroup::Midfielder.index()] = true;
    assert_eq!(
        picture.role_for(
            RoleMatchFx::wide_forward(),
            PlayerFieldPositionGroup::Midfielder
        ),
        PlayerFieldPositionGroup::Midfielder
    );
}

/// …and an ageing starter to succeed comes next.
#[test]
fn an_ageing_starter_outranks_a_merely_weaker_line() {
    let mut picture = BuyerNeedPicture::default();
    picture.best_in_group[PlayerFieldPositionGroup::Midfielder.index()] = 150;
    picture.best_in_group[PlayerFieldPositionGroup::Forward.index()] = 149;
    picture.aging_starter[PlayerFieldPositionGroup::Midfielder.index()] = true;
    assert_eq!(
        picture.role_for(
            RoleMatchFx::wide_forward(),
            PlayerFieldPositionGroup::Midfielder
        ),
        PlayerFieldPositionGroup::Midfielder
    );
}

/// A player who covers one group only is judged exactly as he always was.
#[test]
fn a_single_group_player_is_unaffected() {
    let picture = RoleMatchFx::stacked_midfield_thin_attack();
    for position in PlayerPositionType::ALL {
        let group = position.position_group();
        assert_eq!(
            picture.role_for(PositionCoverage::single(position), group),
            group,
            "{position:?} covers nothing else, so nothing changes for him"
        );
    }
}
