//! Monthly squad-status assignment.
//!
//! Each player gets a `PlayerSquadStatus` (KeyPlayer / FirstTeamRegular /
//! RotationPlayer / Backup / NotNeeded / etc.) derived from their CA rank
//! *within their own position group*. Ranking against the whole squad puts
//! a backup goalkeeper at the bottom of a CA-sorted list dominated by
//! outfield stars — you'd get `NotNeeded` for every 3rd/4th keeper at an
//! elite club, and every downstream code path keyed on squad status would
//! treat them as surplus.

use crate::club::team::Team;
use crate::utils::DateUtils;
use crate::{PlayerFieldPositionGroup, PlayerSquadStatus};
use chrono::NaiveDate;
use std::collections::HashMap;

pub struct SquadStatusUpdater;

impl SquadStatusUpdater {
    /// Recompute every player's `contract.squad_status` against the CA
    /// distribution of their position group.
    pub fn apply(team: &mut Team, date: NaiveDate) {
        let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        for p in team.players.iter() {
            let g = p.position().position_group();
            by_group
                .entry(g)
                .or_default()
                .push(p.player_attributes.current_ability);
        }
        for cas in by_group.values_mut() {
            cas.sort_unstable_by(|a, b| b.cmp(a));
        }

        for player in team.players.iter_mut() {
            let group = player.position().position_group();
            let ca = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, date);
            if let Some(ref mut contract) = player.contract {
                let group_cas = by_group.get(&group).map(|v| v.as_slice()).unwrap_or(&[]);
                contract.squad_status = PlayerSquadStatus::calculate(ca, age, group_cas);
            }
        }
    }
}
