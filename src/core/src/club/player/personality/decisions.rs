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

impl Default for PlayerDecisionHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerDecisionHistory {
    /// The movements a decision that put him on the market is filed
    /// under — each carries its reason in `decision`.
    const LISTING_MOVEMENTS: [&'static str; 5] = [
        "dec_transfer_listed",
        "dec_loan_listed",
        "dec_free_transfer_listed",
        "dec_board_transfer_listed",
        "dec_board_loan_listed",
    ];

    pub fn new() -> Self {
        PlayerDecisionHistory { items: Vec::new() }
    }

    /// The most recent decision that listed him, whatever the register
    /// recorded after it.
    pub fn latest_listing(&self) -> Option<&PlayerDecision> {
        self.items
            .iter()
            .rev()
            .find(|d| Self::LISTING_MOVEMENTS.contains(&d.movement.as_str()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2031, 11, d).unwrap()
    }

    #[test]
    fn a_later_pathway_row_does_not_hide_the_listing() {
        let mut history = PlayerDecisionHistory::new();
        history.add(
            day(1),
            "dec_board_loan_listed".to_string(),
            "dec_reason_development_pathway".to_string(),
            "dec_decided_board".to_string(),
        );
        history.add(
            day(1),
            "dec_pathway_stage_changed".to_string(),
            "pathway_stage_loan_out".to_string(),
            "dec_decided_board".to_string(),
        );
        assert_eq!(
            history.latest_listing().map(|d| d.decision.as_str()),
            Some("dec_reason_development_pathway")
        );
    }

    #[test]
    fn no_listing_row_means_no_listing() {
        let mut history = PlayerDecisionHistory::new();
        history.add(
            day(1),
            "dec_pathway_stage_changed".to_string(),
            "pathway_stage_starter".to_string(),
            "dec_decided_board".to_string(),
        );
        assert!(history.latest_listing().is_none());
    }
}
