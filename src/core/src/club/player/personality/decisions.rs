use crate::utils::FormattingUtils;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct PlayerDecisionHistory {
    pub items: Vec<PlayerDecision>,
}

#[derive(Debug, Clone)]
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

    /// Record a roster or market move as a `From → To` row, appending the
    /// fee (`From → To · $2.5M`) when one changed hands. The register's
    /// transfers, loans, buyouts and returns all share this shape, so the
    /// label is composed here rather than re-spelled at each call site.
    pub fn add_move(&mut self, date: NaiveDate, from: &str, to: &str, fee: f64, decision: &str) {
        let movement = if fee > 0.0 {
            format!("{} → {} · {}", from, to, FormattingUtils::format_money(fee))
        } else {
            format!("{} → {}", from, to)
        };
        self.add(date, movement, decision.to_string(), String::new());
    }
}
