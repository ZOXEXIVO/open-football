//! Per-category coaching effectiveness, normalized to a multiplier
//! centered on ~1.0. A bad coach (average attribute 5/20) produces ~0.75;
//! an elite coach (18/20) produces ~1.35. For players under 23, the club's
//! best `working_with_youngsters` attribute adds a further +0-15% bonus.

use super::skills_array::SkillCategory;

#[derive(Debug, Clone, Copy)]
pub struct CoachingEffect {
    pub technical: f32,
    pub mental: f32,
    pub physical: f32,
    pub goalkeeping: f32,
    /// Bonus multiplier applied on top of the category multiplier for
    /// players under 23.
    pub youth_bonus: f32,
}

impl CoachingEffect {
    pub fn neutral() -> Self {
        Self {
            technical: 1.0,
            mental: 1.0,
            physical: 1.0,
            goalkeeping: 1.0,
            youth_bonus: 1.0,
        }
    }

    /// Build from the best coach attribute found at the club (0-20 scale)
    /// and the youth coaching quality (0.0-1.0 normalized).
    pub fn from_scores(
        technical: u8,
        mental: u8,
        fitness: u8,
        goalkeeping: u8,
        youth_quality_0_1: f32,
    ) -> Self {
        let m = |attr: u8| -> f32 {
            // 0 -> 0.60, 10 -> 1.0, 20 -> 1.40 (linear)
            (0.6 + (attr as f32 / 20.0) * 0.8).clamp(0.55, 1.45)
        };
        Self {
            technical: m(technical),
            mental: m(mental),
            physical: m(fitness),
            goalkeeping: m(goalkeeping),
            youth_bonus: (1.0 + youth_quality_0_1 * 0.15).clamp(1.0, 1.18),
        }
    }

    pub(super) fn for_category(&self, cat: SkillCategory) -> f32 {
        match cat {
            SkillCategory::Technical => self.technical,
            SkillCategory::Mental => self.mental,
            SkillCategory::Physical => self.physical,
            SkillCategory::Goalkeeping => self.goalkeeping,
        }
    }
}
