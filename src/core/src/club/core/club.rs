use crate::club::academy::ClubAcademy;
use crate::club::board::ClubBoard;
use crate::club::facilities::ClubFacilities;
use crate::club::news::{ClubAffair, ClubAffairLog};
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, Player};
use crate::shared::Location;
use crate::transfers::market::knowledge::ClubMarketLedger;
use crate::transfers::pipeline::ClubTransferPlan;
use crate::{ReputationLevel, TeamCollection};
use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub enum ClubPhilosophy {
    /// Develop youth and sell for profit (Ajax, Benfica, Dortmund)
    DevelopAndSell,
    /// Sign established players, compete now (PSG, Chelsea, Man City)
    SignToCompete,
    /// Loan-heavy strategy, minimal spending (smaller clubs)
    LoanFocused,
    /// Balanced approach (most clubs)
    Balanced,
}

#[derive(Debug, Clone)]
pub struct ClubColors {
    pub background: String,
    pub foreground: String,
}

impl Default for ClubColors {
    fn default() -> Self {
        ClubColors {
            background: "#1e272d".to_string(),
            foreground: "#ffffff".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Club {
    pub id: u32,
    pub name: String,

    pub location: Location,

    pub board: ClubBoard,

    pub finance: ClubFinances,

    pub status: ClubStatus,

    pub academy: ClubAcademy,

    pub colors: ClubColors,

    pub teams: TeamCollection,

    pub transfer_plan: ClubTransferPlan,

    pub philosophy: ClubPhilosophy,

    pub facilities: ClubFacilities,

    pub rivals: Vec<u32>,

    /// Which foreign markets this club has actually done business in, and
    /// when. Half of what the club KNOWS of a market (the other half being
    /// its scouts) and the half that survives a scout leaving — see
    /// [`ClubMarketKnowledge`]. Bootstrapped at world load from the squad's
    /// own foreign nationalities, so the shipped world is its own evidence.
    pub market_ledger: ClubMarketLedger,

    /// The club's own diary: dated boardroom and dugout happenings the
    /// press cannot recompute from state. Written where each thing
    /// actually occurs — see [`ClubAffairLog`].
    pub affairs: ClubAffairLog,
}

impl Club {
    pub fn new(
        id: u32,
        name: String,
        location: Location,
        finance: ClubFinances,
        academy: ClubAcademy,
        status: ClubStatus,
        colors: ClubColors,
        teams: TeamCollection,
        facilities: ClubFacilities,
    ) -> Self {
        let philosophy = Self::determine_philosophy(&teams);

        Club {
            id,
            name,
            location,
            finance,
            status,
            academy,
            colors,
            board: ClubBoard::new(),
            teams,
            transfer_plan: ClubTransferPlan::new(),
            philosophy,
            facilities,
            rivals: Vec::new(),
            market_ledger: ClubMarketLedger::default(),
            affairs: ClubAffairLog::new(),
        }
    }

    fn determine_philosophy(teams: &TeamCollection) -> ClubPhilosophy {
        let rep_level = teams
            .main()
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);

        match rep_level {
            ReputationLevel::Elite => ClubPhilosophy::SignToCompete,
            ReputationLevel::Continental => ClubPhilosophy::Balanced,
            ReputationLevel::National => ClubPhilosophy::Balanced,
            _ => ClubPhilosophy::LoanFocused,
        }
    }

    pub fn is_rival(&self, other_club_id: u32) -> bool {
        self.rivals.contains(&other_club_id)
    }

    /// File a dated happening in the club's diary. The single entry
    /// point, so every writer records the date the same way and the
    /// press never has to guess when something occurred.
    pub fn record_affair(&mut self, affair: ClubAffair, date: NaiveDate) {
        self.affairs.record(affair, date);
    }

    /// Every force-selected player across the club, regardless of the
    /// team they're rostered on. Callers pass these straight to the
    /// squad selector as the first reserves so the +1000 selection
    /// bonus pins them into the match-day XI before the usual scoring
    /// logic decides anything else.
    pub fn get_force_selected_players(&self) -> Vec<&Player> {
        self.teams
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .filter(|p| p.is_force_match_selection)
            .collect()
    }
}
