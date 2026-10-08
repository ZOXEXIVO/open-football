//! A side down to ten re-forms: the role that matters is filled from the
//! line the coach can spare, the short line spreads itself, the shape
//! survives the change of ends, and the coach plays the numbers.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerPositionType;
use crate::r#match::engine::flow::arena::formation_variant::{
    FormationLine, FormationVariant, VacatedSlot,
};
use crate::r#match::engine::teamplay::coach::instruction::CoachInstruction;
use crate::r#match::{MatchCoach, MatchField, PlayerSide, RollingTeamMetrics};

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
    let evaluate = |score_diff: i8, deficit: i8, progress: f32| {
        let mut coach = MatchCoach::new();
        coach.evaluate_with_metrics(
            score_diff,
            deficit,
            progress,
            0.9,
            1_000,
            RollingTeamMetrics::default(),
        );
        coach.instruction
    };

    // Half an hour in, leading: eleven men play on, ten protect it.
    assert_eq!(evaluate(1, 0, 0.33), CoachInstruction::Normal);
    assert_eq!(evaluate(1, 1, 0.33), CoachInstruction::SlowDown);
    // Two down, they sit in.
    assert_eq!(evaluate(1, 2, 0.33), CoachInstruction::ParkTheBus);
    // Chasing late with ten, they push, but cannot throw everyone forward.
    assert_eq!(evaluate(-1, 0, 0.95), CoachInstruction::AllOutAttack);
    assert_eq!(evaluate(-1, 1, 0.95), CoachInstruction::PushForward);

    let mut chasing = MatchCoach::new();
    chasing.instruction = CoachInstruction::AllOutAttack;
    assert_eq!(chasing.spare_line(), FormationLine::Midfield);
    assert_eq!(MatchCoach::new().spare_line(), FormationLine::Attack);
}
