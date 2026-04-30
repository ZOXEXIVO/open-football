use crate::club::staff::staff::Staff;
use serde::Serialize;

#[derive(Serialize)]
struct StaffLlm {
    id: u32,
    name: String,
    position: String,
    coaching: StaffCoachingLlm,
    knowledge: StaffKnowledgeLlm,
    mental: StaffMentalLlm,
}

#[derive(Serialize)]
struct StaffCoachingLlm {
    attacking: String,
    defending: String,
    fitness: String,
    mental: String,
    tactical: String,
    technical: String,
    working_with_youngsters: String,
}

#[derive(Serialize)]
struct StaffKnowledgeLlm {
    judging_player_ability: String,
    judging_player_potential: String,
    tactical_knowledge: String,
}

#[derive(Serialize)]
struct StaffMentalLlm {
    adaptability: String,
    determination: String,
    discipline: String,
    man_management: String,
    motivating: String,
}

fn pct_u8(val: u8) -> String {
    format!("{}%", val as u32 * 5)
}

impl Staff {
    pub fn as_llm(&self) -> String {
        let position = self
            .contract
            .as_ref()
            .map(|c| format!("{:?}", c.position))
            .unwrap_or_else(|| "Free".to_string());

        let a = &self.staff_attributes;
        let c = &a.coaching;
        let k = &a.knowledge;
        let m = &a.mental;

        let staff = StaffLlm {
            id: self.id,
            name: self.full_name.to_string(),
            position,
            coaching: StaffCoachingLlm {
                attacking: pct_u8(c.attacking),
                defending: pct_u8(c.defending),
                fitness: pct_u8(c.fitness),
                mental: pct_u8(c.mental),
                tactical: pct_u8(c.tactical),
                technical: pct_u8(c.technical),
                working_with_youngsters: pct_u8(c.working_with_youngsters),
            },
            knowledge: StaffKnowledgeLlm {
                judging_player_ability: pct_u8(k.judging_player_ability),
                judging_player_potential: pct_u8(k.judging_player_potential),
                tactical_knowledge: pct_u8(k.tactical_knowledge),
            },
            mental: StaffMentalLlm {
                adaptability: pct_u8(m.adaptability),
                determination: pct_u8(m.determination),
                discipline: pct_u8(m.discipline),
                man_management: pct_u8(m.man_management),
                motivating: pct_u8(m.motivating),
            },
        };

        serde_json::to_string(&staff).unwrap()
    }
}
