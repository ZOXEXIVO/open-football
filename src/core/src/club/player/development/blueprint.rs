//! Who a player is growing into: his own profile, projected to maturity.

use super::ceilings::PositionalSkillCeilings;
use super::skills_array::SKILL_COUNT;
use crate::club::player::maturation::MaturationGroup;
use crate::{PlayerPositionType, PlayerSkills, SkillType, TrainingFocus};

/// One weight per attribute: the shape of the player he becomes when he has
/// grown into everything. Sized to his potential for his current position
/// on demand, so reaching every target scores exactly his potential.
/// Stored as weight × 100; zero is an attribute he does not have, and an
/// all-zero blueprint is one not drawn yet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkillBlueprint {
    weights: [u8; SKILL_COUNT],
}

impl Default for SkillBlueprint {
    fn default() -> Self {
        SkillBlueprint {
            weights: [0; SKILL_COUNT],
        }
    }
}

impl SkillBlueprint {
    const SCALE: f32 = 100.0;
    /// A very young profile's noise is amplified by the projection; its
    /// weights are held inside this band.
    const LIGHTEST: f32 = 0.05;
    const HEAVIEST: f32 = 2.5;
    /// The attribute level at which the CA scale reads zero.
    pub(crate) const ZERO: f32 = 110.0 / 19.0;
    /// How far an individual plan tilts the blueprint toward its attribute:
    /// a quarter more weight, never more than this much.
    const TILT: f32 = 0.25;
    const MAX_TILT: f32 = 0.3;

    /// Project `skills` to maturity: each attribute's height above the
    /// scale's zero divided by the share of his eventual ability his family
    /// holds now (`held`, by maturation family), then normalised to a mean
    /// weight of one over the attributes he has. An attribute at or below
    /// the zero has no height to grow and is kept as it is.
    pub fn project(skills: &[f32; SKILL_COUNT], held: &[f32; MaturationGroup::COUNT]) -> Self {
        let mut projected = [0.0f32; SKILL_COUNT];
        let (mut sum, mut count) = (0.0f32, 0u32);
        for (i, value) in skills.iter().enumerate() {
            if *value <= 0.0 || PositionalSkillCeilings::is_condition(i) {
                continue;
            }
            let share = held[PositionalSkillCeilings::maturation_group(i) as usize].max(0.05);
            projected[i] = if *value > Self::ZERO {
                Self::ZERO + (value - Self::ZERO) / share
            } else {
                *value
            };
            sum += projected[i];
            count += 1;
        }
        let mean = sum / count.max(1) as f32;
        let mut weights = [0u8; SKILL_COUNT];
        for i in 0..SKILL_COUNT {
            if projected[i] > 0.0 {
                let w = (projected[i] / mean).clamp(Self::LIGHTEST, Self::HEAVIEST);
                weights[i] = (w * Self::SCALE).round() as u8;
            }
        }
        SkillBlueprint { weights }
    }

    pub fn is_drawn(&self) -> bool {
        self.weights.iter().any(|w| *w > 0)
    }

    pub fn weights(&self) -> [f32; SKILL_COUNT] {
        self.weights.map(|w| w as f32 / Self::SCALE)
    }

    /// Whether a projection held a weight at the edge of its band.
    pub fn clamped(&self) -> bool {
        let (lo, hi) = (
            (Self::LIGHTEST * Self::SCALE).round() as u8,
            (Self::HEAVIEST * Self::SCALE).round() as u8,
        );
        self.weights.iter().any(|w| *w == lo || *w == hi)
    }

    /// `weights` tilted toward the attributes an individual plan works on.
    pub fn tilted(mut weights: [f32; SKILL_COUNT], plan: &[TrainingFocus]) -> [f32; SKILL_COUNT] {
        for focus in plan {
            for key in Self::planned(focus) {
                let w = &mut weights[key.idx()];
                if *w > 0.0 {
                    *w += (*w * Self::TILT).min(Self::MAX_TILT);
                }
            }
        }
        weights
    }

