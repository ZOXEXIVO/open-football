//! The position store and the proximity grid find players by id through
//! open-addressed tables, and the hash reads only an id's low bits: ids
//! that agree modulo the table size share a home slot and queue behind
//! each other.

use super::goal_celebration_tests::squad;
use crate::r#match::{GameTickContext, MatchField, MatchPlayerCollection};

/// Twenty-two ids on one home slot — a probe run as long as the roster.
/// Lookups that gave up after eight probes lost everybody past the
/// eighth, and a real 36-man store (bench included) had one such man in
/// roughly one match in six.
#[test]
fn every_player_is_found_when_ids_share_a_slot() {
    let mut home = squad(1, 0);
    let mut away = squad(2, 0);
    for (k, player) in home
        .main_squad
        .iter_mut()
        .chain(away.main_squad.iter_mut())
        .enumerate()
    {
        player.id = 64 * (k as u32 + 1);
    }
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let tick = GameTickContext::new(&field, &players);

    for player in &field.players {
        assert_eq!(
            tick.positions.players.position(player.id),
            player.position,
            "position store lost player {}",
            player.id
        );
        assert_eq!(
            tick.grid.position_of(player.id),
            player.position,
            "grid lost player {}",
            player.id
        );
    }
}
