use chrono::NaiveDate;

#[derive(Debug)]
pub struct PlayerDecisionHistory {
    pub items: Vec<PlayerDecision>,
}

#[derive(Debug)]
pub struct PlayerDecision {
    pub date: NaiveDate,
    pub movement: String,
    pub decision: String,
    pub decided_by: String,
}

impl PlayerDecisionHistory {
    pub fn new() -> Self {
        PlayerDecisionHistory { items: Vec::new() }
    }

    pub fn add(&mut self, date: NaiveDate, movement: String, decision: String, decided_by: String) {
        self.items.push(PlayerDecision {
            date,
            movement,
            decision,
            decided_by,
        });
    }
}
