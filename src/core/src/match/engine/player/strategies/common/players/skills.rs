use crate::r#match::StateProcessingContext;
use crate::r#match::engine::player::strategies::common::players::ops::skill_composites as sc;

/// Coarse shooting context — picks the right shooting composite.
#[derive(Debug, Clone, Copy)]
pub enum ShotRange {
    /// Six-yard / inside-the-box close-range chance.
    Close,
    /// Edge-of-box, ~16-22m.
    Medium,
    /// Outside the box, 25m+.
    Long,
}

/// Operations for skill-based calculations.
///
/// All ability getters route through `skill_composites`, which apply
/// fatigue, late-game mental drift, and stamina mitigation via
/// `effective_skill`. Local ad hoc weighted blends were removed: a
/// single composite source-of-truth means every decision/execution
/// hot path that consults SkillOperationsImpl reads the same fatigue
/// curve as the rest of the engine.
pub struct SkillOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> SkillOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        SkillOperationsImpl { ctx }
    }

    /// Normalize skill value to 0.0-1.0 range (divides by 20.0)
    #[inline]
    pub fn normalized(&self, skill_value: f32) -> f32 {
        (skill_value / 20.0).clamp(0.0, 1.0)
    }

    #[inline]
    fn minute(&self) -> u32 {
        sc::minute_from_ms(self.ctx.context.total_match_time)
    }

    /// Calculate adjusted threshold based on skill
    /// Formula: base_value * (min_factor + skill * range_factor)
    pub fn adjusted_threshold(
        &self,
        base_value: f32,
        skill_value: f32,
        min_factor: f32,
        range_factor: f32,
    ) -> f32 {
        let skill = self.normalized(skill_value);
        base_value * (min_factor + skill * range_factor)
    }

    /// Combine multiple skills with weights into a single factor
    /// Example: combined_factor(&[(finishing, 0.5), (composure, 0.3), (technique, 0.2)])
    pub fn combined_factor(&self, skills_with_weights: &[(f32, f32)]) -> f32 {
        skills_with_weights
            .iter()
            .map(|(skill, weight)| self.normalized(*skill) * weight)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    }

    /// Player's dribble-attack composite (0.0-1.0). Folds fatigue.
    pub fn dribbling_ability(&self) -> f32 {
        sc::dribble_attack(self.ctx.player, self.minute())
    }

    /// Player's passing execution composite (0.0-1.0). Folds fatigue.
    pub fn passing_ability(&self) -> f32 {
        sc::passing_execution(self.ctx.player, self.minute())
    }

    /// Context-sensitive shooting ability. `Close` uses
    /// `shooting_close`, `Medium` uses `shooting_medium`, `Long`
    /// uses `long_shot`.
    pub fn shooting_ability(&self, range: ShotRange) -> f32 {
        let m = self.minute();
        match range {
            ShotRange::Close => sc::shooting_close(self.ctx.player, m),
            ShotRange::Medium => sc::shooting_medium(self.ctx.player, m),
            ShotRange::Long => sc::long_shot(self.ctx.player, m),
        }
    }

    /// Player's defensive ability — average of `defensive_duel` and
    /// `defensive_positioning` so the composite captures both
    /// challenging the man and reading the play. Folds fatigue.
    pub fn defensive_ability(&self) -> f32 {
        let m = self.minute();
        0.5 * (sc::defensive_duel(self.ctx.player, m)
            + sc::defensive_positioning(self.ctx.player, m))
    }

    /// Player's mobility composite (0.0-1.0) — pace/accel/agility
    /// blend, fatigue-aware.
    pub fn physical_ability(&self) -> f32 {
        sc::mobility(self.ctx.player, self.minute())
    }

    /// Player's decision-quality composite (0.0-1.0) — decisions,
    /// composure, concentration blend. Fatigue-aware.
    pub fn mental_ability(&self) -> f32 {
        sc::decision_quality(self.ctx.player, self.minute())
    }

    /// Get stamina factor based on condition (0.0-1.0)
    pub fn stamina_factor(&self) -> f32 {
        self.ctx.player.player_attributes.condition_percentage() as f32 / 100.0
    }

    /// Check if player is tired (stamina below threshold)
    pub fn is_tired(&self, threshold: u32) -> bool {
        self.ctx.player.player_attributes.condition_percentage() < threshold
    }

    /// Calculate fatigue factor for movement (0.0-1.0, accounts for time in state)
    pub fn fatigue_factor(&self, state_duration: u64) -> f32 {
        let stamina = self.stamina_factor();
        let time_factor = (1.0 - (state_duration as f32 / 500.0)).max(0.5);

        stamina * time_factor
    }
}
