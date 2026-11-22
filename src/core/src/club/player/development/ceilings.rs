//! Per-attribute ceilings: the player's own blueprint sized to the share of
//! his potential his age and football let him hold. Ceilings gate growth,
//! never cut:
//! callers must lift the ceiling to the pre-gain value when a skill
//! already sits above it (imports, legacy states).

use super::blueprint::SkillBlueprint;
use super::skills_array::{
    SK_ACCELERATION, SK_AGILITY, SK_BALANCE, SK_JUMPING, SK_MATCH_READINESS, SK_NATURAL_FITNESS,
    SK_PACE, SKILL_COUNT, SkillCategory, SkillKey,
};
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
        Self::sized(player, age, &Self::held_shares(player, age))
    }

    /// The ceilings training and a growing body can reach on their own: those
    /// of a player who has banked no football. Only matches build the rest.
    pub fn without_football(player: &Player, age: f32) -> Self {
        Self::sized(player, age, &Self::unplayed_shares(player, age))
    }

    fn sized(player: &Player, age: f32, held: &[f32; MaturationGroup::COUNT]) -> Self {
        Self::sized_where(
            player,
            age,
            held,
            &player.development.weights(player),
            |_| true,
        )
    }

    /// Ceilings of the attributes `wanted` picks, from `weights`; the rest
    /// read zero. A family's level is solved only if one of its attributes
    /// is wanted. Past his peak each attribute holds its own share of what
    /// ageing leaves, as he keeps his body.
    pub(super) fn sized_where(
        player: &Player,
        age: f32,
        held: &[f32; MaturationGroup::COUNT],
        weights: &[f32; SKILL_COUNT],
        wanted: impl Fn(usize) -> bool,
    ) -> Self {
        let position = player.position();
        let potential = player.player_attributes.potential_ability as f32;
        let maturity_age = player.development.age(age);
        let keep = player.ageing_keep();
        let mut levels = [None; MaturationGroup::COUNT];
        let mut arr = [0.0f32; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            if weights[i] <= 0.0 || !wanted(i) {
                continue;
            }
            let group = Self::maturation_group(i) as usize;
            let level = *levels[group].get_or_insert_with(|| {
                SkillBlueprint::level(position, potential * held[group], weights)
            });
            let aged = SkillMaturation::aged_attribute(maturity_age, i, keep);
            arr[i] = (level * weights[i] * aged).clamp(1.0, 20.0);
        }
        PositionalSkillCeilings { arr }
    }

    /// The share of his eventual ability each maturation family holds at
    /// `age`: its maturity at his own pace of maturing, less the part only
    /// matches build in proportion to the football he has not had.
    pub(crate) fn held_shares(player: &Player, age: f32) -> [f32; MaturationGroup::COUNT] {
        let age = player.development.age(age);
        let football = &player.development.football;
        MaturationGroup::ALL.map(|group| {
            SkillMaturation::ratio(age, group)
                * (1.0 - SkillMaturation::match_share(group) * (1.0 - football.of(group)))
        })
    }

    /// The held share of a player with no football banked.
    pub(crate) fn unplayed_shares(player: &Player, age: f32) -> [f32; MaturationGroup::COUNT] {
        let age = player.development.age(age);
        MaturationGroup::ALL.map(|group| {
            SkillMaturation::ratio(age, group) * (1.0 - SkillMaturation::match_share(group))
        })
    }

    /// Match readiness sits in the attribute array and is a condition: no
    /// development rule reads it.
    pub(crate) fn is_condition(idx: usize) -> bool {
        idx == SK_MATCH_READINESS
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
        match SkillCategory::of(idx) {
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
