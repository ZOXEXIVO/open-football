use crate::Club;
use crate::league::LeagueTable;
use crate::r#match::MatchResult;
use chrono::NaiveDate;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LeagueRegulations {
    pub suspended_players: HashMap<u32, u8>,
    pub yellow_card_accumulation: HashMap<u32, u8>,
    pub ffp_violations: Vec<FFPViolation>,
    pub pending_cases: Vec<DisciplinaryCase>,
}

impl LeagueRegulations {
    pub fn new() -> Self {
        LeagueRegulations {
            suspended_players: HashMap::new(),
            yellow_card_accumulation: HashMap::new(),
            ffp_violations: Vec::new(),
            pending_cases: Vec::new(),
        }
    }

    pub fn process_disciplinary_actions(&mut self, _result: &MatchResult) {}

    pub fn check_ffp_violation(&self, club: &Club) -> bool {
        let deficit = club.finance.balance.outcome - club.finance.balance.income;
        deficit > 30_000_000
    }

    pub fn apply_ffp_sanctions(&mut self, club_id: u32, table: &mut LeagueTable) {
        self.ffp_violations.push(FFPViolation {
            club_id,
            violation_type: FFPViolationType::ExcessiveDeficit,
            sanction: FFPSanction::PointDeduction(6),
        });

        if let Some(row) = table.rows.iter_mut().find(|r| r.team_id == club_id) {
            row.points = row.points.saturating_sub(6);
        }
    }

    pub fn process_pending_cases(&mut self, current_date: NaiveDate) {
        self.pending_cases
            .retain(|case| case.hearing_date > current_date);
    }
}

#[derive(Debug, Clone)]
pub struct FFPViolation {
    pub club_id: u32,
    pub violation_type: FFPViolationType,
    pub sanction: FFPSanction,
}

#[derive(Debug, Clone)]
pub enum FFPViolationType {
    ExcessiveDeficit,
    UnpaidDebts,
    FalseAccounting,
}

#[derive(Debug, Clone)]
pub enum FFPSanction {
    Warning,
    Fine(u32),
    PointDeduction(u8),
    TransferBan,
}

#[derive(Debug, Clone)]
pub struct DisciplinaryCase {
    pub player_id: u32,
    pub incident_type: String,
    pub hearing_date: NaiveDate,
}
