//! A side down to ten re-forms: the role that matters is filled from the
//! line the coach can spare, the short line spreads itself, the shape
//! survives the change of ends, and the coach plays the numbers.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerPositionType;
use crate::r#match::engine::flow::arena::formation_variant::{
    FormationLine, FormationVariant, VacatedSlot,
};
use crate::r#match::{
    CoachCard, MatchCoach, MatchField, Outlook, PlayerSide, ReadCounters, Situation, SquadFit,
};

fn holder(field: &MatchField, team_id: u32, position: PlayerPositionType) -> Option<u32> {
    field
        .players
        .iter()
        .find(|p| {
            p.team_id == team_id && !p.off_pitch && p.tactical_position.current_position == position
        })
        .map(|p| p.id)
}

fn line_count(field: &MatchField, team_id: u32, line: FormationLine) -> usize {
    field
        .players
        .iter()
        .filter(|p| {
            p.team_id == team_id
                && !p.off_pitch
                && FormationLine::of(p.tactical_position.current_position) == Some(line)
        })
        .count()
}

fn start_y(field: &MatchField, id: u32) -> f32 {
    field.get_player(id).unwrap().start_position.y
}

#[test]
fn a_side_protecting_fills_the_back_four_and_gives_up_a_forward() {
    let (mut field, _) = kickoff();
    let centre_back = holder(&field, 1, PlayerPositionType::DefenderCenterLeft).unwrap();
    let midfielder = holder(&field, 1, PlayerPositionType::MidfielderCenterLeft).unwrap();
    let forward = holder(&field, 1, PlayerPositionType::ForwardLeft).unwrap();
    let partner = holder(&field, 1, PlayerPositionType::ForwardRight).unwrap();

    field.take_off(centre_back, FormationLine::Attack);

    assert_eq!(
        holder(&field, 1, PlayerPositionType::DefenderCenterLeft),
        Some(midfielder),
        "the centre-back's role is left empty"
    );
    assert_eq!(
        holder(&field, 1, PlayerPositionType::MidfielderCenterLeft),
        Some(forward)
    );
    assert_eq!(line_count(&field, 1, FormationLine::Defence), 4);
    assert_eq!(line_count(&field, 1, FormationLine::Midfield), 4);
    assert_eq!(line_count(&field, 1, FormationLine::Attack), 1);
    assert_eq!(
        field.vacated,
        vec![VacatedSlot {
            team_id: 1,
            position: PlayerPositionType::ForwardLeft
        }]
    );

    // The lone forward leads the line from the middle of it.
    let side = field.side_of(1);
    let left = FormationVariant::slot(PlayerPositionType::ForwardLeft, side).unwrap();
    let right = FormationVariant::slot(PlayerPositionType::ForwardRight, side).unwrap();
    assert_eq!(start_y(&field, partner), (left.y + right.y) * 0.5);
}

#[test]
fn a_side_chasing_gives_up_a_midfielder_and_keeps_two_up() {
    let (mut field, _) = kickoff();
    let centre_back = holder(&field, 1, PlayerPositionType::DefenderCenterLeft).unwrap();

    field.take_off(centre_back, FormationLine::Midfield);

    assert!(holder(&field, 1, PlayerPositionType::DefenderCenterLeft).is_some());
    assert_eq!(line_count(&field, 1, FormationLine::Defence), 4);
    assert_eq!(line_count(&field, 1, FormationLine::Midfield), 3);
    assert_eq!(line_count(&field, 1, FormationLine::Attack), 2);

    // Three across the width four had: wide, central, wide.
    let side = field.side_of(1);
    let wide_left = FormationVariant::slot(PlayerPositionType::MidfielderLeft, side).unwrap();
    let wide_right = FormationVariant::slot(PlayerPositionType::MidfielderRight, side).unwrap();
    let centre = holder(&field, 1, PlayerPositionType::MidfielderCenterRight).unwrap();
    assert_eq!(start_y(&field, centre), (wide_left.y + wide_right.y) * 0.5);
}

#[test]
fn the_ten_man_shape_survives_the_change_of_ends() {
    let (mut field, _) = kickoff();
    let centre_back = holder(&field, 1, PlayerPositionType::DefenderCenterLeft).unwrap();
    let partner = holder(&field, 1, PlayerPositionType::ForwardRight).unwrap();
    field.take_off(centre_back, FormationLine::Attack);
    let stand_in = holder(&field, 1, PlayerPositionType::DefenderCenterLeft).unwrap();

    let before = field.side_of(1);
    field.swap_squads();
    let after = field.side_of(1);
    assert_ne!(before, after);

    let slot = FormationVariant::slot(PlayerPositionType::DefenderCenterLeft, after).unwrap();
    assert_eq!(field.get_player(stand_in).unwrap().start_position, slot);
    let left = FormationVariant::slot(PlayerPositionType::ForwardLeft, after).unwrap();
    let right = FormationVariant::slot(PlayerPositionType::ForwardRight, after).unwrap();
    assert_eq!(start_y(&field, partner), (left.y + right.y) * 0.5);
    assert_eq!(after == PlayerSide::Right, before == PlayerSide::Left);
}

#[test]
fn a_coach_a_man_down_protects_from_the_first_minute() {
    let coach_at = |difference: i32, minute: f32| {
        let mut coach = MatchCoach::new();
        coach.on_kickoff(
            CoachCard::neutral(),
            Outlook::default(),
            &SquadFit::default(),
            None,
        );
        coach.observe(
            Situation {
                difference,
                minutes_left: 90.0 - minute,
                ..Situation::default()
            },
            minute,
            ReadCounters::default(),
        );
        coach
    };

    // Half an hour in, leading: from the sending-off on, ten protect it
    // harder than eleven do.
    let mut eleven = coach_at(1, 30.0);
    let mut ten = coach_at(1, 30.0);
    ten.on_sending_off(true, 30.0);
    let a_minute_on = |coach: &mut MatchCoach, own_men: u8| {
        coach.observe(
            Situation {
                difference: 1,
                minutes_left: 59.0,
                own_men,
                ..Situation::default()
            },
            31.0,
            ReadCounters::default(),
        );
        coach.stance()
    };
    let (with_eleven, with_ten) = (a_minute_on(&mut eleven, 11), a_minute_on(&mut ten, 10));
    assert!(with_ten < with_eleven, "{with_ten} {with_eleven}");

    // Chasing late with ten, he still commits, and gives up a midfielder
    // rather than a forward.
    let mut chasing = coach_at(-1, 85.0);
    chasing.on_sending_off(true, 85.0);
    chasing.observe(
        Situation {
            difference: -1,
            minutes_left: 4.0,
            own_men: 10,
            ..Situation::default()
        },
        86.0,
        ReadCounters::default(),
    );
    assert!(chasing.stance() > 0.0);
    assert_eq!(chasing.spare_line(), FormationLine::Midfield);
    assert_eq!(coach_at(1, 85.0).spare_line(), FormationLine::Attack);
}
