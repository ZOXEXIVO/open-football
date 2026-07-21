//! Position-weighted per-skill ceilings derived from PA — the exact
//! table the weekly development tick uses for its growth gate, exposed
//! so the daily training path can honour the same contract instead of
//! clamping only at the absolute 20.0. Ceilings gate growth, never cut:
//! callers must lift the ceiling to the pre-gain value when a skill
//! already sits above it (imports, legacy states).

use super::position_weights::{pos_group_from, position_dev_weights};
use super::skills_array::{SKILL_COUNT, SkillKey};
use crate::club::player::player::Player;

pub struct PositionalSkillCeilings {
    arr: [f32; SKILL_COUNT],
}

impl PositionalSkillCeilings {
    pub fn for_player(player: &Player) -> Self {
        let pa = player.player_attributes.potential_ability as f32;
        let base_ceiling = (pa / 200.0 * 20.0).clamp(1.0, 20.0);
        let weights = position_dev_weights(pos_group_from(player.position()));
        let mut arr = [1.0f32; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            arr[i] = (base_ceiling * weights[i]).clamp(1.0, 20.0);
        }
        PositionalSkillCeilings { arr }
    }

    pub fn get(&self, key: SkillKey) -> f32 {
        self.arr[key.idx()]
    }
}
