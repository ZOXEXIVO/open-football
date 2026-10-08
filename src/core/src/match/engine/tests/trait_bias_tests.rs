//! Two players of equal attributes and different traits play differently:
//! the cut-inside winger comes inside where the plain one goes down the
//! line, and the man who dives in goes in more often.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerFieldPositionGroup;
use crate::club::player::traits::PlayerTrait;
use crate::r#match::common_states::TackleDecision;
use crate::r#match::passing::FlankPlay;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, StateProcessor,
};
use nalgebra::Vector3;
use std::cell::Cell;

/// Reads one decision off a live context and stores it.
struct Probe<'a, T: Copy> {
    read: fn(&StateProcessingContext) -> T,
    out: &'a Cell<Option<T>>,
}

impl<T: Copy> StateProcessingHandler for Probe<'_, T> {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        self.out.set(Some((self.read)(ctx)));
        None
    }
}

fn read<T: Copy>(
    field: &mut MatchField,
    context: &MatchContext,
    id: u32,
    traits: Vec<PlayerTrait>,
    probe: fn(&StateProcessingContext) -> T,
) -> T {
    field.get_player_mut(id).unwrap().traits = traits;
    let tick = GameTickContext::new(field, &context.players);
    let out = Cell::new(None);
    let player = field.players.iter_mut().find(|p| p.id == id).unwrap();
    StateProcessor::new(0, player, context, &tick).process_inner(Probe {
        read: probe,
        out: &out,
    });
    out.get().unwrap()
}

/// A home wide man on the ball near the touchline in the opposition half,
/// with the lane inside him half open.
fn winger(field: &mut MatchField) -> u32 {
    let id = field
        .players
        .iter()
        .find(|p| {
            p.team_id == 1
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Midfielder
        })
        .map(|p| p.id)
        .unwrap();
    let x = match field.side_of(1) {
        PlayerSide::Left => 560.0,
        PlayerSide::Right => 280.0,
    };
    let at = Vector3::new(x, 40.0, 0.0);
    field.get_player_mut(id).unwrap().position = at;
    field.ball.position = at;
    field.ball.current_owner = Some(id);
    id
}

#[test]
fn a_cut_inside_winger_comes_inside_where_a_plain_one_goes_down_the_line() {
    let (mut field, context) = kickoff();
    let id = winger(&mut field);
    let half_open = |ctx: &StateProcessingContext| FlankPlay::carry_aim(ctx, 0.45).is_some();

    assert!(
        read(&mut field, &context, id, vec![], half_open),
        "plain winger goes outside"
    );
    assert!(
        !read(
            &mut field,
            &context,
            id,
            vec![PlayerTrait::CutsInsideFromBothWings],
            half_open
        ),
        "the cut-inside winger takes the half-open lane inside"
    );

    let mostly_open = |ctx: &StateProcessingContext| FlankPlay::carry_aim(ctx, 0.60).is_some();
    assert!(
        !read(&mut field, &context, id, vec![], mostly_open),
        "plain winger comes inside"
    );
    assert!(
        read(
            &mut field,
            &context,
            id,
            vec![PlayerTrait::HugsLine],
            mostly_open
        ),
        "the line-hugger still goes down the outside"
    );
}

#[test]
fn a_man_who_dives_in_goes_in_more_readily_than_one_who_stays_on_his_feet() {
    let (mut field, context) = kickoff();
    let id = winger(&mut field);
    let temper = |ctx: &StateProcessingContext| TackleDecision::trait_temper(ctx);

    let plain = read(&mut field, &context, id, vec![], temper);
    let dives = read(
        &mut field,
        &context,
        id,
        vec![PlayerTrait::DivesIntoTackles],
        temper,
    );
    let stays = read(
        &mut field,
        &context,
        id,
        vec![PlayerTrait::StaysOnFeet],
        temper,
    );
    assert_eq!(plain, 1.0);
    assert!(
        dives > plain && plain > stays,
        "dives {dives}, plain {plain}, stays {stays}"
    );
}
