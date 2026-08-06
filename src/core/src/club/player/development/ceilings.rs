//! Position-weighted per-skill ceilings derived from PA — the exact
//! table the weekly development tick uses for its growth gate, exposed
//! so the daily training path can honour the same contract instead of
//! clamping only at the absolute 20.0. Ceilings gate growth, never cut:
//! callers must lift the ceiling to the pre-gain value when a skill
//! already sits above it (imports, legacy states).

use super::position_weights::{pos_group_from, position_dev_weights};
use super::skills_array::{SKILL_COUNT, SkillCategory, SkillKey, skill_category};
use crate::club::player::maturation::{MaturationGroup, SkillMaturation};
use crate::club::player::player::Player;

pub struct PositionalSkillCeilings {
    arr: [f32; SKILL_COUNT],
}

impl PositionalSkillCeilings {
    /// Ceiling for each skill: what the player's potential allows in this
    /// position, scaled by how much of that kind of skill a player his age
    /// has grown into yet.
    ///
    /// The age term is the fix for a model that used to contradict
    /// itself. Potential says where a player finishes; it never said
    /// *when*, so the tick let a sixteen-year-old grow his decisions and
    /// composure to a finished professional's level while the generator
    /// would have built the same boy at 0.55 of it. Both now read
    /// [`SkillMaturation`]. A mind arrives late, and until it does, the
    /// ceiling holds it back — which is what makes a teenager play like a
    /// teenager instead of like the player he is going to become.
    ///
    /// Ceilings gate growth and never cut (see the module docs): a player
    /// already above his age ceiling — an import, an existing save, a late
    /// developer — keeps every point he has and simply stops gaining
    /// until his age catches up.
    pub fn for_player(player: &Player, age: u32) -> Self {
        let pa = player.player_attributes.potential_ability as f32;
        let base_ceiling = (pa / 200.0 * 20.0).clamp(1.0, 20.0);
        let weights = position_dev_weights(pos_group_from(player.position()));
        let mut arr = [1.0f32; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            let maturity = SkillMaturation::ratio(age, maturation_group(skill_category(i)));
            arr[i] = (base_ceiling * weights[i] * maturity).clamp(1.0, 20.0);
        }
        PositionalSkillCeilings { arr }
    }

    pub fn get(&self, key: SkillKey) -> f32 {
        self.arr[key.idx()]
    }
}

/// Bridge the development tick's skill categories onto the shared
/// maturation families. Separate enums on purpose — the generator and the
/// tick index skills differently, so they share the curve, not a layout.
pub(super) fn maturation_group(cat: SkillCategory) -> MaturationGroup {
    match cat {
        SkillCategory::Technical => MaturationGroup::Technical,
        SkillCategory::Mental => MaturationGroup::Mental,
        SkillCategory::Physical => MaturationGroup::Physical,
        SkillCategory::Goalkeeping => MaturationGroup::Goalkeeping,
    }
}
