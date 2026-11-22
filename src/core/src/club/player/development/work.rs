//! Work done on a player's attributes, and the one rule that turns it into
//! ability: progress along his blueprint, read against the headroom his
//! ceilings leave, at his plasticity, as much as he takes in, inside his
//! potential.

use super::blueprint::SkillBlueprint;
use super::ceilings::PositionalSkillCeilings;
use super::ledger::GrowthSource;
use super::skills_array::{SKILL_COUNT, SkillKey};
use crate::club::player::maturation::SkillMaturation;
use crate::club::player::player::Player;
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// What one session gave one player: repetitions of each attribute, and
/// their quality (the coach who took him, the facility, how well he
/// executed the session, the squad's chemistry and his bond with the coach).
#[derive(Debug, Clone, Copy)]
pub struct SessionWork {
    reps: [f32; SKILL_COUNT],
    quality: f32,
}

impl Default for SessionWork {
    fn default() -> Self {
        SessionWork {
            reps: [0.0; SKILL_COUNT],
            quality: 1.0,
        }
    }
}

impl SessionWork {
    pub fn practise(&mut self, key: SkillKey, reps: f32) {
        self.reps[key.idx()] += reps;
    }

    pub fn reps(&self, key: SkillKey) -> f32 {
        self.reps[key.idx()]
    }

    pub fn total_reps(&self) -> f32 {
        self.reps.iter().sum()
    }

    pub fn quality(&self) -> f32 {
        self.quality
    }

    pub fn scale_reps(&mut self, factor: f32) {
        for r in &mut self.reps {
            *r *= factor;
        }
    }

    pub fn scale_quality(&mut self, factor: f32) {
        self.quality *= factor;
    }

    pub(super) fn reps_at(&self, idx: usize) -> f32 {
        self.reps[idx]
    }
}

/// How much of the work that reaches him a player takes in: his character
/// (professionalism, ambition, determination, work rate) and the state of
/// his body (condition, jadedness, the recovery he owes).
pub(super) struct Absorption;

impl Absorption {
    pub(super) fn of(player: &Player) -> f32 {
        Self::character(player) * Self::body(player)
    }

    fn character(player: &Player) -> f32 {
        let weighted = player.attributes.professionalism * 0.40
            + player.attributes.ambition * 0.25
            + player.skills.mental.determination * 0.20
            + player.skills.mental.work_rate * 0.15;
        0.4 + weighted / 20.0 * 1.2
    }

    fn body(player: &Player) -> f32 {
        let condition =
            (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.0, 1.0);
        let jaded = (player.player_attributes.jadedness.max(0) as f32 / 10_000.0).clamp(0.0, 1.0);
        let debt = (player.load.recovery_debt / 800.0).clamp(0.0, 1.0);
        ((0.55 + condition * 0.45) * (1.0 - jaded * 0.35) * (1.0 - debt * 0.2)).clamp(0.4, 1.0)
    }
}

/// Holds a profile's ability at his potential: a change that would score
/// past it keeps its losses and gives back its gains in proportion.
pub(super) struct PotentialBudget;

impl PotentialBudget {
    pub(super) fn hold(
        player: &Player,
        before: &[f32; SKILL_COUNT],
        after: [f32; SKILL_COUNT],
    ) -> [f32; SKILL_COUNT] {
        let position = player.position();
        let potential = player.player_attributes.potential_ability;
        let ca_of = |arr: &[f32; SKILL_COUNT]| -> u8 {
            let mut probe = player.skills;
            probe.write_from_array(arr);
            probe.calculate_ability_for_position(position)
        };
        if ca_of(&after) <= potential {
            return after;
        }
        let blend = |t: f32| -> [f32; SKILL_COUNT] {
            let mut out = *before;
            for i in 0..SKILL_COUNT {
                let d = after[i] - before[i];
                out[i] = before[i] + if d > 0.0 { d * t } else { d };
            }
            out
        };
        if ca_of(&blend(0.0)) >= potential {
            return blend(0.0);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..8 {
            let mid = (lo + hi) * 0.5;
            if ca_of(&blend(mid)) > potential {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        blend(lo)
    }
}

impl Player {
    /// Turn `work` (progress per attribute before his plasticity, his
    /// absorption and his headroom) into attribute change. Each attribute
    /// moves by a share of his own range, from the scale's zero (or half
    /// its target, if lower) to its target at his potential, so the same
    /// work moves a low and a high potential alike. It slows as the
    /// attribute nears its ceiling and stops there; an attribute he does
    /// not have stays absent. Training brings him to the ceilings a player
    /// without football holds; only matches build the share his banked
    /// football opens above them.
    pub(super) fn grow(
        &mut self,
        work: &[f32; SKILL_COUNT],
        date: NaiveDate,
        source: GrowthSource,
    ) {
        let age = DateUtils::age_in_years(self.birth_date, date);
        let maturity_age = self.development.age(age);
        let weights = self.development.weights(self);
        let held = match source {
            GrowthSource::Matches => PositionalSkillCeilings::held_shares(self, age),
            _ => PositionalSkillCeilings::unplayed_shares(self, age),
        };
        let ceilings =
            PositionalSkillCeilings::sized_where(self, age, &held, &weights, |i| work[i] > 0.0);
        let level = SkillBlueprint::level(
            self.position(),
            self.player_attributes.potential_ability as f32,
            &weights,
        );
        let absorb = Absorption::of(self);
        let before = self.skills_to_array();
        let mut after = before;
        for i in 0..SKILL_COUNT {
            let value = before[i];
            if work[i] <= 0.0 || value <= 0.0 || PositionalSkillCeilings::is_condition(i) {
                continue;
            }
            let ceiling = ceilings.at(i);
            if value >= ceiling {
                continue;
            }
            let target = (level * weights[i]).clamp(1.0, 20.0);
            let floor = SkillBlueprint::ZERO.min(target * 0.5);
            let range = (target - floor).max(0.5);
            let headroom = ((ceiling - value) / (ceiling - floor).max(0.5)).clamp(0.0, 1.0);
            let plasticity = SkillMaturation::plasticity(
                maturity_age - SkillMaturation::attribute_shift(i),
                PositionalSkillCeilings::maturation_group(i),
            );
            let progress = work[i] * plasticity * absorb * headroom;
            after[i] = (value + progress * range).min(ceiling);
        }
        self.settle(before, after, source);
    }

    /// Land a change of attributes: inside his potential, written back,
    /// recorded by its source, and his ability re-read.
    pub(super) fn settle(
        &mut self,
        before: [f32; SKILL_COUNT],
        after: [f32; SKILL_COUNT],
        source: GrowthSource,
    ) {
        let after = PotentialBudget::hold(self, &before, after);
        self.skills.write_from_array(&after);
        self.development.ledger.record(source, &before, &after);
        self.player_attributes.current_ability = self
            .skills
            .calculate_ability_for_position(self.position())
            .min(self.player_attributes.potential_ability);
    }
}