    fn planned(focus: &TrainingFocus) -> &'static [super::SkillKey] {
        use super::SkillKey::*;
        match focus {
            TrainingFocus::SpecificSkill(skill) => match skill {
                SkillType::FreeKicks => &[FreeKicks],
                SkillType::Penalties => &[PenaltyTaking],
                SkillType::LongShots => &[LongShots],
                SkillType::Heading => &[Heading],
                SkillType::Tackling => &[Tackling],
                SkillType::Crossing => &[Crossing],
                SkillType::Dribbling => &[Dribbling],
            },
            TrainingFocus::FitnessBuilding => &[Stamina, NaturalFitness],
            TrainingFocus::MentalDevelopment => &[Composure, Decisions],
            TrainingFocus::WeakFootImprovement
            | TrainingFocus::PositionRetraining(_)
            | TrainingFocus::InjuryRecovery => &[],
        }
    }

    /// The level `L` at which the profile `L × weights`, each attribute held
    /// to the 1..20 scale, scores `ability` for `position`.
    pub fn level(position: PlayerPositionType, ability: f32, weights: &[f32; SKILL_COUNT]) -> f32 {
        let prices = PlayerSkills::ability_weights(position);
        let total: f32 = prices.iter().sum();
        let wanted = (ability + 110.0) / 19.0 * total;
        // The score is piecewise linear in the level: solve on the attributes
        // the scale's ends do not hold, then again until none crosses one.
        let mut level = 0.0f32;
        let mut free = 0.0f32;
        for i in 0..SKILL_COUNT {
            free += prices[i] * weights[i];
        }
        if free > 0.0 {
            level = wanted / free;
        }
        for _ in 0..8 {
            let (mut held, mut slope) = (0.0f32, 0.0f32);
            for i in 0..SKILL_COUNT {
                let w = weights[i];
                if w <= 0.0 {
                    continue;
                }
                let v = level * w;
                if v >= 20.0 {
                    held += 20.0 * prices[i];
                } else if v <= 1.0 {
                    held += prices[i];
                } else {
                    slope += prices[i] * w;
                }
            }
            if slope <= 0.0 {
                break;
            }
            let next = ((wanted - held) / slope).max(0.0);
            if (next - level).abs() < 1e-4 {
                level = next;
                break;
            }
            level = next;
        }
        level
    }

    /// The unrounded CA a profile scores for `position`.
    pub fn score(position: PlayerPositionType, values: &[f32; SKILL_COUNT]) -> f32 {
        let prices = PlayerSkills::ability_weights(position);
        let total: f32 = prices.iter().sum();
        let sum: f32 = values.iter().zip(prices).map(|(v, p)| v * p).sum();
        sum / total * 19.0 - 110.0
    }

    /// What each attribute reaches when he has grown into all of it.
    pub fn targets(
        weights: &[f32; SKILL_COUNT],
        position: PlayerPositionType,
        potential: u8,
    ) -> [f32; SKILL_COUNT] {
        let level = Self::level(position, potential as f32, weights);
        weights.map(|w| {
            if w > 0.0 {
                (level * w).clamp(1.0, 20.0)
            } else {
                0.0
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::development::skills_array::skills_to_array;
    use crate::utils::DateUtils;
    use crate::{PeopleNameGeneratorData, PlayerGenerator};
    use chrono::NaiveDate;

    fn names() -> PeopleNameGeneratorData {
        PeopleNameGeneratorData {
            first_names: vec!["Test".into()],
            last_names: vec!["Blueprint".into()],
            nicknames: vec![],
        }
    }

    fn intakes() -> Vec<crate::Player> {
        let today = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let positions = [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::Striker,
        ];
        (0..90)
            .map(|i| {
                PlayerGenerator::generate(1, today, positions[i % 9], (i % 20) as u8, &names())
            })
            .collect()
    }

    /// A profile sitting on every target of his blueprint scores his
    /// potential, within a point.
    #[test]
    fn the_blueprint_is_worth_his_potential() {
        for player in intakes() {
            let position = player.position();
            let pa = player.player_attributes.potential_ability;
            let targets =
                SkillBlueprint::targets(&player.development.blueprint.weights(), position, pa);
            let score = SkillBlueprint::score(position, &targets);
            assert!(
                (score - pa as f32).abs() <= 1.0,
                "{position:?} PA {pa}: his blueprint's targets score {score:.1}"
            );
        }
    }

    /// Within each family the targets stand in the order of his attributes.
    #[test]
    fn targets_keep_his_order() {
        for player in intakes() {
            let today = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
            let age = DateUtils::age_in_years(player.birth_date, today);
            let held = PositionalSkillCeilings::held_shares(&player, age);
            let blueprint = SkillBlueprint::project(&skills_to_array(&player), &held);
            let targets = SkillBlueprint::targets(
                &blueprint.weights(),
                player.position(),
                player.player_attributes.potential_ability,
            );
            let values = skills_to_array(&player);
            for a in 0..SKILL_COUNT {
                for b in 0..SKILL_COUNT {
                    let same_family = PositionalSkillCeilings::maturation_group(a)
                        == PositionalSkillCeilings::maturation_group(b);
                    if !same_family
                        || values[a] <= 0.0
                        || values[b] <= 0.0
                        || PositionalSkillCeilings::is_condition(a)
                        || PositionalSkillCeilings::is_condition(b)
                        || values[a] <= values[b] + 0.05
                    {
                        continue;
                    }
                    assert!(
                        targets[a] >= targets[b] - 0.15,
                        "attribute {a} ({:.2}) is above {b} ({:.2}), its target {:.2} is not ({:.2})",
                        values[a],
                        values[b],
                        targets[a],
                        targets[b]
                    );
                }
            }
        }
    }

    #[test]
    fn a_plan_tilts_the_blueprint_toward_its_attribute() {
        let mut weights = [1.0f32; SKILL_COUNT];
        weights[super::super::SkillKey::GkHandling.idx()] = 0.0;
        let tilted = SkillBlueprint::tilted(
            weights,
            &[TrainingFocus::SpecificSkill(SkillType::Crossing)],
        );
        assert!(tilted[super::super::SkillKey::Crossing.idx()] > 1.0);
        assert!(tilted[super::super::SkillKey::Crossing.idx()] <= 1.0 + SkillBlueprint::MAX_TILT);
        assert_eq!(tilted[super::super::SkillKey::Passing.idx()], 1.0);
    }
}
