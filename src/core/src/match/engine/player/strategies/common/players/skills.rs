use crate::r#match::StateProcessingContext;

/// Operations for skill-based calculations
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

    /// Get player's dribbling ability (0.0-1.0)
    pub fn dribbling_ability(&self) -> f32 {
        let dribbling = self.ctx.player.skills.technical.dribbling;
        let agility = self.ctx.player.skills.physical.agility;
        let technique = self.ctx.player.skills.technical.technique;

        self.combined_factor(&[(dribbling, 0.5), (agility, 0.3), (technique, 0.2)])
    }

    /// Get player's passing ability (0.0-1.0)
    pub fn passing_ability(&self) -> f32 {
        let passing = self.ctx.player.skills.technical.passing;
        let vision = self.ctx.player.skills.mental.vision;
        let technique = self.ctx.player.skills.technical.technique;

        self.combined_factor(&[(passing, 0.5), (vision, 0.3), (technique, 0.2)])
    }

    /// Get player's shooting ability (0.0-1.0)
    pub fn shooting_ability(&self) -> f32 {
        let finishing = self.ctx.player.skills.technical.finishing;
        let composure = self.ctx.player.skills.mental.composure;
        let technique = self.ctx.player.skills.technical.technique;

        self.combined_factor(&[(finishing, 0.5), (composure, 0.3), (technique, 0.2)])
    }

    /// Get player's defensive ability (0.0-1.0)
    pub fn defensive_ability(&self) -> f32 {
        let tackling = self.ctx.player.skills.technical.tackling;
        let marking = self.ctx.player.skills.mental.positioning;
        let aggression = self.ctx.player.skills.mental.aggression;

        self.combined_factor(&[(tackling, 0.5), (marking, 0.3), (aggression, 0.2)])
    }

    /// Get player's physical ability (0.0-1.0)
    pub fn physical_ability(&self) -> f32 {
        let pace = self.ctx.player.skills.physical.pace;
        let stamina = self.ctx.player.skills.physical.stamina;
        let strength = self.ctx.player.skills.physical.strength;

        self.combined_factor(&[(pace, 0.4), (stamina, 0.3), (strength, 0.3)])
    }

    /// Get player's mental ability (0.0-1.0)
    pub fn mental_ability(&self) -> f32 {
        let decisions = self.ctx.player.skills.mental.decisions;
        let positioning = self.ctx.player.skills.mental.positioning;
        let vision = self.ctx.player.skills.mental.vision;

        self.combined_factor(&[(decisions, 0.4), (positioning, 0.3), (vision, 0.3)])
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
