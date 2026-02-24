use crate::club::staff::staff::Staff;

impl Staff {
    /// Compact single-line LLM representation for staff.
    /// Call `Staff::as_llm_legend()` once per prompt for the key descriptions.
    pub fn as_llm(&self) -> String {
        let position = self.contract.as_ref()
            .map(|c| format!("{:?}", c.position))
            .unwrap_or_else(|| "Free".to_string());

        let a = &self.staff_attributes;
        let c = &a.coaching;
        let k = &a.knowledge;
        let m = &a.mental;

        format!(
            "ST|{}|{}|{}|C:{},{},{},{},{},{},{}|K:{},{},{}|M:{},{},{},{},{}",
            self.id, self.full_name, position,
            c.attacking, c.defending, c.fitness, c.mental, c.tactical, c.technical, c.working_with_youngsters,
            k.judging_player_ability, k.judging_player_potential, k.tactical_knowledge,
            m.adaptability, m.determination, m.discipline, m.man_management, m.motivating,
        )
    }

    /// Legend explaining the compact `as_llm()` format for staff.
    pub fn as_llm_legend() -> &'static str {
        "Staff format: ST|id|name|position|C:att,def,fit,men,tac,tec,youth|K:jpa,jpp,tk|M:ada,det,dis,mm,mot\nAll skills 1-20."
    }
}
