use crate::club::staff::staff::Staff;
use serde::Serialize;

#[derive(Serialize)]
struct StaffLlm {
    id: u32,
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "pos")]
    position: String,
    #[serde(rename = "c")]
    coaching: StaffCoachingLlm,
    #[serde(rename = "k")]
    knowledge: StaffKnowledgeLlm,
    #[serde(rename = "m")]
    mental: StaffMentalLlm,
}

#[derive(Serialize)]
struct StaffCoachingLlm {
    #[serde(rename = "att")]
    attacking: u8,
    #[serde(rename = "def")]
    defending: u8,
    #[serde(rename = "fit")]
    fitness: u8,
    #[serde(rename = "men")]
    mental: u8,
    #[serde(rename = "tac")]
    tactical: u8,
    #[serde(rename = "tec")]
    technical: u8,
    #[serde(rename = "yth")]
    working_with_youngsters: u8,
}

#[derive(Serialize)]
struct StaffKnowledgeLlm {
    #[serde(rename = "jpa")]
    judging_player_ability: u8,
    #[serde(rename = "jpp")]
    judging_player_potential: u8,
    #[serde(rename = "tk")]
    tactical_knowledge: u8,
}

#[derive(Serialize)]
struct StaffMentalLlm {
    #[serde(rename = "ada")]
    adaptability: u8,
    #[serde(rename = "det")]
    determination: u8,
    #[serde(rename = "dis")]
    discipline: u8,
    #[serde(rename = "mm")]
    man_management: u8,
    #[serde(rename = "mot")]
    motivating: u8,
}

const STAFF_LEGEND: &str = r#"{"id":"Unique staff ID","pos":"Role","c":"Coaching(1-20):att=attacking,def=defending,fit=fitness,men=mental,tac=tactical,tec=technical,yth=working_with_youngsters","k":"Knowledge(1-20):jpa=judging_ability,jpp=judging_potential,tk=tactical_knowledge","m":"Mental(1-20):ada=adaptability,det=determination,dis=discipline,mm=man_management,mot=motivating"}"#;

impl Staff {
    pub fn llm_legend() -> &'static str {
        STAFF_LEGEND
    }

    pub fn as_llm(&self) -> String {
        let position = self.contract.as_ref()
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
                attacking: c.attacking,
                defending: c.defending,
                fitness: c.fitness,
                mental: c.mental,
                tactical: c.tactical,
                technical: c.technical,
                working_with_youngsters: c.working_with_youngsters,
            },
            knowledge: StaffKnowledgeLlm {
                judging_player_ability: k.judging_player_ability,
                judging_player_potential: k.judging_player_potential,
                tactical_knowledge: k.tactical_knowledge,
            },
            mental: StaffMentalLlm {
                adaptability: m.adaptability,
                determination: m.determination,
                discipline: m.discipline,
                man_management: m.man_management,
                motivating: m.motivating,
            },
        };

        serde_json::to_string(&staff).unwrap()
    }
}
