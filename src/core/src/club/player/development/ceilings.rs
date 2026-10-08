//! Position-weighted per-skill ceilings derived from PA — the exact
//! table the weekly development tick uses for its growth gate, exposed
//! so the daily training path can honour the same contract instead of
//! clamping only at the absolute 20.0. Ceilings gate growth, never cut:
//! callers must lift the ceiling to the pre-gain value when a skill
//! already sits above it (imports, legacy states).

use super::position_weights::{pos_group_from, position_dev_weights};
use super::skills_array::{
    SK_ACCELERATION, SK_AGILITY, SK_BALANCE, SK_JUMPING, SK_NATURAL_FITNESS, SK_PACE, SKILL_COUNT,
    SkillCategory, SkillKey, skill_category,
};
use crate::PlayerSkills;
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
    /// Maturity is a share of the player's *ability*: a family at 0.72
    /// holds the attributes of a player of 0.72 × PA. On the calibrated CA
    /// scale, whose zero sits near attribute level 6, the same share of
    /// the attribute level is a far smaller share of ability: read that
    /// way, the ceiling capped each age at its typical player rather than
    /// its best.
    ///
    /// The part of each family that only matches build
    /// ([`SkillMaturation::match_share`]) opens with the football he is
    /// getting: training brings every young player along, but only one
    /// who plays grows into the whole of his age's share. The other
    /// channels differ in rate only, and rates converge on the same
    /// ceiling — whoever falls behind trains it back.
    ///
    /// Ceilings gate growth and never cut (see the module docs): a player
    /// already above his age ceiling — an import, an existing save, a late
    /// developer, a regular who lost his place — keeps every point he has
    /// and simply stops gaining until his age, or his football, catches up.
    pub fn for_player(player: &Player, age: f32) -> Self {
        let position = player.position();
        let weights = position_dev_weights(pos_group_from(position));
        let potential = player.player_attributes.potential_ability as f32;
        let unplayed = 1.0 - player.load.match_exposure();
        let mut levels = [None; MaturationGroup::COUNT];
        let mut arr = [1.0f32; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            let group = Self::maturation_group(i);
            let level = *levels[group as usize].get_or_insert_with(|| {
                let held_share = SkillMaturation::ratio(age, group)
                    * (1.0 - SkillMaturation::match_share(group) * unplayed);
                let held = (potential * held_share).round() as u8;
                PlayerSkills::shaped_skill_level(position, held, &weights)
            });
            arr[i] = (level * weights[i]).clamp(1.0, 20.0);
        }
        PositionalSkillCeilings { arr }
    }

    pub fn get(&self, key: SkillKey) -> f32 {
        self.arr[key.idx()]
    }

    pub(super) fn at(&self, idx: usize) -> f32 {
        self.arr[idx]
    }

    /// Bridge one skill onto its maturation family. Separate enums on
    /// purpose — the generator and the tick index skills differently, so
    /// they share the curve, not a layout.
    ///
    /// Keyed on the skill rather than on its category because the
    /// explosive split lives INSIDE `SkillCategory::Physical`: speed and
    /// leap mature years before strength and stamina do.
    pub(crate) fn maturation_group(idx: usize) -> MaturationGroup {
        match skill_category(idx) {
            SkillCategory::Technical => MaturationGroup::Technical,
            SkillCategory::Mental => MaturationGroup::Mental,
            SkillCategory::Goalkeeping => MaturationGroup::Goalkeeping,
            SkillCategory::Physical => match idx {
                // The generator's own earliest-peaking band (18-24).
                // `match_readiness` is match sharpness, not a grown
                // physical quality, so it stays with the slow-maturing
                // group where the tick has always treated it.
                SK_ACCELERATION | SK_PACE | SK_AGILITY | SK_JUMPING | SK_BALANCE
                | SK_NATURAL_FITNESS => MaturationGroup::Explosive,
                _ => MaturationGroup::Physical,
            },
        }
    }
}
